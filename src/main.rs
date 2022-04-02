use tracing as log;

use anyhow::Context;
use clap::Parser;
use deploy_flake::{Flake, Flavor};
use openssh::{KnownHosts, Session};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(author = "Andreas Fuchs <asf@boinkor.net>")]
struct Opts {
    /// The flake source code directory to deploy.
    #[clap(long, default_value = ".")]
    flake: PathBuf,

    /// The operating system flavor to deploy to.
    #[clap(long, default_value_t)]
    os_flavor: Flavor,

    /// The host that we deploy to
    to: String,

    /// Name of the configuration to deploy. Defaults to the remote hostname.
    #[clap(long)]
    config_name: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt::init();

    let opts: Opts = Opts::parse();
    log::trace!(cmdline = ?opts);

    let flake = Flake::from_path(&opts.flake)?;
    log::debug!(?flake, "Flake metadata");

    log::info!(flake=flake.resolved_path(), host=?opts.to, "Copying");
    flake.copy_closure(&opts.to)?;

    log::debug!(to=?opts.to, "Connecting");
    let flavor = opts.os_flavor.on_connection(
        &opts.to,
        Session::connect(&opts.to, KnownHosts::Strict)
            .await
            .with_context(|| format!("Connecting to {:?}", &opts.to))?,
    );
    log::info!(flake=?flake.resolved_path(), host=?opts.to, "Building");
    let built = flake.build(flavor, opts.config_name.as_deref()).await?;

    log::info!("Checking system health");
    built.preflight_check().await?;

    log::info!(configuration=?built.configuration(), host=?built.on(), system_name=?built.for_system(), "Testing");
    built.test_config().await?;
    // TODO: rollbacks, maybe?
    log::info!(configuration=?built.configuration(), host=?built.on(), system_name=?built.for_system(), "Activating");
    built.boot_config().await?;

    Ok(())
}
