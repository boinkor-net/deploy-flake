use crate::read_and_log_messages;
use anyhow::Context;
use openssh::{Command, Stdio};
use tokio::io::AsyncReadExt;
use tracing as log;
use tracing::instrument;
use tracing::Instrument;

use core::fmt;
use serde::Deserialize;
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    process::Output,
};

use crate::{NixOperatingSystem, Verb};

/// A nixos operating system instance.
pub struct Nixos {
    host: String,
    session: openssh::Session,
}

pub const DEFAULT_PREFLIGHT_SCRIPT_NAME: &str = "pre-activate-safety-checks";

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

    fn activation_command_line<'a>(
        &'a self,
        verb: super::Verb,
        derivation: &'a Path,
    ) -> Vec<Cow<'a, str>> {
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

    #[instrument(level = "DEBUG", fields(pathname), err)]
    async fn test_file_existence<'s>(&self, path: &Path) -> Result<bool, anyhow::Error> {
        let mut cmd = self.session.command("test");
        cmd.arg("-f").raw_arg(path);
        cmd.stdout(Stdio::null())
            .stderr(Stdio::piped())
            .stdin(Stdio::inherit());
        log::event!(log::Level::DEBUG, command=?cmd, "Running");
        let mut child = cmd.spawn().await?;
        let stderr_read = tokio::task::spawn(
            read_and_log_messages("E", child.stderr().take().unwrap())
                .instrument(log::Span::current()),
        );
        let status = futures::join!(child.wait(), stderr_read);
        let exit_status = status.0?;
        log::event!(log::Level::DEBUG, command=?cmd, ?exit_status, "Finished");
        Ok(exit_status.success())
    }

    #[instrument(level = "DEBUG", fields(cmd), err)]
    async fn run_command<'s>(&self, mut cmd: Command<'s>) -> Result<(), anyhow::Error> {
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::inherit());

        log::event!(log::Level::DEBUG, command=?cmd, "Running");
        let mut child = cmd.spawn().await?;
        // Read stdout/stderr line-by-line and emit them as log messages:
        let stdout_read = tokio::task::spawn(
            read_and_log_messages("O", child.stdout().take().unwrap())
                .instrument(log::Span::current()),
        );
        let stderr_read = tokio::task::spawn(
            read_and_log_messages("E", child.stderr().take().unwrap())
                .instrument(log::Span::current()),
        );
        // Now, wait for it all to finish:
        let status = futures::join!(child.wait(), stdout_read, stderr_read);
        let exit_status = status.0?;
        log::event!(log::Level::DEBUG, command=?cmd, ?exit_status, "Finished");
        if !exit_status.success() {
            anyhow::bail!(
                "Remote command {:?} failed with status {:?}",
                cmd,
                exit_status
            );
        }
        Ok(())
    }
}

impl NixOperatingSystem for Nixos {
    #[instrument(level = "INFO", err)]
    async fn preflight_check_system(&self) -> Result<(), anyhow::Error> {
        let mut cmd = self.session.command("sudo");
        cmd.stdout(Stdio::piped());
        cmd.args(["systemctl", "is-system-running", "--wait"]);
        let health = cmd.output().await?;
        let health_data = String::from_utf8_lossy(&health.stdout);
        let status = health_data.strip_suffix('\n').unwrap_or("");
        if !health.status.success() {
            log::error!(
                ?status,
                "System is not healthy. List of broken units follows:"
            );
            let output = self
                .session
                .command("sudo")
                .args(["systemctl", "list-units", "--failed"])
                .stdout(Stdio::piped())
                .output()
                .await?;
            log::event!(
                log::Level::WARN,
                "Failed units:\n{}",
                String::from_utf8_lossy(&output.stdout)
            );
            anyhow::bail!("Can not deploy to an unhealthy system");
        }
        log::event!(log::Level::DEBUG, ?status, "System is healthy");
        Ok(())
    }

