mod nixos;

use std::{
    fmt,
    path::{Path, PathBuf},
};

pub use nixos::Nixos;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Verb {
    Test,
    Build,
    Boot,
}

pub(crate) trait NixOperatingSystem: fmt::Debug {
    /// Checks if the target system is able to be deployed to.
    async fn preflight_check_system(&self) -> Result<(), anyhow::Error>;

    /// Checks if the built closure can be deployed to the system.
    async fn preflight_check_closure(
        &self,
        derivation: &Path,
        script: &Path,
    ) -> Result<(), anyhow::Error>;

    /// Builds a system configuration closure from the flake and
    /// returns the path to the built closure and the name of the
    /// system that it was built for.
    async fn build_flake(
        &self,
        flake: &crate::Flake,
        config_name: Option<&str>,
        build_cmdline: Vec<String>,
    ) -> Result<(PathBuf, String), anyhow::Error>;

    /// Sets the built system as the current "system" profile
    /// generation, without activation.
    async fn set_as_current_generation(&self, derivation: &Path) -> Result<(), anyhow::Error>;

    /// Test the flake's system configuration on the live system.
    async fn test_config(&self, derivation: &Path) -> Result<(), anyhow::Error>;

    /// Update the system's boot menu to include the configuration as the default boot entry.
    async fn update_boot_for_config(&self, derivation: &Path) -> Result<(), anyhow::Error>;
}
