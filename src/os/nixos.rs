use std::borrow::Cow;

use crate::NixOperatingSystem;

/// A nixos operating system instance.
pub struct Nixos {
    session: openssh::Session,
}

impl Nixos {
    /// Setup a new Nixos connection
    pub(crate) fn new(session: openssh::Session) -> Self {
        Self { session }
    }
}

impl NixOperatingSystem for Nixos {
    fn base_command<'a>(&'a self) -> std::borrow::Cow<'a, str> {
        Cow::from("nixos-rebuild")
    }
}