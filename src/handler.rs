use std::collections::HashMap;
use std::env;
use std::fs::File;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
use std::io::copy;
use std::io::{self, stdin, Read, Write};
use std::iter;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::rc::Rc;

use crate::config;
use crate::config::{FileFilter, FileHandler, SchemeHandler};
use crate::RsopMode;

#[derive(Debug)]
enum FileProcessor {
    Filter(FileFilter),
    Handler(FileHandler),
}

enum PipeOrTmpFile<T> {
    Pipe(T),
    TmpFile(tempfile::NamedTempFile),
}

impl FileProcessor {
    /// Return true if command string contains a given % prefixed pattern
    fn has_pattern(&self, pattern: char) -> bool {
        let re_str = format!("[^%]%{pattern}");
        let re = regex::Regex::new(&re_str).unwrap();
        let command = match self {
            FileProcessor::Filter(f) => &f.command,
            FileProcessor::Handler(h) => &h.command,
        };
        re.is_match(command)
    }
}

#[derive(Debug)]
struct FileHandlers {
    extensions: HashMap<String, Rc<FileProcessor>>,
    mimes: HashMap<String, Rc<FileProcessor>>,
    default: FileHandler,
}

impl FileHandlers {
    pub fn new(default: &FileHandler) -> FileHandlers {
        FileHandlers {
            extensions: HashMap::new(),
            mimes: HashMap::new(),
            default: default.clone(),
        }
    }

    pub fn add(&mut self, processor: Rc<FileProcessor>, filetype: &config::Filetype) {
        for extension in &filetype.extensions {
            self.extensions.insert(extension.clone(), processor.clone());
        }
        for mime in &filetype.mimes {
            self.mimes.insert(mime.clone(), processor.clone());
        }
    }
}

#[derive(Debug)]
struct SchemeHandlers {
    schemes: HashMap<String, SchemeHandler>,
}

impl SchemeHandlers {
    pub fn new() -> SchemeHandlers {
        SchemeHandlers {
            schemes: HashMap::new(),
        }
    }

    pub fn add(&mut self, handler: &SchemeHandler, scheme: &str) {
        self.schemes.insert(scheme.to_owned(), handler.clone());
    }
}

#[derive(Debug)]
pub struct HandlerMapping {
    handlers_preview: FileHandlers,
    handlers_open: FileHandlers,
    handlers_edit: FileHandlers,
    handlers_scheme: SchemeHandlers,
}

