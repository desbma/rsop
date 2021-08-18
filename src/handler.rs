use std::collections::HashMap;
use std::env;
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::fs::File;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
use std::io::{copy, StdinLock};
use std::io::{stdin, Read, Write};
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;

use crate::config;
use crate::config::Handler;
use crate::RsopMode;

#[derive(Debug)]
struct Handlers {
    extensions: HashMap<String, Rc<Handler>>,
    mimes: HashMap<String, Rc<Handler>>,
}

impl Handlers {
    pub fn new() -> Handlers {
        Handlers {
            extensions: HashMap::new(),
            mimes: HashMap::new(),
        }
    }

    pub fn add(&mut self, handler: Rc<Handler>, filetype: &config::Filetype) {
        for extension in &filetype.extensions {
            self.extensions.insert(extension.clone(), handler.clone());
        }
        for mime in &filetype.mimes {
            self.mimes.insert(mime.clone(), handler.clone());
        }
    }
}

#[derive(Debug)]
pub struct HandlerMapping {
    handlers_preview: Handlers,
    default_handler_preview: Handler,
    handlers_open: Handlers,
    default_handler_open: Handler,
}

// How many bytes to read from pipe to guess MIME type, read a full memory page
const PIPE_INITIAL_READ_LENGTH: usize = 4096;

#[cfg(not(any(target_os = "linux", target_os = "android")))]
trait ReadStdin: Read {}

#[cfg(any(target_os = "linux", target_os = "android"))]
trait ReadStdin: Read + AsRawFd {}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
impl ReadStdin for StdinLock<'_> {}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl ReadStdin for File {}

impl HandlerMapping {
    pub fn new(cfg: &config::Config) -> anyhow::Result<HandlerMapping> {
        let mut handlers_open = Handlers::new();
        let mut handlers_preview = Handlers::new();
        for (name, filetype) in &cfg.filetype {
            let handler_open = cfg.handler_open.get(name).cloned().map(Rc::new);
            let handler_preview = cfg.handler_preview.get(name).cloned().map(Rc::new);
            if handler_open.is_none() && handler_preview.is_none() {
                anyhow::bail!("Filetype {} has no handler", name);
            }
            if let Some(handler_open) = handler_open {
                handlers_open.add(handler_open, filetype);
            }
            if let Some(handler_preview) = handler_preview {
                handlers_preview.add(handler_preview, filetype);
            }
        }
        Ok(HandlerMapping {
            default_handler_preview: cfg.default_handler_preview.clone(),
            handlers_preview,
            default_handler_open: cfg.default_handler_open.clone(),
            handlers_open,
        })
    }

    pub fn handle_path(&self, mode: RsopMode, path: &Path) -> anyhow::Result<()> {
        let (handlers, default_handler) = match mode {
            RsopMode::Preview => (&self.handlers_preview, &self.default_handler_preview),
            RsopMode::Open => (&self.handlers_open, &self.default_handler_open),
        };
        Self::dispatch_path(path, handlers, default_handler)
    }

    pub fn handle_pipe(&self, mode: RsopMode) -> anyhow::Result<()> {
        let (handlers, default_handler) = match mode {
            RsopMode::Preview => (&self.handlers_preview, &self.default_handler_preview),
            RsopMode::Open => (&self.handlers_open, &self.default_handler_open),
        };
        Self::dispatch_pipe(handlers, default_handler)
    }

    fn dispatch_path(
        path: &Path,
        handlers: &Handlers,
        default_handler: &Handler,
    ) -> anyhow::Result<()> {
        let extension = path.extension().map(|e| e.to_str()).flatten();
        if let Some(extension) = extension {
            if let Some(handler) = handlers.extensions.get(extension) {
                return Self::run_path(handler, path);
            }
        }
        let mime = tree_magic_mini::from_filepath(path);
        log::debug!("MIME: {:?}", mime);
        if let Some(mime) = mime {
            if let Some(handler) = handlers.mimes.get(mime) {
                return Self::run_path(handler, path);
            }

            // Try "main" MIME type
            let mime_main = mime.split('/').next();
            if let Some(mime_main) = mime_main {
                if let Some(handler) = handlers.mimes.get(mime_main) {
                    return Self::run_path(handler, path);
                }
            }
        }

        // Fallback
        Self::run_path(default_handler, path)
    }

