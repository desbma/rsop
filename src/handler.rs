use std::{
    collections::HashMap,
    env,
    fs::File,
    io::{self, copy, stdin, Read, Write},
    iter,
    os::unix::{
        fs::FileTypeExt,
        io::{AsRawFd, FromRawFd},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    rc::Rc,
};

use anyhow::Context as _;

use crate::{
    config,
    config::{FileFilter, FileHandler, SchemeHandler},
    RsopMode,
};

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
        #[expect(clippy::unwrap_used)]
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
    pub(crate) fn new(default: &FileHandler) -> FileHandlers {
        FileHandlers {
            extensions: HashMap::new(),
            mimes: HashMap::new(),
            default: default.clone(),
        }
    }

    pub(crate) fn add(&mut self, processor: &Rc<FileProcessor>, filetype: &config::Filetype) {
        for extension in &filetype.extensions {
            self.extensions
                .insert(extension.clone(), Rc::clone(processor));
        }
        for mime in &filetype.mimes {
            self.mimes.insert(mime.clone(), Rc::clone(processor));
        }
    }
}

#[derive(Debug)]
struct SchemeHandlers {
    schemes: HashMap<String, SchemeHandler>,
}

impl SchemeHandlers {
    pub(crate) fn new() -> SchemeHandlers {
        SchemeHandlers {
            schemes: HashMap::new(),
        }
    }

    pub(crate) fn add(&mut self, handler: &SchemeHandler, scheme: &str) {
        self.schemes.insert(scheme.to_owned(), handler.clone());
    }
}

#[derive(Debug)]
pub(crate) struct HandlerMapping {
    preview: FileHandlers,
    open: FileHandlers,
    edit: FileHandlers,
    scheme: SchemeHandlers,
}

#[derive(thiserror::Error, Debug)]
pub(crate) enum HandlerError {
    #[error("Failed to run handler command {:?}: {err}", .cmd.connect(" "))]
    Start { err: io::Error, cmd: Vec<String> },
    #[error("Failed to read input file {path:?}: {err}")]
    Input { err: io::Error, path: PathBuf },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// How many bytes to read from pipe to guess MIME type, use a full memory page
const PIPE_INITIAL_READ_LENGTH: usize = 4096;

impl HandlerMapping {
    #[expect(clippy::similar_names)]
    pub(crate) fn new(cfg: &config::Config) -> anyhow::Result<HandlerMapping> {
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
                handlers_open.add(&Rc::new(FileProcessor::Handler(handler_open)), filetype);
            }
            if let Some(handler_edit) = handler_edit {
                Self::validate_handler(&handler_edit)?;
                handlers_edit.add(&Rc::new(FileProcessor::Handler(handler_edit)), filetype);
            }
            if let Some(handler_preview) = handler_preview {
                Self::validate_handler(&handler_preview)?;
                handlers_preview.add(&Rc::new(FileProcessor::Handler(handler_preview)), filetype);
            }
            if let Some(filter) = filter {
                anyhow::ensure!(
                    filter.no_pipe || (Self::count_pattern(&filter.command, 'i') <= 1),
                    "Filter {:?} can not have both 'no_pipe = false' and multiple %i in command",
                    filter
                );
                let proc_filter = Rc::new(FileProcessor::Filter(filter));
                handlers_open.add(&Rc::clone(&proc_filter), filetype);
                handlers_edit.add(&Rc::clone(&proc_filter), filetype);
                handlers_preview.add(&Rc::clone(&proc_filter), filetype);
            }
        }

        let mut handlers_scheme = SchemeHandlers::new();
        for (schemes, handler) in &cfg.handler_scheme {
            handlers_scheme.add(handler, schemes);
        }

