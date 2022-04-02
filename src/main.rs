use tracing as log;

use anyhow::Context;
use clap::Parser;
use deploy_flake::{Flake, Flavor};
use openssh::{KnownHosts, Session};
use std::{path::PathBuf, str::FromStr};
use url::Url;

#[derive(Debug)]
struct Destination {
    os_flavor: Flavor,
    hostname: String,
    config_name: Option<String>,
}

impl FromStr for Destination {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(url) = Url::parse(s) {
            // we have a URL, let's see if it matches something we can deal with:
            match (url.scheme(), url.host_str(), url.path()) {
                (scheme, Some(hostname), path) if scheme == "nixos" => Ok(Destination {
                    os_flavor: Flavor::Nixos,
                    hostname: hostname.to_string(),
                    config_name: path
                        .strip_prefix('/')
                        .filter(|path| path.len() != 0)
                        .map(String::from),
                }),
                _ => anyhow::bail!("Unable to parse {s}"),
            }
        } else {
            Ok(Destination {
                os_flavor: Flavor::Nixos,
                hostname: s.to_string(),
                config_name: None,
            })
        }
    }
}

#[derive(Parser, Debug)]
#[clap(author = "Andreas Fuchs <asf@boinkor.net>")]
struct Opts {
    /// The flake source code directory to deploy.
    #[clap(long, default_value = ".")]
    flake: PathBuf,

    /// The destinations that we deploy to
    #[clap(parse(try_from_str))]
    to: Vec<Destination>,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt::init();

    let opts: Opts = Opts::parse();
    log::trace!(cmdline = ?opts);

    let flake = Flake::from_path(&opts.flake)?;
    log::debug!(?flake, "Flake metadata");

    for destination in opts.to {
        log::info!(flake=?flake.resolved_path(), host=?destination.hostname, "Copying");
        flake.copy_closure(&destination.hostname)?;

        log::debug!(to=?destination.hostname, "Connecting");
        let flavor = destination.os_flavor.on_connection(
            &destination.hostname,
            Session::connect(&destination.hostname, KnownHosts::Strict)
                .await
                .with_context(|| format!("Connecting to {:?}", &destination.hostname))?,
        );
        log::info!(flake=?flake.resolved_path(), host=?destination.hostname, config=?destination.config_name, "Building");
        let built = flake
            .build(flavor, destination.config_name.as_deref())
            .await?;

        log::info!("Checking system health");
        built.preflight_check().await?;

        log::info!(configuration=?built.configuration(), host=?built.on(), system_name=?built.for_system(), "Testing");
        built.test_config().await?;
        // TODO: rollbacks, maybe?
        log::info!(configuration=?built.configuration(), host=?built.on(), system_name=?built.for_system(), "Activating");
        built.boot_config().await?;
    }

    Ok(())
}