    fn dispatch_pipe(handlers: &Handlers, default_handler: &Handler) -> anyhow::Result<()> {
        // Read header
        let mut reader = Self::stdin_reader()?;
        let mut buffer = [0; PIPE_INITIAL_READ_LENGTH];
        let header_len = reader.read(&mut buffer)?;
        let header = &buffer[0..header_len];

        let mime = tree_magic_mini::from_u8(header);
        log::debug!("MIME: {:?}", mime);

        if let Some(handler) = handlers.mimes.get(mime) {
            return Self::run_pipe(handler, header, reader);
        }

        // Try "main" MIME type
        let mime_main = mime.split('/').next();
        if let Some(mime_main) = mime_main {
            if let Some(handler) = handlers.mimes.get(mime_main) {
                return Self::run_pipe(handler, header, reader);
            }
        }

        // Fallback
        Self::run_pipe(default_handler, header, reader)
    }

    fn substitute(s: &str, path: &Path, term_size: &termsize::Size) -> String {
        let mut r = s.to_string();

        lazy_static::lazy_static! {
            static ref COLUMNS_COMMAND_REGEX: regex::Regex = regex::Regex::new(r"([^%])(%c)").unwrap();
        }
        r = COLUMNS_COMMAND_REGEX
            .replace_all(&r, format!("${{1}}{}", term_size.cols))
            .to_string();
        r = r.replace("%%c", "%c");

        lazy_static::lazy_static! {
            static ref LINES_COMMAND_REGEX: regex::Regex = regex::Regex::new(r"([^%])(%l)").unwrap();
        }
        r = LINES_COMMAND_REGEX
            .replace_all(&r, format!("${{1}}{}", term_size.rows))
            .to_string();
        r = r.replace("%%l", "%l");

        lazy_static::lazy_static! {
            static ref INPUT_COMMAND_REGEX: regex::Regex = regex::Regex::new(r"([^%])(%i)").unwrap();
        }
        let mut path_arg = path
            .to_str()
            .unwrap_or_else(|| panic!("Invalid path {:?}", path))
            .to_string();
        if !path_arg.is_empty() {
            path_arg = shlex::quote(&path_arg).to_string();
        }
        r = INPUT_COMMAND_REGEX
            .replace_all(&r, format!("${{1}}{}", path_arg))
            .to_string();
        r = r.replace("%%i", "%i");
        r.trim().to_string()
    }

    // Get terminal size by probing it, reading it from env, or using fallback
    fn term_size() -> termsize::Size {
        match termsize::get() {
            Some(s) => s,
            None => {
                let cols_env = env::var("FZF_PREVIEW_COLUMNS")
                    .ok()
                    .and_then(|v| v.parse::<u16>().ok())
                    .or_else(|| env::var("COLUMNS").ok().and_then(|v| v.parse::<u16>().ok()));
                let rows_env = env::var("FZF_PREVIEW_LINES")
                    .ok()
                    .and_then(|v| v.parse::<u16>().ok())
                    .or_else(|| env::var("LINES").ok().and_then(|v| v.parse::<u16>().ok()));
                if let (Some(cols), Some(rows)) = (cols_env, rows_env) {
                    termsize::Size { rows, cols }
                } else {
                    termsize::Size { rows: 24, cols: 80 }
                }
            }
        }
    }

    fn run_path(handler: &Handler, path: &Path) -> anyhow::Result<()> {
        let term_size = Self::term_size();

        let cmd = Self::substitute(&handler.command, path, &term_size);
        let cmd_args = Self::build_cmd(&cmd, handler.shell)?;

        let mut command = Command::new(&cmd_args[0]);
        command.args(&cmd_args[1..]).stdin(Stdio::null());
        if handler.wait {
            command.status()?;
        } else {
            command.stdout(Stdio::null());
            command.stderr(Stdio::null());
            command.spawn()?;
        }
        Ok(())
    }