        Ok(HandlerMapping {
            preview: handlers_preview,
            open: handlers_open,
            edit: handlers_edit,
            scheme: handlers_scheme,
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
        #[expect(clippy::unwrap_used)]
        let re = regex::Regex::new(&re_str).unwrap();
        re.find_iter(command).count()
    }

    pub(crate) fn handle_path(&self, mode: &RsopMode, path: &Path) -> Result<(), HandlerError> {
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
                self.dispatch_path(&parsed_path, mode)
            } else {
                self.dispatch_url(&url)
            }
        } else {
            self.dispatch_path(path, mode)
        }
    }

    pub(crate) fn handle_pipe(&self, mode: &RsopMode) -> Result<(), HandlerError> {
        let stdin = Self::stdin_reader();
        self.dispatch_pipe(stdin, mode)
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

    #[expect(clippy::wildcard_in_or_patterns)]
    fn dispatch_path(&self, path: &Path, mode: &RsopMode) -> Result<(), HandlerError> {
        // Handler candidates, with fallbacks
        let (mode_handlers, next_handlers) = match mode {
            RsopMode::Preview => (&self.preview, None),
            RsopMode::Edit => (&self.edit, Some(&self.open)),
            RsopMode::Open | _ => (&self.open, Some(&self.edit)),
        };

        // Try by extension first
        if *mode != RsopMode::Identify {
            for handlers in iter::once(mode_handlers).chain(next_handlers) {
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
        }

        let mime = Self::path_mime(path).map_err(|e| HandlerError::Input {
            err: e,
            path: path.to_owned(),
        })?;
        if let RsopMode::Identify = mode {
            println!(
                "{}",
                mime.ok_or_else(|| anyhow::anyhow!("Unable to get MIME type for {:?}", path))?
            );
            return Ok(());
        }

        // Match by MIME
        for handlers in iter::once(mode_handlers).chain(next_handlers) {
            if let Some(mime) = mime {
                // Try sub MIME types
                for sub_mime in Self::split_mime(mime) {
                    log::trace!("Trying MIME {sub_mime:?}");
                    if let Some(handler) = handlers.mimes.get(&sub_mime) {
                        return self.run_path(handler, path, mode, Some(&sub_mime));
                    }
                }
            }
        }

        // Fallback
        self.run_path(
            &FileProcessor::Handler(mode_handlers.default.clone()),
            path,
            mode,
            mime,
        )
    }

    #[expect(clippy::wildcard_in_or_patterns)]
    fn dispatch_pipe<T>(&self, mut pipe: T, mode: &RsopMode) -> Result<(), HandlerError>
    where
        T: Read + Send,
    {
        // Handler candidates
        let (mode_handlers, next_handlers) = match mode {
            RsopMode::Preview => (&self.preview, None),
            RsopMode::Edit => (&self.edit, Some(&self.open)),
            RsopMode::Open | _ => (&self.open, Some(&self.edit)),
        };

        // Read header
        log::trace!(
            "Using max header length of {} bytes",
            PIPE_INITIAL_READ_LENGTH
        );
        let mut buffer: Vec<u8> = vec![0; PIPE_INITIAL_READ_LENGTH];
        let header_len = pipe.read(&mut buffer)?;
        let header = &buffer[0..header_len];

        let mime = tree_magic_mini::from_u8(header);
        log::debug!("MIME: {:?}", mime);
        if let RsopMode::Identify = mode {
            println!("{mime}");
            return Ok(());
        }

        for handlers in iter::once(mode_handlers).chain(next_handlers) {
            // Try sub MIME types
            for sub_mime in Self::split_mime(mime) {
                log::trace!("Trying MIME {sub_mime:?}");
                if let Some(handler) = handlers.mimes.get(&sub_mime) {
                    return self.run_pipe(handler, header, pipe, Some(&sub_mime), mode);
                }
            }
        }

        // Fallback
        self.run_pipe(
            &FileProcessor::Handler(mode_handlers.default.clone()),
            header,
            pipe,
            Some(mime),
            mode,
        )
    }

    fn dispatch_url(&self, url: &url::Url) -> Result<(), HandlerError> {
        let scheme = url.scheme();
        if let Some(handler) = self.scheme.schemes.get(scheme) {
            return Self::run_url(handler, url);
        }

        Err(HandlerError::Other(anyhow::anyhow!(
            "No handler for scheme {:?}",
            scheme
        )))
    }

    // Substitute % prefixed patterns in string
    fn substitute(
        s: &str,
        path: &Path,
        mime: Option<&str>,
        term_size: (u16, u16),
    ) -> anyhow::Result<String> {
        const BASE_SUBST_REGEX: &str = "([^%])(%{})";
        const BASE_SUBST_UNESCAPE_SRC: &str = "%%";
        const BASE_SUBST_UNESCAPE_DST: &str = "%";

        let mut r = s.to_owned();

        let mut path_arg = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid path {path:?}"))?
            .to_owned();
        if !path_arg.is_empty() {
            path_arg = shlex::try_quote(&path_arg)
                .with_context(|| format!("Failed to quote string {path_arg:?}"))?
                .to_string();
        }

        let mut subst_params: Vec<(String, &str, &str, &str)> = vec![
            (
                format!("{}", term_size.0),
                const_format::str_replace!(BASE_SUBST_REGEX, "{}", "c"),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_SRC, 'c'),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_DST, 'c'),
            ),
            (
                format!("{}", term_size.1),
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
                mime.to_owned(),
                const_format::str_replace!(BASE_SUBST_REGEX, "{}", "m"),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_SRC, 'm'),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_DST, 'm'),
            ));
        }
        for (val, re_str, unescape_src, unescape_dst) in subst_params {
            #[expect(clippy::unwrap_used)]
            let re = regex::Regex::new(re_str).unwrap();
            r = re.replace_all(&r, format!("${{1}}{val}")).to_string();
            r = r.replace(unescape_src, unescape_dst);
        }

        Ok(r.trim().to_owned())
    }

    // Get terminal size by probing it, reading it from env, or using fallback
    fn term_size() -> (u16, u16) {
        termion::terminal_size().unwrap_or_else(|_| {
            let cols_env = env::var("FZF_PREVIEW_COLUMNS")
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
                .or_else(|| env::var("COLUMNS").ok().and_then(|v| v.parse::<u16>().ok()));
            let rows_env = env::var("FZF_PREVIEW_LINES")
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
                .or_else(|| env::var("LINES").ok().and_then(|v| v.parse::<u16>().ok()));
            if let (Some(cols), Some(rows)) = (cols_env, rows_env) {
                (cols, rows)
            } else {
                (80, 24)
            }
        })
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
                Self::run_path_handler(handler, path, mime, term_size)
            }
            FileProcessor::Filter(filter) => {
                let mut filter_child = Self::run_path_filter(filter, path, mime, term_size)?;
                #[expect(clippy::unwrap_used)]
                let r = self.dispatch_pipe(filter_child.stdout.take().unwrap(), mode);
                filter_child.kill()?;
                filter_child.wait()?;
                r
            }
        }
    }

    fn run_path_filter(
        filter: &FileFilter,
        path: &Path,
        mime: Option<&str>,
        term_size: (u16, u16),
    ) -> Result<Child, HandlerError> {
        let cmd = Self::substitute(&filter.command, path, mime, term_size)?;
        let cmd_args = Self::build_cmd(&cmd, filter.shell)?;

        let mut command = Command::new(&cmd_args[0]);
        command
            .args(&cmd_args[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| HandlerError::Start {
                err: e,
                cmd: cmd_args.clone(),
            })
    }

    fn run_path_handler(
        handler: &FileHandler,
        path: &Path,
        mime: Option<&str>,
        term_size: (u16, u16),
    ) -> Result<(), HandlerError> {
        let cmd = Self::substitute(&handler.command, path, mime, term_size)?;
        let cmd_args = Self::build_cmd(&cmd, handler.shell)?;

        let mut command = Command::new(&cmd_args[0]);
        command.args(&cmd_args[1..]).stdin(Stdio::null());
        if handler.wait {
            command
                .status()
                .map(|_| ())
                .map_err(|e| HandlerError::Start {
                    err: e,
                    cmd: cmd_args.clone(),
                })
        } else {
            command.stdout(Stdio::null());
            command.stderr(Stdio::null());
            command
                .spawn()
                .map(|_| ())
                .map_err(|e| HandlerError::Start {
                    err: e,
                    cmd: cmd_args.clone(),
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
        T: Read + Send,
    {
        let term_size = Self::term_size();

        match processor {
            FileProcessor::Handler(handler) => {
                Self::run_pipe_handler(handler, header, pipe, mime, term_size)
            }
            FileProcessor::Filter(filter) => crossbeam_utils::thread::scope(|scope| {
                // Write to a temporary file if filter does not support reading from stdin
                let input = if filter.no_pipe {
                    PipeOrTmpFile::TmpFile(Self::pipe_to_tmpfile(header, pipe)?)
                } else {
                    PipeOrTmpFile::Pipe(pipe)
                };

                // Run
                let tmp_file = if let PipeOrTmpFile::TmpFile(tmp_file) = &input {
                    Some(tmp_file)
                } else {
                    None
                };
                let mut filter_child = Self::run_pipe_filter(filter, mime, tmp_file, term_size)?;
                #[expect(clippy::unwrap_used)]
                let filter_child_stdout = filter_child.stdout.take().unwrap();

                #[expect(clippy::shadow_unrelated)]
                if let PipeOrTmpFile::Pipe(mut pipe) = input {
                    // Send data to filter
                    #[expect(clippy::unwrap_used)]
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
        filter: &FileFilter,
        mime: Option<&str>,
        tmp_file: Option<&tempfile::NamedTempFile>,
        term_size: (u16, u16),
    ) -> Result<Child, HandlerError> {
        // Build command
        let path = if let Some(tmp_file) = tmp_file {
            tmp_file.path().to_path_buf()
        } else if let Some(stdin_arg) = &filter.stdin_arg {
            PathBuf::from(stdin_arg)
        } else {
            PathBuf::from("-")
        };
        let cmd = Self::substitute(&filter.command, &path, mime, term_size)?;
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
            cmd: cmd_args.clone(),
        })?;
        Ok(child)
    }

    fn run_pipe_handler<T>(
        handler: &FileHandler,
        header: &[u8],
        pipe: T,
        mime: Option<&str>,
        term_size: (u16, u16),
    ) -> Result<(), HandlerError>
    where
        T: Read,
    {
        // Write to a temporary file if handler does not support reading from stdin
        let input = if handler.no_pipe {
            PipeOrTmpFile::TmpFile(Self::pipe_to_tmpfile(header, pipe)?)
        } else {
            PipeOrTmpFile::Pipe(pipe)
        };

        // Build command
        let path = if let PipeOrTmpFile::TmpFile(tmp_file) = &input {
            tmp_file.path().to_path_buf()
        } else if let Some(stdin_arg) = &handler.stdin_arg {
            PathBuf::from(stdin_arg)
        } else {
            PathBuf::from("-")
        };
        let cmd = Self::substitute(&handler.command, &path, mime, term_size)?;
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
            cmd: cmd_args.clone(),
        })?;

        #[expect(clippy::shadow_unrelated)]
        if let PipeOrTmpFile::Pipe(mut pipe) = input {
            // Send data to handler
            #[expect(clippy::unwrap_used)]
            let mut child_stdin = child.stdin.take().unwrap();
            Self::pipe_forward(&mut pipe, &mut child_stdin, header)?;
            drop(child_stdin);
        }

        if handler.wait || handler.no_pipe {
            child.wait()?;
        }

        Ok(())
    }

    fn run_url(handler: &SchemeHandler, url: &url::Url) -> Result<(), HandlerError> {
        let term_size = Self::term_size();

        // Build command
        let path: PathBuf = PathBuf::from(url.to_owned().as_str());
        let cmd = Self::substitute(&handler.command, &path, None, term_size)?;
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
            cmd: cmd_args.clone(),
        })?;

        Ok(())
    }

    fn stdin_reader() -> File {
        let stdin = stdin();
        // SAFETY:
        // Unfortunately, stdin is buffered, and there is no clean way to get it
        // unbuffered to read only what we want for the header, so use fd hack to get an unbuffered reader
        // see https://users.rust-lang.org/t/add-unbuffered-rawstdin-rawstdout/26013
        unsafe { File::from_raw_fd(stdin.as_raw_fd()) }
    }

    fn pipe_forward<S, D>(src: &mut S, dst: &mut D, header: &[u8]) -> anyhow::Result<usize>
    where
        S: Read,
        D: Write,
    {
        dst.write_all(header)?;
        log::trace!("Header written ({} bytes)", header.len());

        #[expect(clippy::cast_possible_truncation)]
        let copied = copy(src, dst)? as usize;
        log::trace!(
            "Pipe exhausted, moved {} bytes total",
            header.len() + copied
        );

        Ok(header.len() + copied)
    }

    fn pipe_to_tmpfile<T>(header: &[u8], mut pipe: T) -> anyhow::Result<tempfile::NamedTempFile>
    where
        T: Read,
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
        let cmd = if shell {
            vec!["sh".to_owned(), "-c".to_owned(), cmd.to_owned()]
        } else {
            shlex::split(cmd).ok_or_else(|| anyhow::anyhow!("Invalid command {:?}", cmd))?
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

    fn split_mime(s: &str) -> Vec<String> {
        let mut r = vec![s.to_owned()];
        let mut base = s.to_owned();
        if let Some((a, _b)) = base.rsplit_once('+') {
            r.push(a.to_owned());
            base = a.to_owned();
        }
        for (dot_idx, _) in base.rmatch_indices('.') {
            #[expect(clippy::string_slice)]
            r.push(base[..dot_idx].to_string());
        }
        if let Some((a, _b)) = base.split_once('/') {
            r.push(a.to_owned());
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_pattern() {
        let mut handler = FileHandler {
            command: "a ii".to_owned(),
            wait: false,
            shell: false,
            no_pipe: false,
            stdin_arg: Some(String::new()),
        };
        let mut processor = FileProcessor::Handler(handler.clone());
        assert!(!processor.has_pattern('m'));
        assert!(!processor.has_pattern('i'));

        handler.command = "a %i".to_owned();
        processor = FileProcessor::Handler(handler.clone());
        assert!(!processor.has_pattern('m'));
        assert!(processor.has_pattern('i'));

        handler.command = "a %%i".to_owned();
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
        let term_size = (85, 84);
        let path = Path::new("");

        assert_eq!(
            HandlerMapping::substitute("abc def", path, None, term_size).unwrap(),
            "abc def"
        );
        assert_eq!(
            HandlerMapping::substitute("ab%%c def", path, None, term_size).unwrap(),
            "ab%c def"
        );
        assert_eq!(
            HandlerMapping::substitute("ab%c def", path, None, term_size).unwrap(),
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
            Some(vec!["bar".to_owned()])
        );
        assert_eq!(
            HandlerMapping::path_extensions(Path::new("/tmp/foo.bar.baz")).ok(),
            Some(vec!["bar.baz".to_owned(), "baz".to_owned()])
        );
        assert_eq!(
            HandlerMapping::path_extensions(Path::new("/tmp/foo.BaR.bAz")).ok(),
            Some(vec!["bar.baz".to_owned(), "baz".to_owned()])
        );
        assert_eq!(
            HandlerMapping::path_extensions(Path::new("/tmp/foo.bar.baz.blah")).ok(),
            Some(vec!["baz.blah".to_owned(), "blah".to_owned()])
        );
    }

    #[test]
    fn test_split_mime() {
        assert_eq!(
            HandlerMapping::split_mime("application/vnd.debian.binary-package"),
            vec![
                "application/vnd.debian.binary-package",
                "application/vnd.debian",
                "application/vnd",
                "application"
            ]
        );

        assert_eq!(
            HandlerMapping::split_mime("application/pkix-cert+pem"),
            vec![
                "application/pkix-cert+pem",
                "application/pkix-cert",
                "application"
            ]
        );
    }
}
