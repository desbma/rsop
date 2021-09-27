use std::collections::BTreeMap;
use std::env;
use std::str::FromStr;

use structopt::StructOpt;
use strum::VariantNames;

mod cli;
mod config;
mod handler;

#[derive(
    Clone,
    Debug,
    PartialEq,
    strum_macros::EnumString,
    strum_macros::EnumVariantNames,
    strum_macros::ToString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "kebab-case")]
pub enum RsopMode {
    Preview,
    Open,
    XdgOpen,
    Identify,
}

impl Default for RsopMode {
    fn default() -> Self {
        RsopMode::Open
    }
}

lazy_static::lazy_static! {
    static ref BIN_NAME_TO_MODE: BTreeMap<&'static str, RsopMode> = {
        let mut m = BTreeMap::new();
        m.insert("rsp", RsopMode::Preview);
        m.insert("rso", RsopMode::Open);
        m.insert("xdg-open", RsopMode::XdgOpen);
        m.insert("rsi", RsopMode::Identify);
        m
    };
}

fn runtime_mode() -> RsopMode {
    // Get from env var
    let env_mode = env::var("RSOP_MODE");
    if let Ok(env_mode) = env_mode {
        return RsopMode::from_str(&env_mode)
            .unwrap_or_else(|_| panic!("Unexpected value for RSOP_MODE: {:?}", env_mode));
    }

    // Get from binary name
    if let Some(mode) = BIN_NAME_TO_MODE.get(
        env::args()
            .next()
            .unwrap_or_else(|| "".to_string())
            .as_str(),
    ) {
        return mode.to_owned();
    }

    let mut sorted_variants = Vec::from(RsopMode::VARIANTS);
    sorted_variants.sort_unstable();
    log::warn!(
        "Ambiguous runtime mode, defaulting to {}. \
         Please use one of the {} commands or set RSOP_MODE to either {}.",
        RsopMode::default().to_string(),
        BIN_NAME_TO_MODE
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join("/"),
        sorted_variants.join("/")
    );
    RsopMode::default()
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
