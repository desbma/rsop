use std::path::PathBuf;

use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(version=env!("CARGO_PKG_VERSION"), about="Open or preview files.")]
pub struct CommandLineOpts {
    pub path: Option<PathBuf>,
}
