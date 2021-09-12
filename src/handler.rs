use std::collections::HashMap;
use std::env;
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::fs::File;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
use std::io::{copy, StdinLock};
use std::io::{stdin, Read, Write};
use std::os::unix::fs::FileTypeExt;
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::rc::Rc;

use crate::config;
use crate::config::{Filter, Handler};
use crate::RsopMode;

#[derive(Debug)]
enum Processor {
    Filter(Filter),
    Handler(Handler),
}

#[derive(Debug)]
struct Handlers {
    extensions: HashMap<String, Rc<Processor>>,
    mimes: HashMap<String, Rc<Processor>>,
}

impl Handlers {
    pub fn new() -> Handlers {
        Handlers {
            extensions: HashMap::new(),
            mimes: HashMap::new(),
        }
    }

    pub fn add(&mut self, processor: Rc<Processor>, filetype: &config::Filetype) {
        for extension in &filetype.extensions {
            self.extensions.insert(extension.clone(), processor.clone());
        }
        for mime in &filetype.mimes {
            self.mimes.insert(mime.clone(), processor.clone());
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

lazy_static::lazy_static! {
    // How many bytes to read from pipe to guess MIME type, use a full memory page
    static ref PIPE_INITIAL_READ_LENGTH: usize =
        nix::unistd::sysconf(nix::unistd::SysconfVar::PAGE_SIZE).expect("Unable to get page size").unwrap() as usize;
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
trait ReadStdin: Read + Send {}

#[cfg(any(target_os = "linux", target_os = "android"))]
trait ReadStdin: Read + AsRawFd + Send {}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
impl ReadStdin for StdinLock<'_> {}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl ReadStdin for File {}

impl ReadStdin for ChildStdout {}

impl HandlerMapping {
    pub fn new(cfg: &config::Config) -> anyhow::Result<HandlerMapping> {
        let mut handlers_open = Handlers::new();
        let mut handlers_preview = Handlers::new();
        for (name, filetype) in &cfg.filetype {
            let handler_open = cfg.handler_open.get(name).cloned();
            let handler_preview = cfg.handler_preview.get(name).cloned();
            let filter = cfg.filter.get(name).cloned();
            if handler_open.is_none() && handler_preview.is_none() && filter.is_none() {
                anyhow::bail!("Filetype {} is not bound to any handler or filter", name);
            }
            if (handler_open.is_some() || handler_preview.is_some()) && filter.is_some() {
                anyhow::bail!(
                    "Filetype {} can not be bound to both a filter and a handler",
                    name
                );
            }
            if let Some(handler_open) = handler_open {
                handlers_open.add(Rc::new(Processor::Handler(handler_open)), filetype);
            }
            if let Some(handler_preview) = handler_preview {
                handlers_preview.add(Rc::new(Processor::Handler(handler_preview)), filetype);
            }
            if let Some(filter) = filter {
                let proc_filter = Rc::new(Processor::Filter(filter));
                handlers_open.add(proc_filter.clone(), filetype);
                handlers_preview.add(proc_filter, filetype);
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
        let parsed_path = if let Ok(url) = url::Url::parse(
            path.to_str()
                .ok_or_else(|| anyhow::anyhow!("Unable to decode path {:?}", path))?,
        ) {
            // TODO handle other schemes
            let url_path = &url[url::Position::BeforeUsername..];
            let p = PathBuf::from(url_path);
            log::trace!("url={}, parsed_path={:?}", url, p);
            p
        } else {
            path.to_path_buf()
        };
        self.dispatch_path(&parsed_path, &mode)
    }

    pub fn handle_pipe(&self, mode: RsopMode) -> anyhow::Result<()> {
        let stdin = Self::stdin_reader()?;
        self.dispatch_pipe(stdin, &mode)
    }

    #[allow(clippy::wildcard_in_or_patterns)]
    fn dispatch_path(&self, path: &Path, mode: &RsopMode) -> anyhow::Result<()> {
        // Handler candidates
        let (handlers, default_handler) = match mode {
            RsopMode::Preview => (&self.handlers_preview, &self.default_handler_preview),
            RsopMode::Open | _ => (&self.handlers_open, &self.default_handler_open),
        };

        if *mode != RsopMode::Identify {
            let extension = path.extension().map(|e| e.to_str()).flatten();
            if let Some(extension) = extension {
                if let Some(handler) = handlers.extensions.get(extension) {
                    return self.run_path(handler, path, mode);
                }
            }
        }

        // Rather than read socket/pipe, mimic 'file -ib xxx' behavior and return 'inode/yyy' strings
        let file_type = path.metadata()?.file_type();
        let mime = if file_type.is_socket() {
            Some("inode/socket")
        } else if file_type.is_fifo() {
            Some("inode/fifo")
        } else {
            tree_magic_mini::from_filepath(path)
        };
        log::debug!("MIME: {:?}", mime);
        if let RsopMode::Identify = mode {
            println!("{}", mime.unwrap());
            return Ok(());
        }

        if let Some(mime) = mime {
            if let Some(handler) = handlers.mimes.get(mime) {
                return self.run_path(handler, path, mode);
            }

            // Try "main" MIME type
            let mime_main = mime.split('/').next();
            if let Some(mime_main) = mime_main {
                if let Some(handler) = handlers.mimes.get(mime_main) {
                    return self.run_path(handler, path, mode);
                }
            }
        }

        // Fallback
        self.run_path(&Processor::Handler(default_handler.to_owned()), path, mode)
    }

    #[allow(clippy::wildcard_in_or_patterns)]
    fn dispatch_pipe<T>(&self, mut reader: T, mode: &RsopMode) -> anyhow::Result<()>
    where
        T: ReadStdin,
    {
        // Handler candidates
        let (handlers, default_handler) = match mode {
            RsopMode::Preview => (&self.handlers_preview, &self.default_handler_preview),
            RsopMode::Open | _ => (&self.handlers_open, &self.default_handler_open),
        };

        // Read header
        log::trace!(
            "Using max header length of {} bytes",
            *PIPE_INITIAL_READ_LENGTH
        );
        let mut buffer: Vec<u8> = vec![0; *PIPE_INITIAL_READ_LENGTH];
        let header_len = reader.read(&mut buffer)?;
        let header = &buffer[0..header_len];

        let mime = tree_magic_mini::from_u8(header);
        log::debug!("MIME: {:?}", mime);
        if let RsopMode::Identify = mode {
            println!("{}", mime);
            return Ok(());
        }

        if let Some(handler) = handlers.mimes.get(mime) {
            return self.run_pipe(handler, header, reader, mode);
        }

        // Try "main" MIME type
        let mime_main = mime.split('/').next();
        if let Some(mime_main) = mime_main {
            if let Some(handler) = handlers.mimes.get(mime_main) {
                return self.run_pipe(handler, header, reader, mode);
            }
        }

        // Fallback
        self.run_pipe(
            &Processor::Handler(default_handler.to_owned()),
            header,
            reader,
            mode,
        )
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

    fn run_path(&self, processor: &Processor, path: &Path, mode: &RsopMode) -> anyhow::Result<()> {
        let term_size = Self::term_size();

        match processor {
            Processor::Handler(handler) => self.run_path_handler(handler, path, &term_size),
            Processor::Filter(filter) => {
                let mut filter_child = self.run_path_filter(filter, path, &term_size)?;
                let r = self.dispatch_pipe(filter_child.stdout.take().unwrap(), mode);
                filter_child.kill()?;
                filter_child.wait()?;
                r
            }
        }
    }

    fn run_path_filter(
        &self,
        filter: &Filter,
        path: &Path,
        term_size: &termsize::Size,
    ) -> anyhow::Result<Child> {
        let cmd = Self::substitute(&filter.command, path, term_size);
        let cmd_args = Self::build_cmd(&cmd, filter.shell)?;

        let mut command = Command::new(&cmd_args[0]);
        command
            .args(&cmd_args[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped());
        Ok(command.spawn()?)
    }

    fn run_path_handler(
        &self,
        handler: &Handler,
        path: &Path,
        term_size: &termsize::Size,
    ) -> anyhow::Result<()> {
        let cmd = Self::substitute(&handler.command, path, term_size);
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

    fn run_pipe<T>(
        &self,
        processor: &Processor,
        header: &[u8],
        mut stdin: T,
        mode: &RsopMode,
    ) -> anyhow::Result<()>
    where
        T: ReadStdin,
    {
        let term_size = Self::term_size();

        match processor {
            Processor::Handler(handler) => {
                self.run_pipe_handler(handler, header, stdin, &term_size)
            }
            Processor::Filter(filter) => crossbeam_utils::thread::scope(|scope| {
                let mut filter_child = self.run_pipe_filter(filter, &term_size)?;
                let mut filter_child_stdin = filter_child.stdin.take().unwrap();
                let filter_child_stdout = filter_child.stdout.take().unwrap();
                scope.spawn(move |_| {
                    Self::pipe_forward(&mut stdin, &mut filter_child_stdin, header)
                        .expect("Pipe forward failed in thread")
                });
                let r = self.dispatch_pipe(filter_child_stdout, mode);
                filter_child.kill()?;
                filter_child.wait()?;
                r
            })
            .unwrap(),
        }
    }

    fn run_pipe_filter(
        &self,
        filter: &Filter,
        term_size: &termsize::Size,
    ) -> anyhow::Result<Child> {
        let path = if let Some(stdin_arg) = &filter.stdin_arg {
            PathBuf::from(stdin_arg)
        } else {
            PathBuf::from("-")
        };
        let cmd = Self::substitute(&filter.command, &path, term_size);
        let cmd_args = Self::build_cmd(&cmd, filter.shell)?;

        let mut command = Command::new(&cmd_args[0]);
        command
            .args(&cmd_args[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        Ok(command.spawn()?)
    }

    fn run_pipe_handler<T>(
        &self,
        handler: &Handler,
        header: &[u8],
        mut stdin: T,
        term_size: &termsize::Size,
    ) -> anyhow::Result<()>
    where
        T: ReadStdin,
    {
        let path = if let Some(stdin_arg) = &handler.stdin_arg {
            PathBuf::from(stdin_arg)
        } else {
            PathBuf::from("-")
        };
        let cmd = Self::substitute(&handler.command, &path, term_size);
        let cmd_args = Self::build_cmd(&cmd, handler.shell)?;

        let mut command = Command::new(&cmd_args[0]);
        command.args(&cmd_args[1..]).stdin(Stdio::piped());
        if !handler.wait {
            command.stdout(Stdio::null());
            command.stderr(Stdio::null());
        }
        let mut child = command.spawn()?;

        let mut child_stdin = child.stdin.take().unwrap();
        Self::pipe_forward(&mut stdin, &mut child_stdin, header)?;
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
    fn pipe_forward<S, D>(src: &mut S, dst: &mut D, header: &[u8]) -> anyhow::Result<u64>
    where
        S: Read,
        D: Write,
    {
        dst.write_all(header)?;
        log::trace!("Header written ({} bytes)", header.len());

        let copied = copy(src, dst)?;
        log::trace!(
            "Pipe exhausted, moved {} bytes total",
            header.len() + copied
        );

        Ok(header.len() + copied)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    // Efficient 0-copy implementation using splice
    fn pipe_forward<S, D>(src: &mut S, dst: &mut D, header: &[u8]) -> anyhow::Result<usize>
    where
        S: AsRawFd,
        D: AsRawFd + Write,
    {
        dst.write_all(header)?;
        log::trace!("Header written ({} bytes)", header.len());

        let mut c = 0;
        const SPLICE_LEN: usize = usize::MAX;
        const SPLICE_FLAGS: nix::fcntl::SpliceFFlags = nix::fcntl::SpliceFFlags::empty();

        loop {
            let rc = nix::fcntl::splice(
                src.as_raw_fd(),
                None,
                dst.as_raw_fd(),
                None,
                SPLICE_LEN,
                SPLICE_FLAGS,
            );
            let moved = match rc {
                Err(e) if e == nix::errno::Errno::EPIPE => 0,
                Err(e) => return Err(anyhow::Error::new(e)),
                Ok(m) => m,
            };
            log::trace!("moved = {}", moved);
            if moved == 0 {
                break;
            }
            c += moved;
        }

        log::trace!("Pipe exhausted, moved {} bytes total", header.len() + c);

        Ok(header.len() + c)
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
