use std::collections::BTreeMap;
use std::env;
use std::path::Path;
use std::str::FromStr;
use std::sync::LazyLock;

use anyhow::Context;
use clap::Parser;
use strum::VariantNames;

mod cli;
mod config;
mod handler;

#[derive(
    Clone, Debug, Default, Eq, PartialEq, strum::Display, strum::EnumString, strum::VariantNames,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "kebab-case")]
pub enum RsopMode {
    Preview,
    #[default]
    Open,
    XdgOpen,
    Edit,
    Identify,
}

static BIN_NAME_TO_MODE: LazyLock<BTreeMap<&'static str, RsopMode>> = LazyLock::new(|| {
    BTreeMap::from([
        ("rsp", RsopMode::Preview),
        ("rso", RsopMode::Open),
        ("xdg-open", RsopMode::XdgOpen),
        ("rse", RsopMode::Edit),
        ("rsi", RsopMode::Identify),
    ])
});

fn runtime_mode() -> RsopMode {
    // Get from env var
    let env_mode = env::var("RSOP_MODE");
    if let Ok(env_mode) = env_mode {
        return RsopMode::from_str(&env_mode)
            .unwrap_or_else(|_| panic!("Unexpected value for RSOP_MODE: {:?}", env_mode));
    }

    // Get from binary name (env::current_exe() follows symbolic links, so don't use it)
    let first_arg = env::args().next().unwrap();
    let bin_name: Option<&str> = Path::new(&first_arg)
        .file_name()
        .map(|f| f.to_str())
        .unwrap();
    if let Some(bin_name) = bin_name {
        if let Some(mode) = BIN_NAME_TO_MODE.get(bin_name) {
            return mode.to_owned();
        }
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

fn main() -> anyhow::Result<()> {
    // Init logger
    simple_logger::SimpleLogger::new()
        .init()
        .context("Failed to init logger")?;

    // Parse command line opts
    let mode = runtime_mode();
    log::trace!("Runtime mode: {:?}", mode);
    let cl_opts = cli::CommandLineOpts::parse();
    log::trace!("{:?}", cl_opts);

    // Parse config
    let cfg = config::parse_config().context("Failed to read config")?;

    // Build mapping for fast searches
    let handlers = handler::HandlerMapping::new(&cfg).context("Failed to build handler mapping")?;
    log::debug!("{:?}", handlers);

    // Do the job
    if let Some(path) = cl_opts.path {
        handlers.handle_path(mode, &path)?;
    } else {
        handlers.handle_pipe(mode)?;
    }

    Ok(())
}
