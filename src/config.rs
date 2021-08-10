use std::collections::HashMap;

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    Exec,
    Fork,
    ForkWait,
}

impl Default for RunMode {
    fn default() -> Self {
        RunMode::ForkWait
    }
}

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
    #[serde(default)]
    pub mode: RunMode,
    #[serde(default)]
    pub shell: bool,
    pub stdin_arg: Option<String>,
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
    let binary_name = env!("CARGO_PKG_NAME");
    let xdg_dirs = xdg::BaseDirectories::with_prefix(binary_name)?;
    let config_filepath = xdg_dirs
        .find_config_file("config.toml")
        .ok_or_else(|| anyhow::anyhow!("Unable to find config file"))?;
    log::debug!("Config filepath: {:?}", config_filepath);

    let toml_data = std::fs::read_to_string(config_filepath)?;
    log::trace!("Config data: {:?}", toml_data);

    let config = toml::from_str(&toml_data)?;
    log::trace!("Config: {:?}", config);

    Ok(config)
}
