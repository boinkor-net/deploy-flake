use std::{path::PathBuf, time::Duration};

use anyhow::Context as _;
use backon::Retryable as _;
use clap::Parser;
use clap_duration::duration_range_value_parse;
use duration_human::{DurationHuman, DurationHumanValidator};
use openssh::{KnownHosts, Session};
use tokio::{task, time::timeout};
use tracing as log;
use tracing::instrument;

use crate::{Destination, Flake, commands::Behavior};

#[derive(Parser, Debug, PartialEq, Eq, Clone)]
pub struct CommonOpts {
    /// Whether to run the "preflight" check, where deploy-flake
    /// checks if the target system is healthy. Running it is usually
    /// a good idea to do, but when updating boot config on a broken
    /// system, it is necessary to skip.
    #[clap(long, require_equals=true, value_name = "BEHAVIOR", default_missing_value = "run", default_value_t = Behavior::Run, value_enum)]
    pub preflight_check: Behavior,

    /// A program contained in the new system closure, run on the
    /// system being deployed, that checks whether the system closure
    /// is deployable. This program can be created with
    /// `system.extraSystemBuilderCmds` for NixOS.  See the
    /// https://github.com/boinkor-net/preroll-safety library for an
    /// example of pre-activation safety checks.
    #[clap(long, require_equals = true, value_name = "PROGRAM")]
    pub pre_activate_script: Option<PathBuf>,

    /// Whether to run the "test" step, updating the system config
    /// in-place before installing a new boot config. The default runs
    /// the test step, use `--test=skip` to directly install the built
    /// boot configuration.
    #[clap(long, require_equals=true, value_name = "BEHAVIOR", default_missing_value = "run", default_value_t = Behavior::Run, value_enum)]
    pub test: Behavior,

    /// Extra commandline arguments passed to the "nix build"
    /// command. Defaults to the arguments needed to activate the
    /// "flake" and "nix-command" features.
    #[clap(
        long,
        value_delimiter = ' ',
        value_parser,
        default_value = "--extra-experimental-features nix-command --extra-experimental-features flakes"
    )]
    pub build_cmdline: Vec<String>,

    /// Time to allow for `nix-copy-closure` to succeed.
    /// This program has a bad tendency to hang if any hiccups
    /// on the line occur, but larger closures take longer to copy.
    #[clap(long, value_name = "DURATION", value_parser = duration_range_value_parse!(min: 1s, max: 6h), default_value = "5s")]
    pub copy_timeout: DurationHuman,
}

#[derive(Parser, Debug, PartialEq, Eq, Clone)]
#[clap(author = "Andreas Fuchs <asf@boinkor.net>")]
pub struct Opts {
    #[clap(flatten)]
    opts: CommonOpts,

    /// The destinations that will be deployed to.
    ///
    /// Each destination is either just a hostname, or a URL of the
    /// form FLAVOR://HOSTNAME/[CONFIGURATION] where FLAVOR is
    /// "nixos", and the optional CONFIGURATION specifies what
    /// nixosConfiguration to build and deploy on the destination
    /// (defaults to the hostname that the remote host reports).
    #[clap(value_parser)]
    pub to: Vec<Destination>,
}

/// Run a full deploy lifecycle on all the given destinations, in parallel.
///
/// 1. Copy configuration
/// 2. Build configuration
/// 3. Apply configuration
/// 4. Assign configuration as bootable.
pub async fn run(flake: Flake, opts: Opts) -> Result<(), anyhow::Error> {
    futures::future::try_join_all(opts.to.into_iter().map(|destination| {
        let flake = flake.clone();
        let opts = opts.opts.clone();
        task::spawn(async move { deploy(flake, destination, opts).await })
    }))
    .await?;
    Ok(())
}

#[instrument(skip(flake, destination, opts), fields(flake=flake.resolved_path(), dest=destination.hostname) err)]
async fn deploy(
    flake: Flake,
    destination: Destination,
    opts: CommonOpts,
) -> Result<(), anyhow::Error> {
    log::event!(log::Level::DEBUG, flake=?flake.resolved_path(), host=?destination.hostname, "Copying");
    let copy_timeout: Duration = (&opts.copy_timeout).into();
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
        .build(
            flavor,
            destination.config_name.as_deref(),
            opts.build_cmdline,
        )
        .await?;

    if opts.preflight_check == Behavior::Run {
        log::event!(log::Level::DEBUG, dest=?destination.hostname, "Checking system health");
        built
            .preflight_check_system()
            .await?
            .map_err(|failed_units| {
                log::event!(log::Level::WARN, ?failed_units, "Failed systemd units");
                anyhow::anyhow!("Refusing to deploy to a system with failed units {failed_units:?}")
            })?;
        built
            .preflight_check_closure(opts.pre_activate_script.as_deref())
            .await?;
    } else {
        log::event!(log::Level::DEBUG, dest=?destination.hostname, "Skipping system and closure health check");
    }

    if opts.test == Behavior::Run {
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
