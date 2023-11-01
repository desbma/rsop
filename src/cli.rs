use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[structopt(version=env!("CARGO_PKG_VERSION"), about="Open or preview files.")]
pub struct CommandLineOpts {
    pub path: Option<PathBuf>,
}
