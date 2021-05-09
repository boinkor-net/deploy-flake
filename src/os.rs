mod nixos;

use crate::Flake;
use std::borrow::Cow;

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
    fn base_command<'a>(&'a self) -> Cow<'a, str>;

    /// Checks if the system is able to be deployed to.
    async fn preflight_check(&self) -> Result<(), anyhow::Error>;

    /// Executes the given rebuild command (either "test" or "boot" at the moment.).
    async fn run_command(&self, verb: Verb, flake: &Flake) -> Result<(), anyhow::Error>;
}
