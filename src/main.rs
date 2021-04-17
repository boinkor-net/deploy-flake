use clap::Clap;
use deploy_flake::{Flake, Flavor};
use std::path::PathBuf;

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

fn main() -> Result<(), anyhow::Error> {
    let opts: Opts = Opts::parse();
    println!("opts: {:?}", opts);
    let f = Flake::from_path(&opts.flake)?;
    f.copy_closure(&opts.to)?;

    Ok(())
}
