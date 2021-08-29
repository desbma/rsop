use std::env;
use std::str::FromStr;

use structopt::StructOpt;

mod cli;
mod config;
mod handler;

#[derive(Debug, strum_macros::EnumString)]
#[strum(ascii_case_insensitive)]
pub enum RsopMode {
    Preview,
    Open,
}

fn runtime_mode() -> RsopMode {
    // Get from env var
    let env_mode = env::var("RSOP_MODE");
    if let Ok(env_mode) = env_mode {
        return RsopMode::from_str(&env_mode)
            .unwrap_or_else(|_| panic!("Unexpected value for RSOP_MODE: {:?}", env_mode));
    }

    // Get from binary name
    match env::args()
        .next()
        .unwrap_or_else(|| "".to_string())
        .as_str()
    {
        "rsp" => return RsopMode::Preview,
        "rso" => return RsopMode::Open,
        _ => {}
    }

    log::warn!(
        "Ambiguous preview/open runtime mode, defaulting to open. \
         Please use rso/rsp commands or set RSOP_MODE to either 'open' or 'preview'."
    );
    RsopMode::Open
}

fn main() {
    // Init logger
    simple_logger::SimpleLogger::new()
        .init()
        .expect("Failed to init logger");
    better_panic::install();

    // Parse command line opts
    let mode = runtime_mode();
    log::trace!("Runtime mode: {:?}", mode);
    let cl_opts = cli::CommandLineOpts::from_args();
    log::trace!("{:?}", cl_opts);

    // Parse config
    let cfg = config::parse_config().expect("Failed to read config");

    // Build mapping for fast searches
    let handlers = handler::HandlerMapping::new(&cfg).expect("Failed to build handler mapping");
    log::debug!("{:?}", handlers);

    // Do the job
    if let Some(path) = cl_opts.path {
        handlers.handle_path(mode, &path).unwrap();
    } else {
        handlers.handle_pipe(mode).unwrap();
    }
}
