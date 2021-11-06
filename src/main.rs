use kv_log_macro as log;

use ::log::kv;
use anyhow::Context;
use clap::Parser;
use deploy_flake::{Flake, Flavor};
use openssh::{KnownHosts, Session};
use std::path::PathBuf;

mod logging;

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
}

impl Opts {
    fn as_value(&self) -> kv::Value {
        kv::Value::capture_debug(self)
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    logging::init()?;

    let opts: Opts = Opts::parse();
    log::trace!("Parsed command line", { opts: opts.as_value() });

    log::trace!("Reading flake", {path: kv::Value::capture_debug(&opts.flake)});
    let flake = Flake::from_path(&opts.flake)?;
    log::debug!("Flake metadata", { flake: flake.as_value() });

    log::info!("Copying flake", {flake: flake.resolved_path(), to: kv::Value::capture_debug(&opts.to)});
    flake.copy_closure(&opts.to)?;

    log::debug!("Connecting", {to: kv::Value::capture_debug(&opts.to)});

    let flavor = opts.os_flavor.on_connection(
        &opts.to,
        Session::connect(&opts.to, KnownHosts::Strict)
            .await
            .with_context(|| format!("Connecting to {:?}", &opts.to))?,
    );
    log::info!("Building flake", {on: flavor.as_ref(), flake: flake.resolved_path()});
    let built = flake.build(flavor).await?;

    log::info!("Checking system health", { cfg: &built });
    built.preflight_check().await?;

    log::info!("Testing config", { cfg: &built });
    built.test_config().await?;
    // TODO: rollbacks.
    log::info!("Activating config", { cfg: &built });
    built.boot_config().await?;

    Ok(())
}
