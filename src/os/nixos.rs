use anyhow::Context;
use openssh::Command;
use tracing as log;
use tracing::instrument;

use core::fmt;
use serde::Deserialize;
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    process::{Output, Stdio},
};

use crate::{NixOperatingSystem, Verb};

/// A nixos operating system instance.
pub struct Nixos {
    host: String,
    session: openssh::Session,
}

fn strip_shell_output(output: Output) -> String {
    let len = &output.stdout.len();
    let last_byte = output.stdout[len - 1];
    if last_byte == b'\n' {
        String::from_utf8_lossy(&output.stdout[..(len - 1)]).to_string()
    } else {
        String::from_utf8_lossy(&output.stdout).to_string()
    }
}

impl Nixos {
    /// Setup a new Nixos connection
    pub(crate) fn new(host: String, session: openssh::Session) -> Self {
        Self { host, session }
    }

    fn command_line<'a>(&'a self, verb: super::Verb, derivation: &'a Path) -> Vec<Cow<'a, str>> {
        let activate_script = derivation.join("bin/switch-to-configuration");
        vec![
            Cow::from(activate_script.to_string_lossy().to_string()),
            Cow::from(Self::verb_command(verb)),
        ]
    }

    fn verb_command(verb: super::Verb) -> &'static str {
        use super::Verb::*;
        match verb {
            Test => "test",
            Build => "build",
            Boot => "boot",
        }
    }

    async fn hostname(&self) -> Result<String, anyhow::Error> {
        let output = self
            .session
            .command("hostname")
            .stderr(Stdio::inherit())
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Could not query for hostname: {:?}",
                output.status
            ));
        }
        Ok(strip_shell_output(output))
    }

    #[instrument(level = "DEBUG", err)]
    async fn run_command<'s>(&self, mut cmd: Command<'s>) -> Result<(), anyhow::Error> {
        cmd.stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .stdin(Stdio::inherit());

        log::debug!(command=?cmd, "Running");
        let status = cmd.status().await?;
        log::debug!(command=?cmd, ?status, "Finished");
        if !status.success() {
            anyhow::bail!("Remote command {:?} failed with status {:?}", cmd, status);
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl NixOperatingSystem for Nixos {
    #[instrument(level = "DEBUG", err)]
    async fn preflight_check(&self) -> Result<(), anyhow::Error> {
        let mut cmd = self.session.command("sudo");
        cmd.stdout(Stdio::piped());
        cmd.args(&["systemctl", "is-system-running", "--wait"]);
        let health = cmd.output().await?;
        let health_data = String::from_utf8_lossy(&health.stdout);
        let status = health_data.strip_suffix("\n");
        if !health.status.success() {
            log::error!(
                ?status,
                "System is not healthy. List of broken units follows:"
            );
            self.session
                .command("sudo")
                .args(&["systemctl", "list-units", "--failed"])
                .stdout(Stdio::inherit())
                .status()
                .await?;
            anyhow::bail!("Can not deploy to an unhealthy system");
        }
        log::info!(?status, "System is healthy");
        Ok(())
    }

    #[instrument(level = "DEBUG", err)]
    async fn build_flake(
        &self,
        flake: &crate::Flake,
        config_name: Option<&str>,
    ) -> Result<(PathBuf, String), anyhow::Error> {
        let hostname = match config_name {
            None => self.hostname().await?,
            Some(name) => name.to_owned(),
        };

        // We run this twice: Once to get progress to the user & see
        // output; and the second time to get the actual derivation
        // path, which thankfully happens fast because the build
        // result will be cached already.
        let build_args = &["nix", "build", "-L", "--no-link"];
        let mut cmd = self.session.command("env");
        cmd.args(&["-C", "/tmp"])
            .args(build_args)
            .arg(flake.nixos_system_config(&hostname));
        self.run_command(cmd)
            .await
            .context("Could not build the flake")?;

        let mut cmd = self.session.command("env");
        cmd.stderr(Stdio::inherit()).stdin(Stdio::inherit());
        cmd.args(&["-C", "/tmp"])
            .args(build_args)
            .arg("--json")
            .arg(flake.nixos_system_config(&hostname));
        let output = cmd.output().await?;
        if !output.status.success() {
            anyhow::bail!("Could not build the flake.");
        }
        let mut results: Vec<NixBuildResult> = serde_json::from_slice(&output.stdout)?;
        if results.len() == 1 {
            let result = results.pop().unwrap();
            Ok((result.outputs.out, hostname))
        } else {
            Err(anyhow::anyhow!(
                "Did not receive the required number of results: {:?}",
                results
            ))
        }
    }

    #[instrument(level = "DEBUG", err)]
    async fn set_as_current_generation(&self, derivation: &Path) -> Result<(), anyhow::Error> {
        let mut cmd = self.session.command("sudo");
        cmd.stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .stdin(Stdio::inherit());
        cmd.args(&["nix-env", "-p", "/nix/var/nix/profiles/system", "--set"])
            .arg(derivation.to_string_lossy());
        self.run_command(cmd)
            .await
            .with_context(|| format!("Could not set {:?} as the current generation", derivation))?;
        Ok(())
    }

    #[instrument(level = "DEBUG", err)]
    async fn test_config(&self, derivation: &Path) -> Result<(), anyhow::Error> {
        let mut cmd = self.session.command("sudo");
        let flake_base_name = derivation
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Built path has a weird format: {:?}", derivation))?
            .to_str()
            .expect("Nix path must be utf-8 clean");
        let unit_name = format!("{}--{}", Self::verb_command(Verb::Test), flake_base_name);

        cmd.stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .stdin(Stdio::inherit());
        cmd.args(&[
            "systemd-run",
            "--working-directory=/tmp",
            "--service-type=oneshot",
            "--send-sighup",
            "--unit",
            &unit_name,
            "--wait",
            "--quiet",
            "--pipe",
            // Fix perl complaining about bad locale settings:
            "--setenv=LC_ALL=C",
        ]);
        cmd.args(self.command_line(Verb::Test, derivation));
        log::debug!(?unit_name, "Running nixos-rebuild test in background");
        self.run_command(cmd)
            .await
            .with_context(|| format!("testing the system closure {:?} failed", derivation))?;
        Ok(())
    }

    #[instrument(level = "DEBUG", err)]
    async fn update_boot_for_config(&self, derivation: &Path) -> Result<(), anyhow::Error> {
        let mut cmd = self.session.command("sudo");
        cmd.stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .stdin(Stdio::inherit());
        cmd.args(self.command_line(Verb::Boot, derivation))
            .arg(derivation.to_string_lossy());
        let status = cmd.status().await?;
        if !status.success() {
            return Err(anyhow::anyhow!(
                "Could not set {:?} up as the boot system",
                derivation
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for Nixos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.host)
    }
}

#[derive(PartialEq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NixBuildResult {
    drv_path: PathBuf,

    outputs: NixOutput,
}

#[derive(PartialEq, Debug, Deserialize)]
struct NixOutput {
    out: PathBuf,
}
