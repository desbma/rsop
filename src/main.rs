use std::env;
use std::io;

use structopt::StructOpt;

mod cli;
mod config;
mod handler;

fn main() {
    // Init logger
    simple_logger::SimpleLogger::new()
        .init()
        .expect("Failed to init logger");

    // Parse command line opts
    let cl_opts = cli::CommandLineOpts::from_args();
    log::trace!("{:?}", cl_opts);

    // Parse config
    let cfg = config::parse_config().expect("Failed to read config");

    // Build mapping for fast searches
    let handlers = handler::HandlerMapping::new(&cfg).expect("Failed to build handler mapping");
    log::debug!("{:?}", handlers);

    // Do the job
    let preview: bool = env::args().next().unwrap_or_else(|| "".to_string()) == "rsp";
    if let Some(path) = cl_opts.path {
        if preview {
            handlers.preview_path(&path).unwrap();
        } else {
            handlers.open_path(&path).unwrap();
        }
    } else {
        let stdin = io::stdin();
        if preview {
            handlers.preview_pipe(&stdin).unwrap();
        } else {
            handlers.open_pipe(&stdin).unwrap();
        }
    }
}
