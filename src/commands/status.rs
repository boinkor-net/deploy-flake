use anyhow::Context as _;
use clap::Parser;
use openssh::{KnownHosts, Session};
use tokio::task;
use tracing as log;

use crate::{Destination, NixOperatingSystem};

/// Command that gathers information about the currently-running system on a host.

#[derive(Parser, Debug, PartialEq, Eq, Clone)]
pub struct Opts {
    /// The destination hosts to check.
    ///
    /// Each destination is either just a hostname, or a URL of the
    /// form FLAVOR://HOSTNAME/[CONFIGURATION] where FLAVOR is
    /// "nixos", and the optional CONFIGURATION specifies what
    /// nixosConfiguration to build and deploy on the destination
    /// (defaults to the hostname that the remote host reports).
    ///
    /// For the status command, any given CONFIGURATION is ignored.
    #[clap(value_parser)]
    pub to: Vec<Destination>,
}

pub async fn run(opts: Opts) -> anyhow::Result<()> {
    futures::future::try_join_all(
        opts.to
            .into_iter()
            .map(|destination| task::spawn(async move { status(destination).await })),
    )
    .await?;
    Ok(())
}

pub async fn status(destination: Destination) -> anyhow::Result<()> {
    log::debug!("Connecting");
    let flavor = destination.os_flavor.on_connection(
        &destination.hostname,
        Session::connect(&destination.hostname, KnownHosts::Strict)
            .await
            .with_context(|| format!("Connecting to {:?}", &destination.hostname))?,
    );
    log::debug!("Retrieving status");
    let status = flavor
        .current_system_info()
        .await
        .with_context(|| format!("Status for {destination:?}"))?;

    let formatter = humansize::make_format(humansize::DECIMAL);
    log::info!(
        registration_time=?status.registration_time,
        closure_size_human = formatter(status.closure_size),
        closure_size=status.closure_size,
        path=?status.path,
        "Got status"
    );
    Ok(())
}
