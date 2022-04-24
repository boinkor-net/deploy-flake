use tokio::task;
use tracing as log;
use tracing::instrument;

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
                        .filter(|path| !path.is_empty())
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

    /// The destinations that will be deployed to.
    ///
    /// Each destination is either just a hostname, or a URL of the
    /// form FLAVOR://HOSTNAME/[CONFIGURATION] where FLAVOR is
    /// "nixos", and the optional CONFIGURATION specifies what
    /// nixosConfiguration to build and deploy on the destination
    /// (defaults to the hostname that the remote host reports).
    #[clap(parse(try_from_str))]
    to: Vec<Destination>,
}

#[instrument(err)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt::init();

    let opts: Opts = Opts::parse();
    log::trace!(cmdline = ?opts);

    let flake = Flake::from_path(&opts.flake)?;
    log::debug!(?flake, "Flake metadata");

    futures::future::try_join_all(opts.to.into_iter().map(|destination| {
        let flake = flake.clone();
        task::spawn(async move { deploy(flake, destination).await })
    }))
    .await?;

    Ok(())
}

#[instrument(skip(flake, destination), fields(flake=?flake.resolved_path(), dest=?destination.hostname) err)]
async fn deploy(flake: Flake, destination: Destination) -> Result<(), anyhow::Error> {
    log::info!(flake=?flake.resolved_path(), host=?destination.hostname, "Copying");
    flake.copy_closure(&destination.hostname)?;

    log::debug!("Connecting");
    let flavor = destination.os_flavor.on_connection(
        &destination.hostname,
        Session::connect(&destination.hostname, KnownHosts::Strict)
            .await
            .with_context(|| format!("Connecting to {:?}", &destination.hostname))?,
    );
    log::info!(config=?destination.config_name, "Building");
    let built = flake
        .build(flavor, destination.config_name.as_deref())
        .await?;

    log::info!("Checking system health");
    built.preflight_check().await?;

    log::info!(configuration=?built.configuration(), system_name=?built.for_system(), "Testing");
    built.test_config().await?;
    // TODO: rollbacks, maybe?
    log::info!(configuration=?built.configuration(), system_name=?built.for_system(), "Activating");
    built.boot_config().await?;
    Ok(())
}
