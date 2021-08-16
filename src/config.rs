use std::collections::HashMap;
use std::fs::File;
use std::io::Write;

#[derive(Debug, serde::Deserialize)]
pub struct Filetype {
    #[serde(default)]
    pub extensions: Vec<String>,

    #[serde(default)]
    pub mimes: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Handler {
    pub command: String,
    #[serde(default = "default_handler_wait")]
    pub wait: bool,
    #[serde(default)]
    pub shell: bool,
    pub stdin_arg: Option<String>,
}

const fn default_handler_wait() -> bool {
    true
}

#[derive(Debug, serde::Deserialize)]
pub struct Config {
    pub filetype: HashMap<String, Filetype>,

    pub handler_preview: HashMap<String, Handler>,
    pub default_handler_preview: Handler,

    pub handler_open: HashMap<String, Handler>,
    pub default_handler_open: Handler,
}

pub fn parse_config() -> anyhow::Result<Config> {
    const CONFIG_FILENAME: &str = "config.toml";
    const DEFAULT_CONFIG_STR: &str = include_str!("../config/config.toml.default");
    let binary_name = env!("CARGO_PKG_NAME");
    let xdg_dirs = xdg::BaseDirectories::with_prefix(binary_name)?;
    let config_filepath = match xdg_dirs.find_config_file(CONFIG_FILENAME) {
        Some(p) => p,
        None => {
            let path = xdg_dirs.place_config_file(CONFIG_FILENAME)?;
            log::warn!("No config file found, creating a default one in {:?}", path);
            let mut file = File::create(&path)?;
            file.write_all(DEFAULT_CONFIG_STR.as_bytes())?;
            path
        }
    };

    log::debug!("Config filepath: {:?}", config_filepath);

    let toml_data = std::fs::read_to_string(config_filepath)?;
    log::trace!("Config data: {:?}", toml_data);

    let config = toml::from_str(&toml_data)?;
    log::trace!("Config: {:?}", config);

    Ok(config)
}
