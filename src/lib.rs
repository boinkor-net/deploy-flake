mod nix;

use anyhow::Context;
use std::path::{Path, PathBuf};

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
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
