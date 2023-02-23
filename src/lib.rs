use log::Instrument;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tracing::instrument;
mod nix;
mod os;
use tracing as log;

pub use os::{NixOperatingSystem, Verb};

use anyhow::{anyhow, bail, Context};
use os::Nixos;
use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
use tokio::process::Command;

/// All the important bits about a nix flake reference.
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Flake {
    /// The path that the flake source code lives in.
    dir: PathBuf,

    /// The path that the flake derivation lives in, via `nix info`
    resolved_path: PathBuf,
}

/// Read from an AsyncRead stream and log each line as INFO-level messages.
pub(crate) async fn read_and_log_messages(
    stream: &str,
    r: impl AsyncRead + Unpin,
) -> Result<(), anyhow::Error> {
    let br = BufReader::new(r);
    let mut lines = br.lines();
    while let Some(line) = lines
        .next_line()
        .await
        .context("Unable to read next line")?
    {
        log::event!(log::Level::INFO, "{stream} {line}");
    }
    Ok(())
}

impl Flake {
    /// Construct a new flake reference from a source path.
    #[instrument(level = "DEBUG", err)]
    pub fn from_path<P: fmt::Debug + AsRef<Path>>(dir: P) -> Result<Self, anyhow::Error> {
        let dir = dir.as_ref().to_owned();
        let info = nix::FlakeInfo::from_path(&dir).with_context(|| format!("Flake {:?}", &dir))?;
        Ok(Flake {
            dir,
            resolved_path: info.path,
        })
    }

    /// Returns the store path of the flake as a utf-8 string.
    pub fn resolved_path(&self) -> &str {
        self.resolved_path
            .to_str()
            .expect("Resolved flake path must be utf-8 clean")
    }

    /// Returns a flake fragment to a NixOS system configuration for the given hostname.
    pub fn nixos_system_config(&self, hostname: &str) -> String {
        format!(
            "{}#nixosConfigurations.{}.config.system.build.toplevel",
            self.resolved_path(),
            hostname
        )
    }

    /// Copies the store path closure to the destination host.
    #[instrument(skip(self), fields(to), err)]
    pub async fn copy_closure(&self, to: &str) -> Result<(), anyhow::Error> {
        let mut cmd = Command::new("nix-copy-closure");
        cmd.args([to, self.resolved_path()]);
        cmd.stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;
        let stdout_read = tokio::task::spawn(
            read_and_log_messages("O", child.stdout.take().unwrap())
                .instrument(log::Span::current()),
        );

        let stderr_read = tokio::task::spawn(
            read_and_log_messages("E", child.stderr.take().unwrap())
                .instrument(log::Span::current()),
        );

        let outcomes = futures::join!(cmd.status(), stdout_read, stderr_read);
        let result = outcomes.0?;
        if !result.success() {
            bail!("nix-copy-closure failed");
        }
        Ok(())
    }

    #[instrument(err)]
    pub async fn build(
        &self,
        on: Arc<dyn NixOperatingSystem + Send + Sync>,
        config_name: Option<&str>,
    ) -> Result<SystemConfiguration, anyhow::Error> {
        let (path, system_name) = on.build_flake(self, config_name).await?;
        Ok(SystemConfiguration {
            path,
            system: on,
            system_name,
        })
    }
}

/// Represents a "built" system configuration on a system that is ready to be activated.
pub struct SystemConfiguration {
    path: PathBuf,
    system: Arc<dyn NixOperatingSystem + Send + Sync>,
    system_name: String,
}

impl SystemConfiguration {
    #[instrument(skip(self) err)]
    pub async fn test_config(&self) -> Result<(), anyhow::Error> {
        self.system.test_config(&self.path).await
    }

    #[instrument(skip(self) err)]
    pub async fn boot_config(&self) -> Result<(), anyhow::Error> {
        log::event!(
            log::Level::DEBUG,
            "Attempting to activate boot configuration (dry-run)"
        );
        self.system
            .update_boot_for_config(&self.path)
            .await
            .context("Trial run of boot activation failed. No cleanup necessary.")?;

        log::event!(log::Level::DEBUG, "Setting system profile");
        self.system
            .set_as_current_generation(&self.path)
            .await
            .context("You may have to check the system profile generation to clean up.")?;

        self.system.update_boot_for_config(&self.path).await
            .context("Actually setting the boot configuration failed. To clean up, you'll have to reset the system profile.")
    }

    #[instrument(level="DEBUG", skip(self) err)]
    pub async fn preflight_check(&self) -> Result<(), anyhow::Error> {
        self.system.preflight_check().await
    }

    /// Returns the system that the configuration resides on.
    pub fn on(&self) -> &Arc<dyn NixOperatingSystem + Send + Sync> {
        &self.system
    }

    /// Returns the nix store path of the system that will be activated.
    pub fn configuration(&self) -> &Path {
        self.path.as_ref()
    }

    /// Returns the name of the system configuration.
    pub fn for_system(&self) -> &str {
        &self.system_name
    }
}

/// The kind of operating system we deploy to
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Flavor {
    /// NixOS, the default.
    Nixos,
}

impl Default for Flavor {
    fn default() -> Self {
        Flavor::Nixos
    }
}

impl FromStr for Flavor {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "nixos" => Ok(Flavor::Nixos),
            s => Err(anyhow!(
                "Can not parse {:?} - only \"nixos\" is a valid flavor",
                s
            )),
        }
    }
}

impl ToString for Flavor {
    fn to_string(&self) -> String {
        match self {
            Flavor::Nixos => "nixos".to_string(),
        }
    }
}

impl Flavor {
    pub fn on_connection(
        &self,
        host: &str,
        connection: openssh::Session,
    ) -> Arc<dyn NixOperatingSystem + Send + Sync> {
        match self {
            Flavor::Nixos => Arc::new(Nixos::new(host.to_owned(), connection)),
        }
    }
}
