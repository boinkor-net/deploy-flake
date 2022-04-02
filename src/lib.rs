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
    process::Command,
    str::FromStr,
};

/// All the important bits about a nix flake reference.
#[derive(PartialEq, Clone, Debug)]
pub struct Flake {
    /// The path that the flake source code lives in.
    dir: PathBuf,

    /// The path that the flake derivation lives in, via `nix info`
    resolved_path: PathBuf,
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

    /// Synchronously copies the store path closure to the destination host.
    #[instrument(err)]
    pub fn copy_closure(&self, to: &str) -> Result<(), anyhow::Error> {
        let result = Command::new("nix-copy-closure")
            .args(&[to, self.resolved_path()])
            .status()?;
        if !result.success() {
            bail!("nix-copy-closure failed");
        }
        Ok(())
    }

    #[instrument(level = "INFO", "Building flake", err)]
    pub async fn build(
        &self,
        on: Box<dyn NixOperatingSystem>,
        config_name: Option<&str>,
    ) -> Result<SystemConfiguration, anyhow::Error> {
        let (path, system_name) = on.build_flake(&self, config_name).await?;
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
    system: Box<dyn NixOperatingSystem>,
    system_name: String,
}

impl SystemConfiguration {
    #[instrument(skip(self), fields(on=?self.on(), configuration=?self.configuration()), err)]
    pub async fn test_config(&self) -> Result<(), anyhow::Error> {
        self.system.test_config(&self.path).await
    }

    #[instrument(skip(self), fields(on=?self.on(), configuration=?self.configuration()), err)]
    pub async fn boot_config(&self) -> Result<(), anyhow::Error> {
        log::debug!("Attempting to activate boot configuration (dry-run)");
        self.system
            .update_boot_for_config(&self.path)
            .await
            .context("Trial run of boot activation failed. No cleanup necessary.")?;

        log::debug!("Setting system profile");
        self.system
            .set_as_current_generation(&self.path)
            .await
            .context("You may have to check the system profile generation to clean up.")?;

        self.system.update_boot_for_config(&self.path).await
            .context("Actually setting the boot configuration failed. To clean up, you'll have to reset the system profile.")
    }

    #[instrument(skip(self), fields(on=?self.on(), configuration=?self.configuration()), err)]
    pub async fn preflight_check(&self) -> Result<(), anyhow::Error> {
        self.system.preflight_check().await
    }

    /// Returns the system that the configuration resides on.
    pub fn on(&self) -> &Box<dyn NixOperatingSystem> {
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
#[derive(Debug, PartialEq, Clone, Copy)]
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
    ) -> Box<dyn NixOperatingSystem> {
        match self {
            Flavor::Nixos => Box::new(Nixos::new(host.to_owned(), connection)),
        }
    }
}
