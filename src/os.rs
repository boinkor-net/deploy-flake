mod nixos;

use crate::Flake;
use std::borrow::Cow;

pub use nixos::Nixos;

pub trait NixOperatingSystem {
    /// The base command that the operating system flavor uses.
    ///
    /// On NixOS, that is "nixos-rebuild".
    fn base_command<'a>(&'a self) -> Cow<'a, str>;

    fn command_line<'a>(&'a self, verb: &'a str, flake: &'a Flake) -> Vec<Cow<'a, str>> {
        vec![
            self.base_command(),
            Cow::from(verb),
            Cow::from("--flake"),
            Cow::from(flake.resolved_path()),
        ]
    }

    fn test_command<'s>(
        &self,
        session: &'s openssh::Session,
        flake: &'s Flake,
    ) -> openssh::Command<'s> {
        let mut cmd = session.command("/usr/bin/env");
        cmd.args(self.command_line("test", flake));
        cmd
    }
}
