use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, serde::Deserialize)]
pub struct Filetype {
    #[serde(default)]
    pub extensions: Vec<String>,

    #[serde(default)]
    pub mimes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
pub struct FileHandler {
    pub command: String,
    #[serde(default = "default_file_handler_wait")]
    pub wait: bool,
    #[serde(default)]
    pub shell: bool,
    #[serde(default)]
    pub no_pipe: bool,
    pub stdin_arg: Option<String>,
}

const fn default_file_handler_wait() -> bool {
    true
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct FileFilter {
    pub command: String,
    #[serde(default)]
    pub shell: bool,
    #[serde(default)]
    pub no_pipe: bool,
    pub stdin_arg: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct SchemeHandler {
    pub command: String,
    #[serde(default)]
    pub shell: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct Config {
    #[serde(default)]
    pub filetype: HashMap<String, Filetype>,

    #[serde(default)]
    pub handler_preview: HashMap<String, FileHandler>,
    pub default_handler_preview: FileHandler,

    #[serde(default)]
    pub handler_open: HashMap<String, FileHandler>,
    pub default_handler_open: FileHandler,

    #[serde(default)]
    pub filter: HashMap<String, FileFilter>,

    #[serde(default)]
    pub handler_scheme: HashMap<String, SchemeHandler>,
}

pub fn parse_config() -> anyhow::Result<Config> {
    parse_config_path(&get_config_path()?)
}

fn get_config_path() -> anyhow::Result<PathBuf> {
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

    Ok(config_filepath)
}

fn parse_config_path(path: &Path) -> anyhow::Result<Config> {
    let toml_data = std::fs::read_to_string(path)?;
    log::trace!("Config data: {:?}", toml_data);

    let mut config: Config = toml::from_str(&toml_data)?;
    // Normalize extensions to lower case
    for filetype in config.filetype.values_mut() {
        filetype.extensions = filetype
            .extensions
            .iter()
            .map(|e| e.to_lowercase())
            .collect();
    }
    log::trace!("Config: {:?}", config);

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tiny_config() {
        const TINY_CONFIG_STR: &str = include_str!("../config/config.toml.tiny");
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(&TINY_CONFIG_STR.as_bytes()).unwrap();

        let res = parse_config_path(config_file.path());
        assert!(res.is_ok());
        let config = res.unwrap();

        assert_eq!(config.filetype.len(), 0);
        assert_eq!(config.handler_preview.len(), 0);
        assert_eq!(
            config.default_handler_preview,
            FileHandler {
                command: "file %i".to_string(),
                wait: true,
                shell: false,
                no_pipe: false,
                stdin_arg: None
            }
        );
        assert_eq!(config.handler_open.len(), 0);
        assert_eq!(
            config.default_handler_open,
            FileHandler {
                command: "cat -A %i".to_string(),
                wait: true,
                shell: false,
                no_pipe: false,
                stdin_arg: None
            }
        );
        assert_eq!(config.filter.len(), 0);
    }

    #[test]
    fn test_default_config() {
        const DEFAULT_CONFIG_STR: &str = include_str!("../config/config.toml.default");
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file
            .write_all(&DEFAULT_CONFIG_STR.as_bytes())
            .unwrap();

        let res = parse_config_path(config_file.path());
        assert!(res.is_ok());
        let config = res.unwrap();

        assert_eq!(config.filetype.len(), 2);
        assert_eq!(config.handler_preview.len(), 1);
        assert_eq!(
            config.default_handler_preview,
            FileHandler {
                command: "file %i".to_string(),
                wait: true,
                shell: false,
                no_pipe: false,
                stdin_arg: None
            }
        );
        assert_eq!(config.handler_open.len(), 1);
        assert_eq!(
            config.default_handler_open,
            FileHandler {
                command: "cat -A %i".to_string(),
                wait: true,
                shell: false,
                no_pipe: false,
                stdin_arg: None
            }
        );
        assert_eq!(config.filter.len(), 1);
    }

    #[test]
    fn test_advanced_config() {
        const ADVANCED_CONFIG_STR: &str = include_str!("../config/config.toml.advanced");
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file
            .write_all(&ADVANCED_CONFIG_STR.as_bytes())
            .unwrap();

        let res = parse_config_path(config_file.path());
        assert!(res.is_ok());
        let config = res.unwrap();

        assert_eq!(config.filetype.len(), 30);
        assert_eq!(config.handler_preview.len(), 22);
        assert_eq!(
            config.default_handler_preview,
            FileHandler {
                command: "echo '🔍 MIME: %m'; hexyl --border none %i | head -n $((%l - 1))"
                    .to_string(),
                wait: true,
                shell: true,
                no_pipe: false,
                stdin_arg: Some("".to_string())
            }
        );
        assert_eq!(config.handler_open.len(), 21);
        assert_eq!(
            config.default_handler_open,
            FileHandler {
                command: "hexyl %i | less -R".to_string(),
                wait: true,
                shell: true,
                no_pipe: false,
                stdin_arg: Some("".to_string())
            }
        );
        assert_eq!(config.filter.len(), 5);
    }
}
