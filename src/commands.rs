/// CLI commands and their implementation.
use std::{path::PathBuf, str::FromStr};

use clap::{Parser, Subcommand};

use crate::{Flake, Instrumentation};

pub mod deploy;
pub mod status;

/// Determines the behavior for certain aspects of the deploy process
#[derive(clap::ValueEnum, Clone, Copy, Debug, Eq, PartialEq)]
pub enum Behavior {
    /// Run the behavior
    Run,

    /// Skip the behavior.
    Skip,
}

impl FromStr for Behavior {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "skip" => Ok(Behavior::Skip),
            "run" => Ok(Behavior::Run),
            _ => anyhow::bail!("Unknown behavior {s:?}"),
        }
    }
}

#[derive(Parser, Clone, Debug, Eq, PartialEq)]
pub struct Opts {
    /// What kind of instrumentation to emit.
    /// Either "tui" or "json".
    #[clap(long, default_value = "tui")]
    pub instrumentation: Instrumentation,

    /// The flake source code directory to deploy.
    #[clap(long, default_value = ".")]
    pub flake: PathBuf,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Clone, Debug, Eq, PartialEq)]
pub enum Command {
    /// Run a full deploy lifecycle for the systems configured in the flake.
    #[command(arg_required_else_help = true)]
    Deploy(deploy::Opts),

    /// Print the status of the given hosts.
    #[command(arg_required_else_help = true)]
    Status(status::Opts),
}

impl Command {
    pub async fn run(self, flake: Flake) -> Result<(), anyhow::Error> {
        match self {
            Command::Deploy(opts) => deploy::run(flake, opts).await,
            Command::Status(opts) => status::run(opts).await,
        }
    }
}
