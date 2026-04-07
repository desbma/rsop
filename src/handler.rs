use std::{
    collections::HashMap,
    env,
    fs::File,
    io::{self, Read, Write, copy, stdin},
    iter,
    os::unix::{
        fs::FileTypeExt as _,
        io::{AsRawFd as _, FromRawFd as _},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    rc::Rc,
};

use anyhow::Context as _;

use crate::{
    RsopMode, config,
    config::{FileFilter, FileHandler, SchemeHandler},
};

#[derive(Debug)]
enum FileProcessor {
    Filter(FileFilter),
    Handler(FileHandler),
}

enum PipeOrTmpFile<T> {
    Pipe(T),
    TmpFile(tempfile::NamedTempFile),
    #[cfg(target_os = "linux")]
    MemFd(File),
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
                "Filetype {name} is not bound to any handler or filter"
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
                    "Filter {filter:?} can not have both 'no_pipe = false' and multiple %i in command"
                );
                let proc_filter = Rc::new(FileProcessor::Filter(filter));
                handlers_open.add(&Rc::clone(&proc_filter), filetype);
                // handlers_edit.add(&Rc::clone(&proc_filter), filetype);
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
        #[cfg(not(target_os = "linux"))]
        anyhow::ensure!(
            !handler.no_pipe || handler.wait,
            "Handler {handler:?} can not have both 'no_pipe = true' and 'wait = false'"
        );
        #[cfg(target_os = "linux")]
        anyhow::ensure!(
            !handler.no_pipe
                || handler.wait
                || (Self::count_pattern(&handler.command, 't') == 0
                    && Self::count_pattern(&handler.command, 'T') == 0),
            "Handler {handler:?} can not have 'no_pipe = true' and 'wait = false' with %t or %T patterns"
        );
        anyhow::ensure!(
            handler.no_pipe || (Self::count_pattern(&handler.command, 'i') <= 1),
            "Handler {handler:?} can not have both 'no_pipe = false' and multiple %i in command"
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
                    .ok_or_else(|| anyhow::anyhow!("Unable to decode path {path:?}"))?,
            ),
        ) {
            if url.scheme() == "file" {
                let url_path = &url[url::Position::BeforeUsername..];
                let parsed_path = PathBuf::from(url_path);
                log::trace!("url={url}, parsed_path={parsed_path:?}");
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
        log::debug!("MIME: {mime:?}");

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
                mime.ok_or_else(|| anyhow::anyhow!("Unable to get MIME type for {path:?}"))?
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
        log::trace!("Using max header length of {PIPE_INITIAL_READ_LENGTH} bytes");
        let mut buffer: Vec<u8> = vec![0; PIPE_INITIAL_READ_LENGTH];
        let header_len = pipe.read(&mut buffer)?;
        let header = &buffer[0..header_len];

        let mime = tree_magic_mini::from_u8(header);
        log::debug!("MIME: {mime:?}");
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
            "No handler for scheme {scheme:?}"
        )))
    }

    // Substitute % prefixed patterns in string
    fn substitute(
        s: &str,
        path: &Path,
        mime: Option<&str>,
        term_size: (u16, u16),
        tmp_file: Option<&tempfile::NamedTempFile>,
        tmp_dir: Option<&tempfile::TempDir>,
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
        if let Some(tmp_file) = tmp_file {
            subst_params.push((
                tmp_file
                    .path()
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Invalid path {path:?}"))?
                    .to_owned(),
                const_format::str_replace!(BASE_SUBST_REGEX, "{}", "t"),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_SRC, 't'),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_DST, 't'),
            ));
        }
        if let Some(tmp_dir) = tmp_dir {
            subst_params.push((
                tmp_dir
                    .path()
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("Invalid path {path:?}"))?
                    .to_owned(),
                const_format::str_replace!(BASE_SUBST_REGEX, "{}", "T"),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_SRC, 'T'),
                const_format::concatcp!(BASE_SUBST_UNESCAPE_DST, 'T'),
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
                .or_else(|| {
                    let v = env::var("COLUMNS").ok()?;
                    v.parse::<u16>().ok()
                });
            let rows_env = env::var("FZF_PREVIEW_LINES")
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
                .or_else(|| {
                    let v = env::var("LINES").ok()?;
                    v.parse::<u16>().ok()
                });
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
        let tmp_file = if Self::count_pattern(&filter.command, 't') > 0 {
            Some(tempfile::NamedTempFile::new()?)
        } else {
            None
        };
        let tmp_dir = if Self::count_pattern(&filter.command, 'T') > 0 {
            Some(tempfile::tempdir()?)
        } else {
            None
        };
        let cmd = Self::substitute(
            &filter.command,
            path,
            mime,
            term_size,
            tmp_file.as_ref(),
            tmp_dir.as_ref(),
        )?;
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
        let tmp_file = if Self::count_pattern(&handler.command, 't') > 0 {
            Some(tempfile::NamedTempFile::new()?)
        } else {
            None
        };
        let tmp_dir = if Self::count_pattern(&handler.command, 'T') > 0 {
            Some(tempfile::tempdir()?)
        } else {
            None
        };
        let cmd = Self::substitute(
            &handler.command,
            path,
            mime,
            term_size,
            tmp_file.as_ref(),
            tmp_dir.as_ref(),
        )?;
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
            .map_err(|e| anyhow::anyhow!("Worker thread error: {e:?}"))?,
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
        let tmp_file2 = if Self::count_pattern(&filter.command, 't') > 0 {
            Some(tempfile::NamedTempFile::new()?)
        } else {
            None
        };
        let tmp_dir = if Self::count_pattern(&filter.command, 'T') > 0 {
            Some(tempfile::tempdir()?)
        } else {
            None
        };
        let cmd = Self::substitute(
            &filter.command,
            &path,
            mime,
            term_size,
            tmp_file2.as_ref(),
            tmp_dir.as_ref(),
        )?;
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
        // Write to a temporary file (or memfd) if handler does not support reading from stdin
        let input = if handler.no_pipe {
            #[cfg(target_os = "linux")]
            {
                if handler.wait {
                    PipeOrTmpFile::TmpFile(Self::pipe_to_tmpfile(header, pipe)?)
                } else {
                    PipeOrTmpFile::MemFd(Self::pipe_to_memfd(header, pipe)?)
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                PipeOrTmpFile::TmpFile(Self::pipe_to_tmpfile(header, pipe)?)
            }
        } else {
            PipeOrTmpFile::Pipe(pipe)
        };

        // Build command
        let path = match &input {
            PipeOrTmpFile::TmpFile(tmp_file) => tmp_file.path().to_path_buf(),
            #[cfg(target_os = "linux")]
            PipeOrTmpFile::MemFd(file) => {
                PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
            }
            PipeOrTmpFile::Pipe(_) => {
                if let Some(stdin_arg) = &handler.stdin_arg {
                    PathBuf::from(stdin_arg)
                } else {
                    PathBuf::from("-")
                }
            }
        };
        let tmp_file2 = if Self::count_pattern(&handler.command, 't') > 0 {
            Some(tempfile::NamedTempFile::new()?)
        } else {
            None
        };
        let tmp_dir = if Self::count_pattern(&handler.command, 'T') > 0 {
            Some(tempfile::tempdir()?)
        } else {
            None
        };
        let cmd = Self::substitute(
            &handler.command,
            &path,
            mime,
            term_size,
            tmp_file2.as_ref(),
            tmp_dir.as_ref(),
        )?;
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

        if handler.wait {
            child.wait()?;
        }

        Ok(())
    }

    fn run_url(handler: &SchemeHandler, url: &url::Url) -> Result<(), HandlerError> {
        let term_size = Self::term_size();

        // Build command
        let path: PathBuf = PathBuf::from(url.to_owned().as_str());
        let tmp_file = if Self::count_pattern(&handler.command, 't') > 0 {
            Some(tempfile::NamedTempFile::new()?)
        } else {
            None
        };
        let tmp_dir = if Self::count_pattern(&handler.command, 'T') > 0 {
            Some(tempfile::tempdir()?)
        } else {
            None
        };
        let cmd = Self::substitute(
            &handler.command,
            &path,
            None,
            term_size,
            tmp_file.as_ref(),
            tmp_dir.as_ref(),
        )?;
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

    #[cfg(target_os = "linux")]
    fn pipe_to_memfd<T>(header: &[u8], mut pipe: T) -> anyhow::Result<File>
    where
        T: Read,
    {
        use std::io::Seek as _;
        // Create the memfd *without* MFD_CLOEXEC so the fd is inherited by
        // the child across exec() — that's what keeps the memfd alive once
        // the parent exits.
        let name = std::ffi::CString::new(env!("CARGO_PKG_NAME")).context("Invalid memfd name")?;
        let fd = nix::sys::memfd::memfd_create(name.as_c_str(), nix::sys::memfd::MFdFlags::empty())
            .context("memfd_create failed")?;
        let mut file = File::from(fd);
        log::debug!("Writing to memfd at fd {}", file.as_raw_fd());
        Self::pipe_forward(&mut pipe, &mut file, header)?;
        file.seek(io::SeekFrom::Start(0))?;
        Ok(file)
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
            shlex::split(cmd).ok_or_else(|| anyhow::anyhow!("Invalid command {cmd:?}"))?
        };
        log::debug!("Will run command: {cmd:?}");
        Ok(cmd)
    }

    fn path_extensions(path: &Path) -> anyhow::Result<Vec<String>> {
        let mut extensions = Vec::new();
        if let Some(extension) = path.extension() {
            // Try to get double extension first if we have one
            let filename = path
                .file_name()
                .and_then(|f| f.to_str())
                .ok_or_else(|| anyhow::anyhow!("Unable to get file name from path {path:?}"))?;
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
                    .ok_or_else(|| anyhow::anyhow!("Unable to decode extension for path {path:?}"))?
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
    fn has_pattern() {
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
    fn count_pattern() {
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
    fn substitute() {
        let term_size = (85, 84);
        let path = Path::new("");

        assert_eq!(
            HandlerMapping::substitute("abc def", path, None, term_size, None, None).unwrap(),
            "abc def"
        );
        assert_eq!(
            HandlerMapping::substitute("ab%%c def", path, None, term_size, None, None).unwrap(),
            "ab%c def"
        );
        assert_eq!(
            HandlerMapping::substitute("ab%c def", path, None, term_size, None, None).unwrap(),
            "ab85 def"
        );
    }

    #[test]
    fn path_extensions() {
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
    fn split_mime() {
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

    #[test]
    fn split_mime_simple() {
        assert_eq!(
            HandlerMapping::split_mime("text/plain"),
            vec!["text/plain", "text"]
        );
    }

    #[test]
    fn split_mime_no_slash() {
        assert_eq!(HandlerMapping::split_mime("text"), vec!["text"]);
    }

    #[test]
    fn split_mime_plus_and_dots() {
        assert_eq!(
            HandlerMapping::split_mime("application/vnd.oasis.opendocument+xml"),
            vec![
                "application/vnd.oasis.opendocument+xml",
                "application/vnd.oasis.opendocument",
                "application/vnd.oasis",
                "application/vnd",
                "application"
            ]
        );
    }

    #[test]
    fn path_extensions_hidden_file() {
        assert_eq!(
            HandlerMapping::path_extensions(Path::new("/tmp/.hidden")).ok(),
            Some(vec![])
        );
    }

    #[test]
    fn path_extensions_hidden_file_with_ext() {
        assert_eq!(
            HandlerMapping::path_extensions(Path::new("/tmp/.hidden.txt")).ok(),
            Some(vec!["hidden.txt".to_owned(), "txt".to_owned()])
        );
    }

    #[test]
    fn path_extensions_single_dot() {
        assert_eq!(
            HandlerMapping::path_extensions(Path::new("/tmp/foo.")).ok(),
            Some(vec![String::new()])
        );
    }

    #[test]
    fn substitute_path() {
        let term_size = (80, 24);
        let path = Path::new("/tmp/test.txt");

        assert_eq!(
            HandlerMapping::substitute("cat %i", path, None, term_size, None, None).unwrap(),
            "cat /tmp/test.txt"
        );
    }

    #[test]
    fn substitute_mime() {
        let term_size = (80, 24);
        let path = Path::new("/tmp/test.txt");

        assert_eq!(
            HandlerMapping::substitute("echo %m", path, Some("text/plain"), term_size, None, None)
                .unwrap(),
            "echo text/plain"
        );
    }

    #[test]
    fn substitute_term_size() {
        let term_size = (120, 40);
        let path = Path::new("");

        assert_eq!(
            HandlerMapping::substitute("head -n %l %i", path, None, term_size, None, None).unwrap(),
            "head -n 40"
        );
        assert_eq!(
            HandlerMapping::substitute("cols=%c lines=%l", path, None, term_size, None, None)
                .unwrap(),
            "cols=120 lines=40"
        );
    }

    #[test]
    fn substitute_double_percent_escape() {
        let term_size = (80, 24);
        let path = Path::new("");

        assert_eq!(
            HandlerMapping::substitute("a %%i b", path, None, term_size, None, None).unwrap(),
            "a %i b"
        );
        assert_eq!(
            HandlerMapping::substitute("a %%c b", path, None, term_size, None, None).unwrap(),
            "a %c b"
        );
        assert_eq!(
            HandlerMapping::substitute("a %%l b", path, None, term_size, None, None).unwrap(),
            "a %l b"
        );
        assert_eq!(
            HandlerMapping::substitute("a %%m b", path, Some("text/plain"), term_size, None, None)
                .unwrap(),
            "a %m b"
        );
    }

    #[test]
    fn substitute_multiple_patterns() {
        let term_size = (100, 50);
        let path = Path::new("/tmp/file.txt");

        assert_eq!(
            HandlerMapping::substitute(
                "bat -n --terminal-width %c -r :%l %i",
                path,
                None,
                term_size,
                None,
                None
            )
            .unwrap(),
            "bat -n --terminal-width 100 -r :50 /tmp/file.txt"
        );
    }

    #[test]
    fn substitute_multiple_same_pattern() {
        let term_size = (80, 24);
        let path = Path::new("/tmp/a.txt");

        assert_eq!(
            HandlerMapping::substitute("echo %i %i", path, None, term_size, None, None).unwrap(),
            "echo /tmp/a.txt /tmp/a.txt"
        );
    }

    #[test]
    fn substitute_path_with_spaces() {
        let term_size = (80, 24);
        let path = Path::new("/tmp/my file.txt");

        let result =
            HandlerMapping::substitute("cat %i", path, None, term_size, None, None).unwrap();
        assert_eq!(result, "cat '/tmp/my file.txt'");
    }

    #[test]
    fn substitute_tmp_file() {
        let term_size = (80, 24);
        let path = Path::new("/tmp/test.txt");
        let tmp = tempfile::NamedTempFile::new().unwrap();

        let result =
            HandlerMapping::substitute("cp %i %t", path, None, term_size, Some(&tmp), None)
                .unwrap();
        assert!(result.starts_with("cp /tmp/test.txt "));
        assert!(result.contains(tmp.path().to_str().unwrap()));
    }

    #[test]
    fn substitute_tmp_dir() {
        let term_size = (80, 24);
        let path = Path::new("/tmp/test.txt");
        let tmp_dir = tempfile::tempdir().unwrap();

        let result =
            HandlerMapping::substitute("cp %i %T", path, None, term_size, None, Some(&tmp_dir))
                .unwrap();
        assert!(result.starts_with("cp /tmp/test.txt "));
        assert!(result.contains(tmp_dir.path().to_str().unwrap()));
    }

    #[test]
    fn substitute_no_patterns() {
        let term_size = (80, 24);
        let path = Path::new("");

        assert_eq!(
            HandlerMapping::substitute("plain command here", path, None, term_size, None, None)
                .unwrap(),
            "plain command here"
        );
    }

    #[test]
    fn substitute_trims_whitespace() {
        let term_size = (80, 24);
        let path = Path::new("");

        assert_eq!(
            HandlerMapping::substitute("  cmd  ", path, None, term_size, None, None).unwrap(),
            "cmd"
        );
    }

    #[test]
    fn count_pattern_no_match() {
        assert_eq!(HandlerMapping::count_pattern("hello world", 'i'), 0);
        assert_eq!(HandlerMapping::count_pattern("hello world", 'm'), 0);
        assert_eq!(HandlerMapping::count_pattern("hello world", 'c'), 0);
        assert_eq!(HandlerMapping::count_pattern("hello world", 'l'), 0);
    }

    #[test]
    fn count_pattern_escaped() {
        assert_eq!(HandlerMapping::count_pattern("a %%i b", 'i'), 0);
        assert_eq!(HandlerMapping::count_pattern("a %%m b", 'm'), 0);
    }

    #[test]
    fn count_pattern_mixed() {
        assert_eq!(HandlerMapping::count_pattern("a %i b %%i c %i", 'i'), 2);
    }

    #[test]
    fn has_pattern_filter() {
        let filter = FileFilter {
            command: "gzip -dc %i".to_owned(),
            shell: false,
            no_pipe: false,
            stdin_arg: None,
        };
        let processor = FileProcessor::Filter(filter);
        assert!(processor.has_pattern('i'));
        assert!(!processor.has_pattern('m'));
    }

    #[test]
    fn build_cmd_no_shell() {
        let cmd = HandlerMapping::build_cmd("cat /tmp/test.txt", false).unwrap();
        assert_eq!(cmd, vec!["cat", "/tmp/test.txt"]);
    }

    #[test]
    fn build_cmd_shell() {
        let cmd = HandlerMapping::build_cmd("echo hello | grep h", true).unwrap();
        assert_eq!(cmd, vec!["sh", "-c", "echo hello | grep h"]);
    }

    #[test]
    fn build_cmd_quoted() {
        let cmd = HandlerMapping::build_cmd("cat '/tmp/my file.txt'", false).unwrap();
        assert_eq!(cmd, vec!["cat", "/tmp/my file.txt"]);
    }

    #[test]
    fn build_cmd_invalid() {
        let result = HandlerMapping::build_cmd("cat 'unclosed", false);
        assert!(result.is_err());
    }

    fn default_handler(command: &str) -> FileHandler {
        FileHandler {
            command: command.to_owned(),
            wait: true,
            shell: false,
            no_pipe: false,
            stdin_arg: None,
        }
    }

    fn minimal_config() -> config::Config {
        config::Config {
            filetype: HashMap::new(),
            handler_preview: HashMap::new(),
            default_handler_preview: default_handler("file %i"),
            handler_open: HashMap::new(),
            default_handler_open: default_handler("cat %i"),
            handler_edit: HashMap::new(),
            filter: HashMap::new(),
            handler_scheme: HashMap::new(),
        }
    }

    #[test]
    fn handler_mapping_new_minimal() {
        let config = minimal_config();
        let mapping = HandlerMapping::new(&config);
        assert!(mapping.is_ok());
    }

    #[test]
    fn handler_mapping_new_with_filetype_and_handler() {
        let mut config = minimal_config();
        config.filetype.insert(
            "text".to_owned(),
            config::Filetype {
                extensions: vec!["txt".to_owned()],
                mimes: vec!["text/plain".to_owned()],
            },
        );
        config
            .handler_preview
            .insert("text".to_owned(), default_handler("head %i"));
        let mapping = HandlerMapping::new(&config);
        assert!(mapping.is_ok());
    }

    #[test]
    fn handler_mapping_unbound_filetype() {
        let mut config = minimal_config();
        config.filetype.insert(
            "orphan".to_owned(),
            config::Filetype {
                extensions: vec![],
                mimes: vec!["text/plain".to_owned()],
            },
        );
        let err = HandlerMapping::new(&config).unwrap_err();
        assert!(err.to_string().contains("orphan"));
        assert!(err.to_string().contains("not bound"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn handler_mapping_no_pipe_and_no_wait_ok_on_linux() {
        let mut config = minimal_config();
        config.filetype.insert(
            "text".to_owned(),
            config::Filetype {
                extensions: vec![],
                mimes: vec!["text".to_owned()],
            },
        );
        config.handler_preview.insert(
            "text".to_owned(),
            FileHandler {
                command: "cat %i".to_owned(),
                wait: false,
                shell: false,
                no_pipe: true,
                stdin_arg: None,
            },
        );
        assert!(HandlerMapping::new(&config).is_ok());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn handler_mapping_no_pipe_no_wait_with_tmp_rejected() {
        let mut config = minimal_config();
        config.filetype.insert(
            "text".to_owned(),
            config::Filetype {
                extensions: vec![],
                mimes: vec!["text".to_owned()],
            },
        );
        config.handler_preview.insert(
            "text".to_owned(),
            FileHandler {
                command: "do %i %t".to_owned(),
                wait: false,
                shell: false,
                no_pipe: true,
                stdin_arg: None,
            },
        );
        let err = HandlerMapping::new(&config).unwrap_err();
        assert!(err.to_string().contains("%t"));
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn handler_mapping_no_pipe_and_no_wait() {
        let mut config = minimal_config();
        config.filetype.insert(
            "text".to_owned(),
            config::Filetype {
                extensions: vec![],
                mimes: vec!["text".to_owned()],
            },
        );
        config.handler_preview.insert(
            "text".to_owned(),
            FileHandler {
                command: "cat %i".to_owned(),
                wait: false,
                shell: false,
                no_pipe: true,
                stdin_arg: None,
            },
        );
        let err = HandlerMapping::new(&config).unwrap_err();
        assert!(err.to_string().contains("no_pipe"));
        assert!(err.to_string().contains("wait"));
    }

    #[test]
    fn handler_mapping_multiple_i_no_pipe_false() {
        let mut config = minimal_config();
        config.filetype.insert(
            "text".to_owned(),
            config::Filetype {
                extensions: vec![],
                mimes: vec!["text".to_owned()],
            },
        );
        config.handler_preview.insert(
            "text".to_owned(),
            FileHandler {
                command: "diff %i %i".to_owned(),
                wait: true,
                shell: false,
                no_pipe: false,
                stdin_arg: None,
            },
        );
        let err = HandlerMapping::new(&config).unwrap_err();
        assert!(err.to_string().contains("no_pipe"));
        assert!(err.to_string().contains("%i"));
    }

    #[test]
    fn handler_mapping_multiple_i_no_pipe_true() {
        let mut config = minimal_config();
        config.filetype.insert(
            "text".to_owned(),
            config::Filetype {
                extensions: vec![],
                mimes: vec!["text".to_owned()],
            },
        );
        config.handler_preview.insert(
            "text".to_owned(),
            FileHandler {
                command: "diff %i %i".to_owned(),
                wait: true,
                shell: false,
                no_pipe: true,
                stdin_arg: None,
            },
        );
        assert!(HandlerMapping::new(&config).is_ok());
    }

    #[test]
    fn handler_mapping_filter_multiple_i_no_pipe_false() {
        let mut config = minimal_config();
        config.filetype.insert(
            "gz".to_owned(),
            config::Filetype {
                extensions: vec![],
                mimes: vec!["application/gzip".to_owned()],
            },
        );
        config.filter.insert(
            "gz".to_owned(),
            FileFilter {
                command: "zcat %i %i".to_owned(),
                shell: false,
                no_pipe: false,
                stdin_arg: None,
            },
        );
        let err = HandlerMapping::new(&config).unwrap_err();
        assert!(err.to_string().contains("no_pipe"));
        assert!(err.to_string().contains("%i"));
    }

    #[test]
    fn handler_mapping_filter_multiple_i_no_pipe_true() {
        let mut config = minimal_config();
        config.filetype.insert(
            "gz".to_owned(),
            config::Filetype {
                extensions: vec![],
                mimes: vec!["application/gzip".to_owned()],
            },
        );
        config.filter.insert(
            "gz".to_owned(),
            FileFilter {
                command: "zcat %i %i".to_owned(),
                shell: false,
                no_pipe: true,
                stdin_arg: None,
            },
        );
        assert!(HandlerMapping::new(&config).is_ok());
    }

    #[test]
    fn pipe_forward_empty_header() {
        let mut src = io::Cursor::new(b"hello world");
        let mut dst = Vec::new();

        let written = HandlerMapping::pipe_forward(&mut src, &mut dst, &[]).unwrap();
        assert_eq!(written, 11);
        assert_eq!(dst, b"hello world");
    }

    #[test]
    fn pipe_forward_with_header() {
        let mut src = io::Cursor::new(b" world");
        let mut dst = Vec::new();

        let written = HandlerMapping::pipe_forward(&mut src, &mut dst, b"hello").unwrap();
        assert_eq!(written, 11);
        assert_eq!(dst, b"hello world");
    }

    #[test]
    fn pipe_forward_empty_src() {
        let mut src = io::Cursor::new(b"");
        let mut dst = Vec::new();

        let written = HandlerMapping::pipe_forward(&mut src, &mut dst, b"header").unwrap();
        assert_eq!(written, 6);
        assert_eq!(dst, b"header");
    }

    #[test]
    fn pipe_to_tmpfile_with_data() {
        let pipe = io::Cursor::new(b"rest of data");
        let tmp = HandlerMapping::pipe_to_tmpfile(b"header-", pipe).unwrap();

        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert_eq!(content, "header-rest of data");
    }

    #[test]
    fn pipe_to_tmpfile_empty() {
        let pipe = io::Cursor::new(b"");
        let tmp = HandlerMapping::pipe_to_tmpfile(b"", pipe).unwrap();

        let content = std::fs::read_to_string(tmp.path()).unwrap();
        assert_eq!(content, "");
    }

    #[test]
    fn file_handlers_new() {
        let default = FileHandler {
            command: "cat %i".to_owned(),
            wait: true,
            shell: false,
            no_pipe: false,
            stdin_arg: None,
        };
        let handlers = FileHandlers::new(&default);
        assert!(handlers.extensions.is_empty());
        assert!(handlers.mimes.is_empty());
        assert_eq!(handlers.default, default);
    }

    #[test]
    fn file_handlers_add() {
        let default = FileHandler {
            command: "cat %i".to_owned(),
            wait: true,
            shell: false,
            no_pipe: false,
            stdin_arg: None,
        };
        let mut handlers = FileHandlers::new(&default);

        let processor = Rc::new(FileProcessor::Handler(FileHandler {
            command: "head %i".to_owned(),
            wait: true,
            shell: false,
            no_pipe: false,
            stdin_arg: None,
        }));

        let filetype = config::Filetype {
            extensions: vec!["txt".to_owned(), "log".to_owned()],
            mimes: vec!["text/plain".to_owned()],
        };
        handlers.add(&processor, &filetype);

        assert!(handlers.extensions.contains_key("txt"));
        assert!(handlers.extensions.contains_key("log"));
        assert!(handlers.mimes.contains_key("text/plain"));
    }

    #[test]
    fn scheme_handlers_new_and_add() {
        let mut scheme_handlers = SchemeHandlers::new();
        assert!(scheme_handlers.schemes.is_empty());

        let handler = SchemeHandler {
            command: "firefox %i".to_owned(),
            shell: false,
        };
        scheme_handlers.add(&handler, "https");
        assert!(scheme_handlers.schemes.contains_key("https"));
        assert_eq!(
            scheme_handlers.schemes.get("https").unwrap().command,
            "firefox %i"
        );
    }

    #[test]
    fn validate_handler_ok() {
        let handler = FileHandler {
            command: "cat %i".to_owned(),
            wait: true,
            shell: false,
            no_pipe: false,
            stdin_arg: None,
        };
        assert!(HandlerMapping::validate_handler(&handler).is_ok());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn validate_handler_no_pipe_no_wait_ok_on_linux() {
        let handler = FileHandler {
            command: "cat %i".to_owned(),
            wait: false,
            shell: false,
            no_pipe: true,
            stdin_arg: None,
        };
        assert!(HandlerMapping::validate_handler(&handler).is_ok());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn validate_handler_no_pipe_no_wait_with_tmp_rejected() {
        let handler = FileHandler {
            command: "do %i %T".to_owned(),
            wait: false,
            shell: false,
            no_pipe: true,
            stdin_arg: None,
        };
        assert!(HandlerMapping::validate_handler(&handler).is_err());
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn validate_handler_no_pipe_no_wait() {
        let handler = FileHandler {
            command: "cat %i".to_owned(),
            wait: false,
            shell: false,
            no_pipe: true,
            stdin_arg: None,
        };
        assert!(HandlerMapping::validate_handler(&handler).is_err());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn pipe_to_memfd_with_data() {
        let pipe = io::Cursor::new(b"rest of data");
        let file = HandlerMapping::pipe_to_memfd(b"header-", pipe).unwrap();
        let path = format!("/proc/self/fd/{}", file.as_raw_fd());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "header-rest of data");
    }

    #[test]
    fn validate_handler_multiple_i_no_pipe_false() {
        let handler = FileHandler {
            command: "diff %i %i".to_owned(),
            wait: true,
            shell: false,
            no_pipe: false,
            stdin_arg: None,
        };
        assert!(HandlerMapping::validate_handler(&handler).is_err());
    }

    #[test]
    fn validate_handler_multiple_i_no_pipe_true() {
        let handler = FileHandler {
            command: "diff %i %i".to_owned(),
            wait: true,
            shell: false,
            no_pipe: true,
            stdin_arg: None,
        };
        assert!(HandlerMapping::validate_handler(&handler).is_ok());
    }

    #[test]
    fn validate_handler_single_i_no_pipe_false() {
        let handler = FileHandler {
            command: "cat %i".to_owned(),
            wait: true,
            shell: false,
            no_pipe: false,
            stdin_arg: None,
        };
        assert!(HandlerMapping::validate_handler(&handler).is_ok());
    }
}
