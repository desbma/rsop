use std::{
    collections::HashMap,
    fs::File,
    io::Write as _,
    path::{Path, PathBuf},
};

#[derive(Debug, serde::Deserialize)]
pub(crate) struct Filetype {
    #[serde(default)]
    pub extensions: Vec<String>,

    #[serde(default)]
    pub mimes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
pub(crate) struct FileHandler {
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
pub(crate) struct FileFilter {
    pub command: String,
    #[serde(default)]
    pub shell: bool,
    #[serde(default)]
    pub no_pipe: bool,
    pub stdin_arg: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SchemeHandler {
    pub command: String,
    #[serde(default)]
    pub shell: bool,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct Config {
    #[serde(default)]
    pub filetype: HashMap<String, Filetype>,

    #[serde(default)]
    pub handler_preview: HashMap<String, FileHandler>,
    pub default_handler_preview: FileHandler,

    #[serde(default)]
    pub handler_open: HashMap<String, FileHandler>,
    pub default_handler_open: FileHandler,

    #[serde(default)]
    pub handler_edit: HashMap<String, FileHandler>,

    #[serde(default)]
    pub filter: HashMap<String, FileFilter>,

    #[serde(default)]
    pub handler_scheme: HashMap<String, SchemeHandler>,
}

pub(crate) fn parse_config() -> anyhow::Result<Config> {
    parse_config_path(&get_config_path()?)
}

fn get_config_path() -> anyhow::Result<PathBuf> {
    const CONFIG_FILENAME: &str = "config.toml";
    const DEFAULT_CONFIG_STR: &str = include_str!("../config/config.toml.default");
    let binary_name = env!("CARGO_PKG_NAME");
    let xdg_dirs = xdg::BaseDirectories::with_prefix(binary_name);
    let config_filepath = if let Some(p) = xdg_dirs.find_config_file(CONFIG_FILENAME) {
        p
    } else {
        let path = xdg_dirs.place_config_file(CONFIG_FILENAME)?;
        log::warn!("No config file found, creating a default one in {path:?}");
        let mut file = File::create(&path)?;
        file.write_all(DEFAULT_CONFIG_STR.as_bytes())?;
        path
    };

    log::debug!("Config filepath: {config_filepath:?}");

    Ok(config_filepath)
}

fn parse_config_path(path: &Path) -> anyhow::Result<Config> {
    let toml_data = std::fs::read_to_string(path)?;
    log::trace!("Config data: {toml_data:?}");

    let mut config: Config = toml::from_str(&toml_data)?;
    // Normalize extensions to lower case
    for filetype in config.filetype.values_mut() {
        filetype.extensions = filetype
            .extensions
            .iter()
            .map(|e| e.to_lowercase())
            .collect();
    }
    log::trace!("Config: {config:?}");

    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    #[test]
    fn tiny_config() {
        const TINY_CONFIG_STR: &str = include_str!("../config/config.toml.tiny");
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(TINY_CONFIG_STR.as_bytes()).unwrap();

        let res = parse_config_path(config_file.path());
        assert!(res.is_ok());
        let config = res.unwrap();

        assert_eq!(config.filetype.len(), 0);
        assert_eq!(config.handler_preview.len(), 0);
        assert_eq!(
            config.default_handler_preview,
            FileHandler {
                command: "file %i".to_owned(),
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
                command: "cat -A %i".to_owned(),
                wait: true,
                shell: false,
                no_pipe: false,
                stdin_arg: None
            }
        );
        assert_eq!(config.filter.len(), 0);
    }

    #[test]
    fn default_config() {
        const DEFAULT_CONFIG_STR: &str = include_str!("../config/config.toml.default");
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file
            .write_all(DEFAULT_CONFIG_STR.as_bytes())
            .unwrap();

        let res = parse_config_path(config_file.path());
        assert!(res.is_ok());
        let config = res.unwrap();

        assert_eq!(config.filetype.len(), 2);
        assert_eq!(config.handler_preview.len(), 1);
        assert_eq!(
            config.default_handler_preview,
            FileHandler {
                command: "file %i".to_owned(),
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
                command: "cat -A %i".to_owned(),
                wait: true,
                shell: false,
                no_pipe: false,
                stdin_arg: None
            }
        );
        assert_eq!(config.filter.len(), 1);
    }

    #[test]
    fn advanced_config() {
        const ADVANCED_CONFIG_STR: &str = include_str!("../config/config.toml.advanced");
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file
            .write_all(ADVANCED_CONFIG_STR.as_bytes())
            .unwrap();

        let res = parse_config_path(config_file.path());
        assert!(res.is_ok());
        let config = res.unwrap();

        assert_eq!(config.filetype.len(), 35);
        assert_eq!(config.handler_preview.len(), 25);
        assert_eq!(
            config.default_handler_preview,
            FileHandler {
                command: "echo '🔍 MIME: %m'; hexyl --border none %i | head -n $((%l - 1))"
                    .to_owned(),
                wait: true,
                shell: true,
                no_pipe: false,
                stdin_arg: Some(String::new())
            }
        );
        assert_eq!(config.handler_open.len(), 20);
        assert_eq!(
            config.default_handler_open,
            FileHandler {
                command: "hexyl %i | less -R".to_owned(),
                wait: true,
                shell: true,
                no_pipe: false,
                stdin_arg: Some(String::new())
            }
        );
        assert_eq!(config.filter.len(), 5);
    }

    #[test]
    fn missing_default_handlers() {
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(b"").unwrap();

        let err = parse_config_path(config_file.path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("default_handler_preview"));
    }

    #[test]
    fn missing_default_handler_preview() {
        let toml = r#"
[default_handler_open]
command = "cat %i"
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let err = parse_config_path(config_file.path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("default_handler_preview"));
    }

    #[test]
    fn missing_default_handler_open() {
        let toml = r#"
[default_handler_preview]
command = "file %i"
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let err = parse_config_path(config_file.path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("default_handler_open"));
    }

    #[test]
    fn invalid_toml() {
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file
            .write_all(b"this is not valid toml [[[")
            .unwrap();

        let err = parse_config_path(config_file.path()).unwrap_err();
        assert!(err.downcast_ref::<toml::de::Error>().is_some());
    }

    #[test]
    fn nonexistent_file() {
        let err =
            parse_config_path(Path::new("/tmp/nonexistent_rsop_test_config.toml")).unwrap_err();
        assert!(err.downcast_ref::<io::Error>().is_some());
    }

    #[test]
    fn handler_defaults() {
        let toml = r#"
[default_handler_preview]
command = "file %i"

[default_handler_open]
command = "cat %i"

[filetype.text]
mimes = ["text"]

[handler_preview.text]
command = "head %i"
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let config = parse_config_path(config_file.path()).unwrap();
        let text_handler = config.handler_preview.get("text").unwrap();
        assert!(text_handler.wait);
        assert!(!text_handler.shell);
        assert!(!text_handler.no_pipe);
        assert!(text_handler.stdin_arg.is_none());
    }

    #[test]
    fn handler_all_fields() {
        let toml = r#"
[default_handler_preview]
command = "file %i"

[default_handler_open]
command = "cat %i"

[filetype.text]
mimes = ["text"]

[handler_preview.text]
command = "bat %i"
wait = false
shell = true
no_pipe = true
stdin_arg = "/dev/stdin"
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let config = parse_config_path(config_file.path()).unwrap();
        let text_handler = config.handler_preview.get("text").unwrap();
        assert_eq!(
            *text_handler,
            FileHandler {
                command: "bat %i".to_owned(),
                wait: false,
                shell: true,
                no_pipe: true,
                stdin_arg: Some("/dev/stdin".to_owned()),
            }
        );
    }

    #[test]
    fn extension_normalization() {
        let toml = r#"
[default_handler_preview]
command = "file %i"

[default_handler_open]
command = "cat %i"

[filetype.text]
extensions = ["TXT", "Md", "RST"]
mimes = ["text"]

[handler_preview.text]
command = "head %i"
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let config = parse_config_path(config_file.path()).unwrap();
        let text_ft = config.filetype.get("text").unwrap();
        assert_eq!(text_ft.extensions, vec!["txt", "md", "rst"]);
    }

    #[test]
    fn filetype_extensions_only() {
        let toml = r#"
[default_handler_preview]
command = "file %i"

[default_handler_open]
command = "cat %i"

[filetype.text]
extensions = ["txt"]

[handler_preview.text]
command = "head %i"
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let config = parse_config_path(config_file.path()).unwrap();
        let text_ft = config.filetype.get("text").unwrap();
        assert_eq!(text_ft.extensions, vec!["txt"]);
        assert!(text_ft.mimes.is_empty());
    }

    #[test]
    fn filetype_mimes_only() {
        let toml = r#"
[default_handler_preview]
command = "file %i"

[default_handler_open]
command = "cat %i"

[filetype.text]
mimes = ["text/plain"]

[handler_preview.text]
command = "head %i"
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let config = parse_config_path(config_file.path()).unwrap();
        let text_ft = config.filetype.get("text").unwrap();
        assert!(text_ft.extensions.is_empty());
        assert_eq!(text_ft.mimes, vec!["text/plain"]);
    }

    #[test]
    fn filter_config() {
        let toml = r#"
[default_handler_preview]
command = "file %i"

[default_handler_open]
command = "cat %i"

[filetype.gzip]
mimes = ["application/gzip"]

[filter.gzip]
command = "gzip -dc %i"
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let config = parse_config_path(config_file.path()).unwrap();
        let filter = config.filter.get("gzip").unwrap();
        assert_eq!(filter.command, "gzip -dc %i");
        assert!(!filter.shell);
        assert!(!filter.no_pipe);
        assert!(filter.stdin_arg.is_none());
    }

    #[test]
    fn filter_all_fields() {
        let toml = r#"
[default_handler_preview]
command = "file %i"

[default_handler_open]
command = "cat %i"

[filetype.gzip]
mimes = ["application/gzip"]

[filter.gzip]
command = "pigz -dc %i"
shell = true
no_pipe = true
stdin_arg = ""
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let config = parse_config_path(config_file.path()).unwrap();
        let filter = config.filter.get("gzip").unwrap();
        assert_eq!(filter.command, "pigz -dc %i");
        assert!(filter.shell);
        assert!(filter.no_pipe);
        assert_eq!(filter.stdin_arg, Some(String::new()));
    }

    #[test]
    fn scheme_handler_config() {
        let toml = r#"
[default_handler_preview]
command = "file %i"

[default_handler_open]
command = "cat %i"

[handler_scheme.http]
command = "firefox %i"

[handler_scheme.https]
command = "firefox %i"
shell = true
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let config = parse_config_path(config_file.path()).unwrap();
        assert_eq!(config.handler_scheme.len(), 2);
        let http = config.handler_scheme.get("http").unwrap();
        assert_eq!(http.command, "firefox %i");
        assert!(!http.shell);
        let https = config.handler_scheme.get("https").unwrap();
        assert!(https.shell);
    }

    #[test]
    fn handler_edit_config() {
        let toml = r#"
[default_handler_preview]
command = "file %i"

[default_handler_open]
command = "cat %i"

[filetype.text]
mimes = ["text"]

[handler_edit.text]
command = "vim %i"
no_pipe = true
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let config = parse_config_path(config_file.path()).unwrap();
        let edit = config.handler_edit.get("text").unwrap();
        assert_eq!(edit.command, "vim %i");
        assert!(edit.no_pipe);
    }

    #[test]
    fn empty_filetypes_and_handlers() {
        let toml = r#"
[default_handler_preview]
command = "file %i"

[default_handler_open]
command = "cat %i"
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let config = parse_config_path(config_file.path()).unwrap();
        assert!(config.filetype.is_empty());
        assert!(config.handler_preview.is_empty());
        assert!(config.handler_open.is_empty());
        assert!(config.handler_edit.is_empty());
        assert!(config.filter.is_empty());
        assert!(config.handler_scheme.is_empty());
    }

    #[test]
    fn multiple_filetypes() {
        let toml = r#"
[default_handler_preview]
command = "file %i"

[default_handler_open]
command = "cat %i"

[filetype.text]
mimes = ["text"]
extensions = ["txt"]

[filetype.image]
mimes = ["image"]
extensions = ["png", "jpg"]

[filetype.audio]
mimes = ["audio"]

[handler_preview.text]
command = "head %i"

[handler_preview.image]
command = "chafa %i"

[handler_open.audio]
command = "mpv %i"
wait = false
"#;
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.write_all(toml.as_bytes()).unwrap();

        let config = parse_config_path(config_file.path()).unwrap();
        assert_eq!(config.filetype.len(), 3);
        assert_eq!(config.handler_preview.len(), 2);
        assert_eq!(config.handler_open.len(), 1);

        let audio_handler = config.handler_open.get("audio").unwrap();
        assert!(!audio_handler.wait);
    }
}
