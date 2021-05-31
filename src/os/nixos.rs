use kv_log_macro as log;

use core::fmt;
use serde::Deserialize;
use std::{
    borrow::Cow,
    path::PathBuf,
    process::{Output, Stdio},
};

use ::log::kv::{self, ToValue};

use crate::NixOperatingSystem;

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

    fn command_line<'a>(&'a self, verb: super::Verb, flake: &'a crate::Flake) -> Vec<Cow<'a, str>> {
        vec![
            self.base_command(),
            Cow::from(Self::verb_command(verb)),
            Cow::from("--show-trace"),
            Cow::from("--flake"),
            Cow::from(flake.resolved_path()),
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
}

#[async_trait::async_trait]
impl NixOperatingSystem for Nixos {
    fn base_command(&'_ self) -> std::borrow::Cow<'_, str> {
        Cow::from("nixos-rebuild")
    }

    async fn run_command(
        &self,
        verb: super::Verb,
        flake: &crate::Flake,
    ) -> Result<(), anyhow::Error> {
        let mut cmd = self.session.command("sudo");
        let flake_base_name = flake
            .resolved_path
            .file_name()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Resolved path has a weird format: {:?}",
                    flake.resolved_path
                )
            })?
            .to_str()
            .expect("Nix path must be utf-8 clean");
        let unit_name = format!("{}--{}", Self::verb_command(verb), flake_base_name);

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
        cmd.args(self.command_line(verb, flake));
        log::debug!("Running nixos-rebuild", {
            verb: verb,
            unit_name: unit_name.as_str(),
        });
        let status = cmd.status().await?;
        if !status.success() {
            return Err(anyhow::anyhow!("Invoking {:?} failed: {:?}", verb, status));
        }
        Ok(())
    }

    async fn preflight_check(&self) -> Result<(), anyhow::Error> {
        let mut cmd = self.session.command("sudo");
        cmd.stdout(Stdio::piped());
        cmd.args(&["systemctl", "is-system-running", "--wait"]);
        let health = cmd.output().await?;
        let health_data = String::from_utf8_lossy(&health.stdout);
        let status = health_data.strip_suffix("\n");
        if !health.status.success() {
            log::error!("System is not healthy. List of broken units follows:", {
                status: status
            });
            self.session
                .command("sudo")
                .args(&["systemctl", "list-units", "--failed"])
                .stdout(Stdio::inherit())
                .status()
                .await?;
            anyhow::bail!("Can not deploy to an unhealthy system");
        }
        log::info!("System is healthy", { status: status });
        Ok(())
    }

    async fn build_flake(&self, flake: &crate::Flake) -> Result<PathBuf, anyhow::Error> {
        let hostname = self.hostname().await?;

        let mut cmd = self.session.command("env");
        cmd.args(&["-C", "/tmp"])
            .args(&["nix", "build", "-L", "--no-link", "--json"])
            .arg(flake.nixos_system_config(&hostname));
        cmd.stderr(Stdio::inherit()).stdin(Stdio::inherit());
        let output = cmd.output().await?;
        if !output.status.success() {
            anyhow::bail!("Could not build the flake.");
        }
        let mut results: Vec<NixBuildResult> = serde_json::from_slice(&output.stdout)?;
        if results.len() == 1 {
            let result = results.pop().unwrap();
            Ok(result.outputs.out)
        } else {
            Err(anyhow::anyhow!(
                "Did not receive the required number of results: {:?}",
                results
            ))
        }
    }
}

impl fmt::Debug for Nixos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "nixos://{}", self.host)
    }
}

impl ToValue for Nixos {
    fn to_value(&self) -> kv::Value<'_> {
        kv::Value::capture_debug(self)
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