    #[instrument(level = "INFO", err)]
    async fn preflight_check_closure(
        &self,
        derivation: &Path,
        script: Option<&Path>,
    ) -> Result<(), anyhow::Error> {
        let script_path = if script.is_none() {
            // Try to use the default pre-activation script name emitted by preflight-safety:
            let script_path = derivation.join(DEFAULT_PREFLIGHT_SCRIPT_NAME);
            log::event!(log::Level::DEBUG, dest=?self.host, script=?script_path.file_name(), "Checking for existence of inferred pre-activation script");
            if !self.test_file_existence(&script_path).await? {
                return Ok(());
            }
            script_path
        } else {
            derivation.join(script.unwrap())
        };
        log::event!(log::Level::INFO, dest=?self.host, script=?script_path.file_name(), "Running pre-activation script");
        let mut cmd = self.session.command("sudo");
        cmd.raw_arg(script_path);
        self.run_command(cmd)
            .await
            .context("System closure self-checks failed")?;
        Ok(())
    }

    #[instrument(level = "DEBUG", err, skip(build_cmdline))]
    async fn build_flake(
        &self,
        flake: &crate::Flake,
        config_name: Option<&str>,
        build_cmdline: Vec<String>,
    ) -> Result<(PathBuf, String), anyhow::Error> {
        let hostname = match config_name {
            None => self.hostname().await?,
            Some(name) => name.to_owned(),
        };

        // We run this twice: Once to get progress to the user & see
        // output; and the second time to get the actual derivation
        // path, which thankfully happens fast because the build
        // result will be cached already.
        let build_args = ["nix", Self::verb_command(Verb::Build), "-L", "--no-link"];
        let mut cmd = self.session.command("env");
        cmd.args(["-C", "/tmp"])
            .args(build_args)
            .args(&build_cmdline)
            .arg(flake.nixos_system_config(&hostname));
        self.run_command(cmd)
            .await
            .context("Could not build the flake")?;

        let mut cmd = self.session.command("env");
        cmd.stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .stdin(Stdio::inherit());
        cmd.args(["-C", "/tmp"])
            .args(build_args)
            .args(&build_cmdline)
            .arg("--json")
            .arg(flake.nixos_system_config(&hostname));
        let mut child = cmd.spawn().await?;
        let stderr_log = tokio::task::spawn(read_and_log_messages(
            "E",
            child.stderr().take().expect("should have stderr"),
        ));
        let mut child_stdout = child.stdout().take().expect("should have stdout");
        let mut stdout = vec![];
        let all = futures::join!(
            child.wait(),
            stderr_log,
            child_stdout.read_to_end(&mut stdout)
        );
        let status = all.0?;
        if !status.success() {
            anyhow::bail!("Could not build the flake.");
        }
        let mut results: Vec<NixBuildResult> = serde_json::from_slice(&stdout)?;
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
        cmd.args(["nix-env", "-p", "/nix/var/nix/profiles/system", "--set"])
            .arg(derivation.to_string_lossy());
        self.run_command(cmd)
            .await
            .with_context(|| format!("Could not set {derivation:?} as the current generation"))?;
        Ok(())
    }

    #[instrument(level = "DEBUG", skip(self), fields(host=self.host), err)]
    async fn test_config(&self, derivation: &Path) -> Result<(), anyhow::Error> {
        let mut cmd = self.session.command("sudo");
        let flake_base_name = derivation
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Built path has a weird format: {:?}", derivation))?
            .to_str()
            .expect("Nix path must be utf-8 clean");
        let unit_name = format!("{}--{}", Self::verb_command(Verb::Test), flake_base_name);

        cmd.args([
            "systemd-run",
            "--working-directory=/tmp",
            "--service-type=oneshot",
            "--send-sighup",
            "--unit",
            &unit_name,
            "--wait",
            "--quiet",
            "--collect",
            "--pipe",
            // Fix perl complaining about bad locale settings:
            "--setenv=LC_ALL=C",
        ]);
        cmd.args(self.activation_command_line(Verb::Test, derivation));
        log::event!(
            log::Level::DEBUG,
            ?unit_name,
            "Running nixos-rebuild test in background"
        );
        self.run_command(cmd)
            .await
            .with_context(|| format!("testing the system closure {derivation:?} failed"))?;
        Ok(())
    }

    #[instrument(level = "DEBUG", err)]
    async fn update_boot_for_config(&self, derivation: &Path) -> Result<(), anyhow::Error> {
        let mut cmd = self.session.command("sudo");
        cmd.args(self.activation_command_line(Verb::Boot, derivation))
            .arg(derivation.to_string_lossy());
        self.run_command(cmd)
            .await
            .with_context(|| format!("Could not set {:?} up as the boot system", derivation))?;
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
