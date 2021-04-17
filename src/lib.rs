mod nix;
mod os;

use log::kv::Value;
pub use os::NixOperatingSystem;

use anyhow::{anyhow, bail, Context};
use os::Nixos;
use std::{
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
    pub fn from_path<P: AsRef<Path>>(dir: P) -> Result<Self, anyhow::Error> {
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

    /// Synchronously copies the store path closure to the destination host.
    pub fn copy_closure(&self, to: &str) -> Result<(), anyhow::Error> {
        let result = Command::new("nix-copy-closure")
            .args(&[to, self.resolved_path()])
            .status()?;
        if !result.success() {
            bail!("nix-copy-closure failed");
        }
        Ok(())
    }

    pub fn as_value(&self) -> Value {
        Value::capture_debug(self)
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
    pub fn on_connection(&self, connection: openssh::Session) -> Box<dyn NixOperatingSystem> {
        match self {
            Flavor::Nixos => Box::new(Nixos::new(connection)),
        }
    }
}
