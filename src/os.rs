mod nixos;

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use log::kv::{self, ToValue};
pub use nixos::Nixos;

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Verb {
    Test,
    Build,
    Boot,
}

impl ToValue for Verb {
    fn to_value(&self) -> kv::Value {
        kv::Value::capture_debug(self)
    }
}

#[async_trait::async_trait]
pub trait NixOperatingSystem: ToValue {
    /// The base command that the operating system flavor uses.
    ///
    /// On NixOS, that is "nixos-rebuild".
    fn base_command(&'_ self) -> Cow<'_, str>;

    /// Checks if the system is able to be deployed to.
    async fn preflight_check(&self) -> Result<(), anyhow::Error>;

    /// Builds a system configuration closure from the flake and
    /// returns the path to the built closure.
    async fn build_flake(&self, flake: &crate::Flake) -> Result<PathBuf, anyhow::Error>;

    /// Sets the built system as the current "system" profile
    /// generation, without activation.
    async fn set_as_current_generation(&self, derivation: &Path) -> Result<(), anyhow::Error>;

    /// Test the flake's system configuration on the live system.
    async fn test_config(&self, derivation: &Path) -> Result<(), anyhow::Error>;

    /// Update the system's boot menu to include the configuration as the default boot entry.
    async fn update_boot_for_config(&self, derivation: &Path) -> Result<(), anyhow::Error>;
}
