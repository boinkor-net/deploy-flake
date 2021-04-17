use clap::Clap;
use deploy_flake::Flake;
use std::path::PathBuf;

#[derive(Clap)]
#[clap(author = "Andreas Fuchs <asf@boinkor.net>")]
struct Opts {
    /// The flake source code directory to deploy.
    #[clap(long, default_value = ".")]
    flake: PathBuf,
}

fn main() -> Result<(), anyhow::Error> {
    let opts: Opts = Opts::parse();
    let f = Flake::from_path(&opts.flake)?;
    println!("Flake: {:?}", f);
    Ok(())
}
