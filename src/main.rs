//! RSOP

use std::{collections::BTreeMap, env, path::Path, str::FromStr as _, sync::LazyLock};

use anyhow::Context as _;
use clap::Parser as _;
use strum::VariantNames as _;

mod cli;
mod config;
mod handler;

#[derive(
    Clone, Debug, Default, Eq, PartialEq, strum::Display, strum::EnumString, strum::VariantNames,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum RsopMode {
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

fn runtime_mode() -> anyhow::Result<RsopMode> {
    // Get from env var
    let env_mode = env::var("RSOP_MODE");
    if let Ok(env_mode) = env_mode {
        return RsopMode::from_str(&env_mode)
            .with_context(|| format!("Unexpected value for RSOP_MODE: {env_mode:?}"));
    }

    // Get from binary name (env::current_exe() follows symbolic links, so don't use it)
    let first_arg = env::args()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Unable to get current binary path"))?;
    let bin_name: Option<&str> = Path::new(&first_arg)
        .file_name()
        .map(|f| f.to_str())
        .ok_or_else(|| anyhow::anyhow!("Unable to get current binary filename"))?;
    if let Some(bin_name) = bin_name {
        if let Some(mode) = BIN_NAME_TO_MODE.get(bin_name) {
            return Ok(mode.to_owned());
        }
    }

    let mut sorted_variants = Vec::from(RsopMode::VARIANTS);
    sorted_variants.sort_unstable();
    log::warn!(
        "Ambiguous runtime mode, defaulting to {}. \
         Please use one of the {} commands or set RSOP_MODE to either {}.",
        RsopMode::default(),
        BIN_NAME_TO_MODE
            .keys()
            .copied()
            .collect::<Vec<_>>()
            .join("/"),
        sorted_variants.join("/")
    );
    Ok(RsopMode::default())
}

fn main() -> anyhow::Result<()> {
    // Init logger
    simple_logger::SimpleLogger::new()
        .init()
        .context("Failed to init logger")?;

    // Parse command line opts
    let mode = runtime_mode()?;
    log::trace!("Runtime mode: {mode:?}");
    let cl_opts = cli::CommandLineOpts::parse();
    log::trace!("{cl_opts:?}");

    // Parse config
    let cfg = config::parse_config().context("Failed to read config")?;

    // Build mapping for fast searches
    let handlers = handler::HandlerMapping::new(&cfg).context("Failed to build handler mapping")?;
    log::debug!("{handlers:?}");

    // Do the job
    if let Some(path) = cl_opts.path {
        handlers.handle_path(&mode, &path)?;
    } else {
        handlers.handle_pipe(&mode)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;

    #[test]
    fn rsop_mode_default() {
        assert_eq!(RsopMode::default(), RsopMode::Open);
    }

    #[test]
    fn rsop_mode_display() {
        assert_eq!(RsopMode::Preview.to_string(), "preview");
        assert_eq!(RsopMode::Open.to_string(), "open");
        assert_eq!(RsopMode::XdgOpen.to_string(), "xdg-open");
        assert_eq!(RsopMode::Edit.to_string(), "edit");
        assert_eq!(RsopMode::Identify.to_string(), "identify");
    }

    #[test]
    fn rsop_mode_from_str() {
        assert_eq!(RsopMode::from_str("preview").unwrap(), RsopMode::Preview);
        assert_eq!(RsopMode::from_str("open").unwrap(), RsopMode::Open);
        assert_eq!(RsopMode::from_str("xdg-open").unwrap(), RsopMode::XdgOpen);
        assert_eq!(RsopMode::from_str("edit").unwrap(), RsopMode::Edit);
        assert_eq!(RsopMode::from_str("identify").unwrap(), RsopMode::Identify);
    }

    #[test]
    fn rsop_mode_from_str_case_insensitive() {
        assert_eq!(RsopMode::from_str("PREVIEW").unwrap(), RsopMode::Preview);
        assert_eq!(RsopMode::from_str("Preview").unwrap(), RsopMode::Preview);
        assert_eq!(RsopMode::from_str("XDG-OPEN").unwrap(), RsopMode::XdgOpen);
        assert_eq!(RsopMode::from_str("Xdg-Open").unwrap(), RsopMode::XdgOpen);
    }

    #[test]
    fn rsop_mode_from_str_invalid() {
        assert!(RsopMode::from_str("unknown").is_err());
        assert!(RsopMode::from_str("").is_err());
    }

    #[test]
    fn bin_name_to_mode_mappings() {
        assert_eq!(*BIN_NAME_TO_MODE.get("rsp").unwrap(), RsopMode::Preview);
        assert_eq!(*BIN_NAME_TO_MODE.get("rso").unwrap(), RsopMode::Open);
        assert_eq!(
            *BIN_NAME_TO_MODE.get("xdg-open").unwrap(),
            RsopMode::XdgOpen
        );
        assert_eq!(*BIN_NAME_TO_MODE.get("rse").unwrap(), RsopMode::Edit);
        assert_eq!(*BIN_NAME_TO_MODE.get("rsi").unwrap(), RsopMode::Identify);
    }

    #[test]
    fn bin_name_to_mode_count() {
        assert_eq!(BIN_NAME_TO_MODE.len(), 5);
    }

    #[test]
    fn bin_name_to_mode_unknown() {
        assert!(BIN_NAME_TO_MODE.get("unknown").is_none());
        assert!(BIN_NAME_TO_MODE.get("rsop").is_none());
    }

    #[test]
    fn rsop_mode_variants() {
        let variants = RsopMode::VARIANTS;
        assert_eq!(variants.len(), 5);
        assert!(variants.contains(&"preview"));
        assert!(variants.contains(&"open"));
        assert!(variants.contains(&"xdg-open"));
        assert!(variants.contains(&"edit"));
        assert!(variants.contains(&"identify"));
    }

    #[test]
    fn rsop_mode_clone() {
        let mode = RsopMode::Preview;
        let cloned = mode.clone();
        assert_eq!(mode, cloned);
    }

    #[test]
    fn rsop_mode_equality() {
        assert_eq!(RsopMode::Open, RsopMode::Open);
        assert_ne!(RsopMode::Open, RsopMode::Edit);
        assert_ne!(RsopMode::Preview, RsopMode::XdgOpen);
    }
}
