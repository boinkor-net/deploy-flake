use tokio::task;
use tracing as log;
use tracing::instrument;

use anyhow::Context;
use clap::Parser;
use deploy_flake::{Flake, Flavor};
use openssh::{KnownHosts, Session};
use std::{path::PathBuf, str::FromStr};
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;
use url::Url;

#[derive(Debug, Clone)]
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
            match (url.scheme(), url.host_str(), url.path(), url.username()) {
                (scheme, Some(host), path, username) if scheme == "nixos" => {
                    let hostname = if username.is_empty() {
                        host.to_string()
                    } else {
                        format!("{username}@{host}")
                    };
                    Ok(Destination {
                        os_flavor: Flavor::Nixos,
                        hostname,
                        config_name: path
                            .strip_prefix('/')
                            .filter(|path| !path.is_empty())
                            .map(String::from),
                    })
                }
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

#[derive(clap::ValueEnum, Clone, Copy, Debug, Eq, PartialEq)]
enum Behavior {
    Run,
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
    #[clap(value_parser)]
    to: Vec<Destination>,

    /// Whether to run the "preflight" check, where deploy-flake
    /// checks if the target system is healthy. Running it is usually
    /// a good idea to do, but when updating boot config on a broken
    /// system, it is necessary to skip.
    #[clap(long, require_equals=true, value_name = "BEHAVIOR", default_missing_value = "run", default_value_t = Behavior::Run, value_enum)]
    preflight_check: Behavior,

    /// Whether to run the "test" step, updating the system config
    /// in-place before installing a new boot config. The default runs
    /// the test step, use `--test=skip` to directly install the built
    /// boot configuration.
    #[clap(long, require_equals=true, value_name = "BEHAVIOR", default_missing_value = "run", default_value_t = Behavior::Run, value_enum)]
    test: Behavior,

    /// Extra commandline arguments passed to the "nix build"
    /// command. Defaults to the arguments needed to activate the
    /// "flake" and "nix-command" features.
    #[clap(
        long,
        value_delimiter = ' ',
        value_parser,
        default_value = "--extra-experimental-features nix-command --extra-experimental-features flakes"
    )]
    build_cmdline: Vec<String>,
}

#[instrument(err)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let indicatif_layer = tracing_indicatif::IndicatifLayer::new();
    let filter = EnvFilter::builder()
        .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
        .from_env_lossy();
    let writer = indicatif_layer.get_stderr_writer();
    let app_log_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .compact()
        .with_writer(writer.clone())
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.target() != deploy_flake::SUBPROCESS_LOG_TARGET
        }));
    let subprocess_log_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_level(false)
        .compact()
        .with_writer(writer.clone())
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.target() == deploy_flake::SUBPROCESS_LOG_TARGET
        }));
    tracing_subscriber::registry()
        .with(filter)
        .with(app_log_layer)
        .with(subprocess_log_layer)
        .with(indicatif_layer)
        .init();

    let opts: Opts = Opts::parse();
    log::trace!(cmdline = ?opts);

    let flake = Flake::from_path(&opts.flake)?;
    log::debug!(?flake, "Flake metadata");

    let do_preflight = opts.preflight_check;
    let do_test = opts.test;
    let build_cmdline = opts.build_cmdline.clone();

    futures::future::try_join_all(opts.to.into_iter().map(|destination| {
        let flake = flake.clone();
        let build_cmdline = build_cmdline.clone();
        task::spawn(async move {
            deploy(flake, destination, do_preflight, do_test, build_cmdline).await
        })
    }))
    .await?;

    Ok(())
}

#[instrument(skip(flake, destination, do_test, build_cmdline), fields(flake=flake.resolved_path(), dest=destination.hostname) err)]
async fn deploy(
    flake: Flake,
    destination: Destination,
    do_preflight: Behavior,
    do_test: Behavior,
    build_cmdline: Vec<String>,
) -> Result<(), anyhow::Error> {
    log::event!(log::Level::DEBUG, flake=?flake.resolved_path(), host=?destination.hostname, "Copying");
    flake.copy_closure(&destination.hostname).await?;

    log::debug!("Connecting");
    let flavor = destination.os_flavor.on_connection(
        &destination.hostname,
        Session::connect(&destination.hostname, KnownHosts::Strict)
            .await
            .with_context(|| format!("Connecting to {:?}", &destination.hostname))?,
    );
    log::event!(log::Level::DEBUG, config=?destination.config_name, "Building");
    let built = flake
        .build(flavor, destination.config_name.as_deref(), build_cmdline)
        .await?;

    if do_preflight == Behavior::Run {
        log::event!(log::Level::DEBUG, dest=?destination.hostname, "Checking system health");
        built.preflight_check().await?;
    } else {
        log::event!(log::Level::DEBUG, dest=?destination.hostname, "Skipping system health check");
    }

    if do_test == Behavior::Run {
        log::event!(log::Level::DEBUG, configuration=?built.configuration(), system_name=?built.for_system(), "Testing");
        built.test_config().await?;
    } else {
        log::event!(log::Level::DEBUG, configuration=?built.configuration(), system_name=?built.for_system(), "Skipping test");
    }
    // TODO: rollbacks, maybe?
    log::event!(log::Level::DEBUG, configuration=?built.configuration(), system_name=?built.for_system(), "Activating");
    built.boot_config().await?;
    log::event!(log::Level::INFO, configuration=?built.configuration(), system_name=?built.for_system(), "Successfully activated");
    Ok(())
}
