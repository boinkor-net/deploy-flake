use ::log::kv;
use clap::Clap;
use deploy_flake::{Flake, Flavor};
use kv_log_macro as log;
use std::path::PathBuf;

mod logging;

#[derive(Clap, Debug)]
#[clap(author = "Andreas Fuchs <asf@boinkor.net>")]
struct Opts {
    /// The flake source code directory to deploy.
    #[clap(long, default_value = ".")]
    flake: PathBuf,

    /// The operating system flavor to deploy to.
    #[clap(long, default_value)]
    os_flavor: Flavor,

    /// The host that we deploy to
    to: String,
}

impl Opts {
    fn as_value(&self) -> kv::Value {
        kv::Value::capture_debug(self)
    }
}

fn main() -> Result<(), anyhow::Error> {
    logging::init()?;

    let opts: Opts = Opts::parse();
    log::trace!("Parsed command line", { opts: opts.as_value() });

    log::trace!("Reading flake", {path: kv::Value::capture_debug(&opts.flake)});
    let f = Flake::from_path(&opts.flake)?;
    log::debug!("Flake metadata", { flake: f.as_value() });

    log::info!("Copying flake", {to: kv::Value::capture_debug(&opts.to)});
    f.copy_closure(&opts.to)?;

    Ok(())
}