    fn run_pipe<T>(handler: &Handler, header: &[u8], mut stdin: T) -> anyhow::Result<()>
    where
        T: ReadStdin,
    {
        let term_size = Self::term_size();

        let path = if let Some(stdin_arg) = &handler.stdin_arg {
            PathBuf::from(stdin_arg)
        } else {
            PathBuf::from("-")
        };
        let cmd = Self::substitute(&handler.command, &path, &term_size);
        let cmd_args = Self::build_cmd(&cmd, handler.shell)?;

        let mut child = Command::new(&cmd_args[0])
            .args(&cmd_args[1..])
            .stdin(Stdio::piped())
            .spawn()?;

        let mut child_stdin = child.stdin.take().unwrap();
        child_stdin.write_all(header)?;
        log::trace!("Header written ({} bytes)", header.len());

        let copied = Self::pipe_copy(&mut stdin, &mut child_stdin)?;
        log::trace!("Pipe exhausted, copied {} bytes", copied);
        drop(child_stdin);
        if handler.wait {
            child.wait()?;
        }

        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn stdin_reader() -> anyhow::Result<StdinLock<'static>> {
        let stdin = Box::leak(Box::new(stdin()));
        Ok(stdin.lock())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn stdin_reader() -> anyhow::Result<File> {
        // Unfortunately, stdin is buffered, and there is no clean way to get it
        // unbuffered to read only what we want for the header, so use fd hack to get an unbuffered reader
        // see https://users.rust-lang.org/t/add-unbuffered-rawstdin-rawstdout/26013
        // On plaforms other than linux we don't care about buffering because we use chunk copy instead of splice
        let stdin = stdin();
        let reader = unsafe { File::from_raw_fd(stdin.as_raw_fd()) };
        Ok(reader)
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    // Default chunk copy using stdlib's std::io::cpy when splice syscall is not available
    fn pipe_copy<S, D>(src: &mut S, dst: &mut D) -> anyhow::Result<u64>
    where
        S: Read,
        D: Write,
    {
        Ok(copy(src, dst)?)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    // Efficient 0-copy implementation using splice
    fn pipe_copy<S, D>(src: &mut S, dst: &mut D) -> anyhow::Result<usize>
    where
        S: AsRawFd,
        D: AsRawFd + Write,
    {
        let mut c = 0;
        const SPLICE_LEN: usize = usize::MAX;
        const SPLICE_FLAGS: nix::fcntl::SpliceFFlags = nix::fcntl::SpliceFFlags::empty();

        loop {
            let moved = nix::fcntl::splice(
                src.as_raw_fd(),
                None,
                dst.as_raw_fd(),
                None,
                SPLICE_LEN,
                SPLICE_FLAGS,
            )?;
            log::trace!("moved = {}", moved);
            if moved == 0 {
                break;
            }
            c += moved;
        }
        Ok(c)
    }

    fn build_cmd(cmd: &str, shell: bool) -> anyhow::Result<Vec<String>> {
        let cmd = if !shell {
            shlex::split(cmd).ok_or_else(|| anyhow::anyhow!("Invalid command {:?}", cmd))?
        } else {
            vec!["sh".to_string(), "-c".to_string(), cmd.to_string()]
        };
        log::debug!("Will run command: {:?}", cmd);
        Ok(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substitute() {
        let term_size = termsize::Size { rows: 84, cols: 85 };
        let path = Path::new("");

        assert_eq!(
            HandlerMapping::substitute("abc def", &path, &term_size),
            "abc def"
        );
        assert_eq!(
            HandlerMapping::substitute("ab%%c def", &path, &term_size),
            "ab%c def"
        );
        assert_eq!(
            HandlerMapping::substitute("ab%c def", &path, &term_size),
            "ab85 def"
        );
    }
}
