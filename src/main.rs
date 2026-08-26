use deploy_flake::Flake;
use tracing as log;
use tracing::instrument;

use clap::Parser;
use deploy_flake::commands::Opts;

#[instrument(err)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let opts: Opts = Opts::parse();

    opts.instrumentation.setup();
    log::trace!(cmdline = ?opts);

    let flake = Flake::from_path(&opts.flake)?;
    log::debug!(?flake, "Flake metadata");
    opts.command.run(flake).await?;

    Ok(())
}