#[derive(thiserror::Error, Debug)]
pub enum HandlerError {
    #[error("Failed to run handler command {:?}: {err}", .cmd.connect(" "))]
    Start { err: io::Error, cmd: Vec<String> },
    #[error("Failed to read input file {path:?}: {err}")]
    Input { err: io::Error, path: PathBuf },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[cfg(any(target_os = "linux", target_os = "android"))]
lazy_static::lazy_static! {
    // How many bytes to read from pipe to guess MIME type, use a full memory page
    static ref PIPE_INITIAL_READ_LENGTH: usize =
        nix::unistd::sysconf(nix::unistd::SysconfVar::PAGE_SIZE).expect("Unable to get page size").unwrap() as usize;
}
#[cfg(not(any(target_os = "linux", target_os = "android")))]
lazy_static::lazy_static! {
    static ref PIPE_INITIAL_READ_LENGTH: usize = 4096;
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
trait ReadPipe: Read + Send {}

#[cfg(any(target_os = "linux", target_os = "android"))]
trait ReadPipe: Read + AsRawFd + Send {}

impl ReadPipe for File {}

impl ReadPipe for ChildStdout {}

impl HandlerMapping {
    pub fn new(cfg: &config::Config) -> anyhow::Result<HandlerMapping> {
        let mut handlers_open = FileHandlers::new(&cfg.default_handler_open);
        let mut handlers_edit = FileHandlers::new(&cfg.default_handler_open);
        let mut handlers_preview = FileHandlers::new(&cfg.default_handler_preview);
        for (name, filetype) in &cfg.filetype {
            let handler_open = cfg.handler_open.get(name).cloned();
            let handler_edit = cfg.handler_edit.get(name).cloned();
            let handler_preview = cfg.handler_preview.get(name).cloned();
            let filter = cfg.filter.get(name).cloned();
            anyhow::ensure!(
                handler_open.is_some()
                    || handler_edit.is_some()
                    || handler_preview.is_some()
                    || filter.is_some(),
                "Filetype {} is not bound to any handler or filter",
                name
            );
            if let Some(handler_open) = handler_open {
                Self::validate_handler(&handler_open)?;
                handlers_open.add(Rc::new(FileProcessor::Handler(handler_open)), filetype);
            }
            if let Some(handler_edit) = handler_edit {
                Self::validate_handler(&handler_edit)?;
                handlers_edit.add(Rc::new(FileProcessor::Handler(handler_edit)), filetype);
            }
            if let Some(handler_preview) = handler_preview {
                Self::validate_handler(&handler_preview)?;
                handlers_preview.add(Rc::new(FileProcessor::Handler(handler_preview)), filetype);
            }
            if let Some(filter) = filter {
                anyhow::ensure!(
                    filter.no_pipe || (Self::count_pattern(&filter.command, 'i') <= 1),
                    "Filter {:?} can not have both 'no_pipe = false' and multiple %i in command",
                    filter
                );
                let proc_filter = Rc::new(FileProcessor::Filter(filter));
                handlers_open.add(proc_filter.clone(), filetype);
                handlers_edit.add(proc_filter.clone(), filetype);
                handlers_preview.add(proc_filter, filetype);
            }
        }

        let mut handlers_scheme = SchemeHandlers::new();
        for (schemes, handler) in &cfg.handler_scheme {
            handlers_scheme.add(handler, schemes);
        }

        Ok(HandlerMapping {
            handlers_preview,
            handlers_open,
            handlers_edit,
            handlers_scheme,
        })
    }

    fn validate_handler(handler: &FileHandler) -> anyhow::Result<()> {
        anyhow::ensure!(
            !handler.no_pipe || handler.wait,
            "Handler {:?} can not have both 'no_pipe = true' and 'wait = false'",
            handler
        );
        anyhow::ensure!(
            handler.no_pipe || (Self::count_pattern(&handler.command, 'i') <= 1),
            "Handler {:?} can not have both 'no_pipe = false' and multiple %i in command",
            handler
        );
        Ok(())
    }

    /// Count number of a given % prefixed pattern in command string
    fn count_pattern(command: &str, pattern: char) -> usize {
        let re_str = format!("[^%]%{pattern}");
        let re = regex::Regex::new(&re_str).unwrap();
        re.find_iter(command).count()
    }

    pub fn handle_path(&self, mode: RsopMode, path: &Path) -> Result<(), HandlerError> {
        if let (RsopMode::XdgOpen, Ok(url)) = (
            &mode,
            url::Url::parse(
                path.to_str()
                    .ok_or_else(|| anyhow::anyhow!("Unable to decode path {:?}", path))?,
            ),
        ) {
            if url.scheme() == "file" {
                let url_path = &url[url::Position::BeforeUsername..];
                let parsed_path = PathBuf::from(url_path);
                log::trace!("url={}, parsed_path={:?}", url, parsed_path);
                self.dispatch_path(&parsed_path, &mode)
            } else {
                self.dispatch_url(&url)
            }
        } else {
            self.dispatch_path(path, &mode)
        }
    }

    pub fn handle_pipe(&self, mode: RsopMode) -> Result<(), HandlerError> {
        let stdin = Self::stdin_reader()?;
        self.dispatch_pipe(stdin, &mode)
    }

    fn path_mime(path: &Path) -> Result<Option<&str>, io::Error> {
        // Rather than read socket/pipe, mimic 'file -ib xxx' behavior and return 'inode/yyy' strings
        let metadata = path.metadata()?;
        let file_type = metadata.file_type();
        let mime = if file_type.is_socket() {
            Some("inode/socket")
        } else if file_type.is_fifo() {
            Some("inode/fifo")
        } else {
            // tree_magic_mini::from_filepath returns Option and not a Result<_, io::Error>
            // so probe first to properly propagate the proper error cause
            File::open(path)?;
            tree_magic_mini::from_filepath(path)
        };
        log::debug!("MIME: {:?}", mime);

        Ok(mime)
    }

    #[allow(clippy::wildcard_in_or_patterns)]
    fn dispatch_path(&self, path: &Path, mode: &RsopMode) -> Result<(), HandlerError> {
        // Handler candidates
        let (handlers, next_handlers) = match mode {
            RsopMode::Preview => (&self.handlers_preview, None),
            RsopMode::Edit => (&self.handlers_edit, Some(&self.handlers_open)),
            RsopMode::Open | _ => (&self.handlers_open, Some(&self.handlers_edit)),
        };

        let handlers_it = iter::once(handlers).chain(next_handlers.into_iter());
        let mut mime = None;

        for handlers in handlers_it {
            if *mode != RsopMode::Identify {
                for extension in Self::path_extensions(path)? {
                    if let Some(handler) = handlers.extensions.get(&extension) {
                        let mime = if handler.has_pattern('m') {
                            // Probe MIME type even if we already found a handler, to substitute in command
                            Self::path_mime(path).map_err(|e| HandlerError::Input {
                                err: e,
                                path: path.to_owned(),
                            })?
                        } else {
                            None
                        };
                        return self.run_path(handler, path, mode, mime);
                    }
                }
            }

            if mime.is_none() {
                mime = Self::path_mime(path).map_err(|e| HandlerError::Input {
                    err: e,
                    path: path.to_owned(),
                })?;
            }
            if let RsopMode::Identify = mode {
                println!(
                    "{}",
                    mime.ok_or_else(|| anyhow::anyhow!("Unable to get MIME type for {:?}", path))?
                );
                return Ok(());
            }

            if let Some(mime) = mime {
                if let Some(handler) = handlers.mimes.get(mime) {
                    return self.run_path(handler, path, mode, Some(mime));
                }

                // Try "main" MIME type
                let mime_main = mime.split('/').next();
                if let Some(mime_main) = mime_main {
                    if let Some(handler) = handlers.mimes.get(mime_main) {
                        return self.run_path(handler, path, mode, Some(mime));
                    }
                }
            }
        }

        // Fallback
        self.run_path(
            &FileProcessor::Handler(handlers.default.to_owned()),
            path,
            mode,
            mime,
        )
    }

    #[allow(clippy::wildcard_in_or_patterns)]
    fn dispatch_pipe<T>(&self, mut pipe: T, mode: &RsopMode) -> Result<(), HandlerError>
    where
        T: ReadPipe,
    {
        // Handler candidates
        let handlers = match mode {
            RsopMode::Preview => &self.handlers_preview,
            RsopMode::Open | _ => &self.handlers_open,
        };

        // Read header
        log::trace!(
            "Using max header length of {} bytes",
            *PIPE_INITIAL_READ_LENGTH
        );
        let mut buffer: Vec<u8> = vec![0; *PIPE_INITIAL_READ_LENGTH];
        let header_len = pipe.read(&mut buffer)?;
        let header = &buffer[0..header_len];

        let mime = tree_magic_mini::from_u8(header);
        log::debug!("MIME: {:?}", mime);
        if let RsopMode::Identify = mode {
            println!("{mime}");
            return Ok(());
        }

        if let Some(handler) = handlers.mimes.get(mime) {
            return self.run_pipe(handler, header, pipe, Some(mime), mode);
        }

        // Try "main" MIME type
        let mime_main = mime.split('/').next();
        if let Some(mime_main) = mime_main {
            if let Some(handler) = handlers.mimes.get(mime_main) {
                return self.run_pipe(handler, header, pipe, Some(mime), mode);
            }
        }

        // Fallback
        self.run_pipe(
            &FileProcessor::Handler(handlers.default.to_owned()),
            header,
            pipe,
            Some(mime),
            mode,
        )
    }

    fn dispatch_url(&self, url: &url::Url) -> Result<(), HandlerError> {
        let scheme = url.scheme();
        if let Some(handler) = self.handlers_scheme.schemes.get(scheme) {
            return self.run_url(handler, url);
        }

        Err(HandlerError::Other(anyhow::anyhow!(
            "No handler for scheme {:?}",
            scheme
        )))
    }

    // Substitute % prefixed patterns in string
    fn substitute(s: &str, path: &Path, mime: Option<&str>, term_size: &termsize::Size) -> String {
        let mut r = s.to_string();

        let mut path_arg = path
            .to_str()
            .unwrap_or_else(|| panic!("Invalid path {:?}", path))
            .to_string();
        if !path_arg.is_empty() {
            path_arg = shlex::quote(&path_arg).to_string();
        }

        const BASE_SUBST_REGEX: &str = "([^%])(%{})";
        const BASE_SUBST_UNESCAPE_SRC: &str = "%%";
        const BASE_SUBST_UNESCAPE_DST: &str = "%";
        let mut subst_params: Vec<(String, &str, &str, &str)> = vec![
            (
                format!("{}", term_size.cols),
                const_format::str_replace!(BASE_SUBST_REGEX, "{}", "c"),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_SRC, 'c'),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_DST, 'c'),
            ),
            (
                format!("{}", term_size.rows),
                const_format::str_replace!(BASE_SUBST_REGEX, "{}", "l"),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_SRC, 'l'),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_DST, 'l'),
            ),
            (
                path_arg,
                const_format::str_replace!(BASE_SUBST_REGEX, "{}", "i"),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_SRC, 'i'),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_DST, 'i'),
            ),
        ];
        if let Some(mime) = mime {
            subst_params.push((
                mime.to_string(),
                const_format::str_replace!(BASE_SUBST_REGEX, "{}", "m"),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_SRC, 'm'),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_DST, 'm'),
            ));
        }
        for (val, re_str, unescape_src, unescape_dst) in subst_params {
            let re = regex::Regex::new(re_str).unwrap();
            r = re.replace_all(&r, format!("${{1}}{val}")).to_string();
            r = r.replace(unescape_src, unescape_dst);
        }

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

    fn run_path(
        &self,
        processor: &FileProcessor,
        path: &Path,
        mode: &RsopMode,
        mime: Option<&str>,
    ) -> Result<(), HandlerError> {
        let term_size = Self::term_size();

        match processor {
            FileProcessor::Handler(handler) => {
                self.run_path_handler(handler, path, mime, &term_size)
            }
            FileProcessor::Filter(filter) => {
                let mut filter_child = self.run_path_filter(filter, path, mime, &term_size)?;
                let r = self.dispatch_pipe(filter_child.stdout.take().unwrap(), mode);
                filter_child.kill()?;
                filter_child.wait()?;
                r
            }
        }
    }

    fn run_path_filter(
        &self,
        filter: &FileFilter,
        path: &Path,
        mime: Option<&str>,
        term_size: &termsize::Size,
    ) -> Result<Child, HandlerError> {
        let cmd = Self::substitute(&filter.command, path, mime, term_size);
        let cmd_args = Self::build_cmd(&cmd, filter.shell)?;

        let mut command = Command::new(&cmd_args[0]);
        command
            .args(&cmd_args[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| HandlerError::Start {
                err: e,
                cmd: cmd_args.to_owned(),
            })
    }

    fn run_path_handler(
        &self,
        handler: &FileHandler,
        path: &Path,
        mime: Option<&str>,
        term_size: &termsize::Size,
    ) -> Result<(), HandlerError> {
        let cmd = Self::substitute(&handler.command, path, mime, term_size);
        let cmd_args = Self::build_cmd(&cmd, handler.shell)?;

        let mut command = Command::new(&cmd_args[0]);
        command.args(&cmd_args[1..]).stdin(Stdio::null());
        if handler.wait {
            command
                .status()
                .map(|_| ())
                .map_err(|e| HandlerError::Start {
                    err: e,
                    cmd: cmd_args.to_owned(),
                })
        } else {
            command.stdout(Stdio::null());
            command.stderr(Stdio::null());
            command
                .spawn()
                .map(|_| ())
                .map_err(|e| HandlerError::Start {
                    err: e,
                    cmd: cmd_args.to_owned(),
                })
        }
    }

    fn run_pipe<T>(
        &self,
        processor: &FileProcessor,
        header: &[u8],
        pipe: T,
        mime: Option<&str>,
        mode: &RsopMode,
    ) -> Result<(), HandlerError>
    where
        T: ReadPipe,
    {
        let term_size = Self::term_size();

        match processor {
            FileProcessor::Handler(handler) => {
                self.run_pipe_handler(handler, header, pipe, mime, &term_size)
            }
            FileProcessor::Filter(filter) => crossbeam_utils::thread::scope(|scope| {
                // Write to a temporary file if filter does not support reading from stdin
                let input = if filter.no_pipe {
                    PipeOrTmpFile::TmpFile(Self::pipe_to_tmpfile(header, pipe)?)
                } else {
                    PipeOrTmpFile::Pipe(pipe)
                };

                // Run
                let tmp_file = if let PipeOrTmpFile::TmpFile(ref tmp_file) = input {
                    Some(tmp_file)
                } else {
                    None
                };
                let mut filter_child = self.run_pipe_filter(filter, mime, tmp_file, &term_size)?;
                let filter_child_stdout = filter_child.stdout.take().unwrap();

                if let PipeOrTmpFile::Pipe(mut pipe) = input {
                    // Send data to filter
                    let mut filter_child_stdin = filter_child.stdin.take().unwrap();
                    scope.spawn(move |_| {
                        Self::pipe_forward(&mut pipe, &mut filter_child_stdin, header)
                    });
                }

                // Dispatch to next handler/filter
                let r = self.dispatch_pipe(filter_child_stdout, mode);

                // Cleanup
                filter_child.kill()?;
                filter_child.wait()?;

                r
            })
            .map_err(|e| anyhow::anyhow!("Worker thread error: {:?}", e))?,
        }
    }

    fn run_pipe_filter(
        &self,
        filter: &FileFilter,
        mime: Option<&str>,
        tmp_file: Option<&tempfile::NamedTempFile>,
        term_size: &termsize::Size,
    ) -> Result<Child, HandlerError> {
        // Build command
        let path = if let Some(tmp_file) = tmp_file {
            tmp_file.path().to_path_buf()
        } else if let Some(stdin_arg) = &filter.stdin_arg {
            PathBuf::from(stdin_arg)
        } else {
            PathBuf::from("-")
        };
        let cmd = Self::substitute(&filter.command, &path, mime, term_size);
        let cmd_args = Self::build_cmd(&cmd, filter.shell)?;

        // Run
        let mut command = Command::new(&cmd_args[0]);
        command.args(&cmd_args[1..]);
        if tmp_file.is_none() {
            command.stdin(Stdio::piped());
        } else {
            command
                .stdin(Stdio::null())
                .env("RSOP_INPUT_IS_STDIN_COPY", "1");
        }
        command.stdout(Stdio::piped());
        let child = command.spawn().map_err(|e| HandlerError::Start {
            err: e,
            cmd: cmd_args.to_owned(),
        })?;
        Ok(child)
    }

    fn run_pipe_handler<T>(
        &self,
        handler: &FileHandler,
        header: &[u8],
        pipe: T,
        mime: Option<&str>,
        term_size: &termsize::Size,
    ) -> Result<(), HandlerError>
    where
        T: ReadPipe,
    {
        // Write to a temporary file if handler does not support reading from stdin
        let input = if handler.no_pipe {
            PipeOrTmpFile::TmpFile(Self::pipe_to_tmpfile(header, pipe)?)
        } else {
            PipeOrTmpFile::Pipe(pipe)
        };

        // Build command
        let path = if let PipeOrTmpFile::TmpFile(ref tmp_file) = input {
            tmp_file.path().to_path_buf()
        } else if let Some(stdin_arg) = &handler.stdin_arg {
            PathBuf::from(stdin_arg)
        } else {
            PathBuf::from("-")
        };
        let cmd = Self::substitute(&handler.command, &path, mime, term_size);
        let cmd_args = Self::build_cmd(&cmd, handler.shell)?;

        // Run
        let mut command = Command::new(&cmd_args[0]);
        command.args(&cmd_args[1..]);
        if let PipeOrTmpFile::Pipe(_) = input {
            command.stdin(Stdio::piped());
        } else {
            command
                .stdin(Stdio::null())
                .env("RSOP_INPUT_IS_STDIN_COPY", "1");
        }
        if !handler.wait {
            command.stdout(Stdio::null());
            command.stderr(Stdio::null());
        }
        let mut child = command.spawn().map_err(|e| HandlerError::Start {
            err: e,
            cmd: cmd_args.to_owned(),
        })?;

        if let PipeOrTmpFile::Pipe(mut pipe) = input {
            // Send data to handler
            let mut child_stdin = child.stdin.take().unwrap();
            Self::pipe_forward(&mut pipe, &mut child_stdin, header)?;
            drop(child_stdin);
        }

        if handler.wait || handler.no_pipe {
            child.wait()?;
        }

        Ok(())
    }

    fn run_url(&self, handler: &SchemeHandler, url: &url::Url) -> Result<(), HandlerError> {
        let term_size = Self::term_size();

        // Build command
        let path: PathBuf = PathBuf::from(url.to_owned().as_str());
        let cmd = Self::substitute(&handler.command, &path, None, &term_size);
        let cmd_args = Self::build_cmd(&cmd, handler.shell)?;

        // Run
        let mut command = Command::new(&cmd_args[0]);
        command.args(&cmd_args[1..]);
        // To mimic xdg-open, close all input/outputs and detach
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        command.spawn().map_err(|e| HandlerError::Start {
            err: e,
            cmd: cmd_args.to_owned(),
        })?;

        Ok(())
    }

    fn stdin_reader() -> anyhow::Result<File> {
        // Unfortunately, stdin is buffered, and there is no clean way to get it
        // unbuffered to read only what we want for the header, so use fd hack to get an unbuffered reader
        // see https://users.rust-lang.org/t/add-unbuffered-rawstdin-rawstdout/26013
        // On plaforms other than linux we don't care about buffering because we use chunk copy instead of splice
        let stdin = stdin();
        let reader = unsafe { File::from_raw_fd(stdin.as_raw_fd()) };
        Ok(reader)
    }

    // Default chunk copy using stdlib's std::io::copy when splice syscall is not available
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn pipe_forward<S, D>(src: &mut S, dst: &mut D, header: &[u8]) -> anyhow::Result<usize>
    where
        S: Read,
        D: Write,
    {
        dst.write_all(header)?;
        log::trace!("Header written ({} bytes)", header.len());

        let copied = copy(src, dst)? as usize;
        log::trace!(
            "Pipe exhausted, moved {} bytes total",
            header.len() + copied
        );

        Ok(header.len() + copied)
    }

    // Efficient 0-copy implementation using splice
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn pipe_forward<S, D>(src: &mut S, dst: &mut D, header: &[u8]) -> anyhow::Result<usize>
    where
        S: AsRawFd,
        D: AsRawFd + Write,
    {
        dst.write_all(header)?;
        log::trace!("Header written ({} bytes)", header.len());

        let mut c = 0;
        const SPLICE_LEN: usize = 2usize.pow(62); // splice returns -EINVAL for pipe to file with usize::MAX len
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

    fn pipe_to_tmpfile<T>(header: &[u8], mut pipe: T) -> anyhow::Result<tempfile::NamedTempFile>
    where
        T: ReadPipe,
    {
        let mut tmp_file = tempfile::Builder::new()
            .prefix(const_format::concatcp!(env!("CARGO_PKG_NAME"), '_'))
            .tempfile()?;
        log::debug!("Writing to temporary file {:?}", tmp_file.path());
        let file = tmp_file.as_file_mut();
        Self::pipe_forward(&mut pipe, file, header)?;
        file.flush()?;
        Ok(tmp_file)
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

    fn path_extensions(path: &Path) -> anyhow::Result<Vec<String>> {
        let mut extensions = Vec::new();
        if let Some(extension) = path.extension() {
            // Try to get double extension first if we have one
            let filename = path
                .file_name()
                .and_then(|f| f.to_str())
                .ok_or_else(|| anyhow::anyhow!("Unable to get file name from path {:?}", path))?;
            let double_ext_parts: Vec<_> = filename
                .split('.')
                .skip(1)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .take(2)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if double_ext_parts.len() == 2 {
                extensions.push(double_ext_parts.join(".").to_lowercase());
            }
            extensions.push(
                extension
                    .to_str()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Unable to decode extension for path {:?}", path)
                    })?
                    .to_lowercase(),
            );
        }
        Ok(extensions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_pattern() {
        let mut handler = FileHandler {
            command: "a ii".to_string(),
            wait: false,
            shell: false,
            no_pipe: false,
            stdin_arg: Some("".to_string()),
        };
        let mut processor = FileProcessor::Handler(handler.to_owned());
        assert!(!processor.has_pattern('m'));
        assert!(!processor.has_pattern('i'));

        handler.command = "a %i".to_string();
        processor = FileProcessor::Handler(handler.to_owned());
        assert!(!processor.has_pattern('m'));
        assert!(processor.has_pattern('i'));

        handler.command = "a %%i".to_string();
        processor = FileProcessor::Handler(handler);
        assert!(!processor.has_pattern('m'));
        assert!(!processor.has_pattern('i'));
    }

    #[test]
    fn test_count_pattern() {
        assert_eq!(HandlerMapping::count_pattern("aa ii ii", 'm'), 0);
        assert_eq!(HandlerMapping::count_pattern("aa ii ii", 'i'), 0);

        assert_eq!(HandlerMapping::count_pattern("a %i", 'm'), 0);
        assert_eq!(HandlerMapping::count_pattern("a %i", 'i'), 1);

        assert_eq!(HandlerMapping::count_pattern("a %i %i %m", 'm'), 1);
        assert_eq!(HandlerMapping::count_pattern("a %i %i %m", 'i'), 2);

        assert_eq!(HandlerMapping::count_pattern("a %%i %i %%m", 'm'), 0);
        assert_eq!(HandlerMapping::count_pattern("a %%i %i %%m", 'i'), 1);
    }

    #[test]
    fn test_substitute() {
        let term_size = termsize::Size { rows: 84, cols: 85 };
        let path = Path::new("");

        assert_eq!(
            HandlerMapping::substitute("abc def", path, None, &term_size),
            "abc def"
        );
        assert_eq!(
            HandlerMapping::substitute("ab%%c def", path, None, &term_size),
            "ab%c def"
        );
        assert_eq!(
            HandlerMapping::substitute("ab%c def", path, None, &term_size),
            "ab85 def"
        );
    }

    #[test]
    fn test_path_extensions() {
        assert_eq!(
            HandlerMapping::path_extensions(Path::new("/tmp/")).ok(),
            Some(vec![])
        );
        assert_eq!(
            HandlerMapping::path_extensions(Path::new("/tmp/foo")).ok(),
            Some(vec![])
        );
        assert_eq!(
            HandlerMapping::path_extensions(Path::new("/tmp/foo.bar")).ok(),
            Some(vec!["bar".to_string()])
        );
        assert_eq!(
            HandlerMapping::path_extensions(Path::new("/tmp/foo.bar.baz")).ok(),
            Some(vec!["bar.baz".to_string(), "baz".to_string()])
        );
        assert_eq!(
            HandlerMapping::path_extensions(Path::new("/tmp/foo.BaR.bAz")).ok(),
            Some(vec!["bar.baz".to_string(), "baz".to_string()])
        );
        assert_eq!(
            HandlerMapping::path_extensions(Path::new("/tmp/foo.bar.baz.blah")).ok(),
            Some(vec!["baz.blah".to_string(), "blah".to_string()])
        );
    }
}
