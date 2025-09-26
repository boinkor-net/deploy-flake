use clap_duration::duration_range_value_parse;
use tokio::task;
use tokio::time::timeout;
use tracing as log;
use tracing::instrument;

use anyhow::Context;
use backon::Retryable as _;
use clap::Parser;
use deploy_flake::{Destination, Flake};
use duration_human::{DurationHuman, DurationHumanValidator};
use openssh::{KnownHosts, Session};
use std::time::Duration;
use std::{path::PathBuf, str::FromStr};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

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

    /// A program contained in the new system closure, run on the
    /// system being deployed, that checks whether the system closure
    /// is deployable. This program can be created with
    /// `system.extraSystemBuilderCmds` for NixOS.  See the
    /// https://github.com/boinkor-net/preroll-safety library for an
    /// example of pre-activation safety checks.
    #[clap(long, require_equals = true, value_name = "PROGRAM")]
    pre_activate_script: Option<PathBuf>,

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

    /// Time to allow for `nix-copy-closure` to succeed.
    /// This program has a bad tendency to hang if any hiccups
    /// on the line occur, but larger closures take longer to copy.
    #[clap(long, value_name = "DURATION", value_parser = duration_range_value_parse!(min: 1s, max: 6h), default_value = "5s")]
    copy_timeout: DurationHuman,
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
    let pre_activate_script = opts.pre_activate_script;
    let build_cmdline = opts.build_cmdline.clone();
    let copy_timeout = (&opts.copy_timeout).into();

    futures::future::try_join_all(opts.to.into_iter().map(|destination| {
        let flake = flake.clone();
        let build_cmdline = build_cmdline.clone();
        let pre_activate_script = pre_activate_script.clone();
        task::spawn(async move {
            deploy(
                flake,
                destination,
                copy_timeout,
                do_preflight,
                pre_activate_script,
                do_test,
                build_cmdline,
            )
            .await
        })
    }))
    .await?;

    Ok(())
}

#[instrument(skip(flake, destination, pre_activate_script, do_test, build_cmdline, copy_timeout), fields(flake=flake.resolved_path(), dest=destination.hostname) err)]
async fn deploy(
    flake: Flake,
    destination: Destination,
    copy_timeout: Duration,
    do_preflight: Behavior,
    pre_activate_script: Option<PathBuf>,
    do_test: Behavior,
    build_cmdline: Vec<String>,
) -> Result<(), anyhow::Error> {
    log::event!(log::Level::DEBUG, flake=?flake.resolved_path(), host=?destination.hostname, "Copying");
    let closure_copier =
        || async { timeout(copy_timeout, flake.copy_closure(&destination.hostname)).await };
    closure_copier
        .retry(backon::ExponentialBuilder::default())
        .notify(|error: &tokio::time::error::Elapsed, backoff: Duration| {
            log::warn!(%error, ?backoff, "Timed out copying the closure, retrying...");
        })
        .await??;

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
        built.preflight_check_system().await?;
        built
            .preflight_check_closure(pre_activate_script.as_deref())
            .await?;
    } else {
        log::event!(log::Level::DEBUG, dest=?destination.hostname, "Skipping system and closure health check");
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
