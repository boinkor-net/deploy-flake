use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::Context;
use serde::Deserialize;

#[derive(Deserialize, Debug, PartialEq, Clone)]
pub(crate) struct FlakeInfo {
    pub(crate) path: PathBuf,
}

impl FlakeInfo {
    pub(crate) fn from_path<P: AsRef<Path>>(p: P) -> Result<Self, anyhow::Error> {
        let output = Command::new("nix")
            .args(&["flake", "info", "--json"])
            .current_dir(p)
            .output()
            .context("Could not execute nix flake info")?;
        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "nix flake info failed:\n{}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(serde_json::from_slice(&output.stdout)?)
    }
}
