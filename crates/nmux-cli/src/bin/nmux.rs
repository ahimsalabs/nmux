use std::io::{self, BufRead, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use nmux_cli::local;
use nmux_core::session::AttachMode;
use nmux_proto::protocol;

const STDIN_BYTES_DETACH: u8 = 0x1d;
const REDRAW_TERMINAL_ENTER: &str = "\x1b[?1049h\x1b[?25l";
const REDRAW_TERMINAL_EXIT: &str = "\x1b[?25h\x1b[?1049l";
static SIGWINCH_RECEIVED: AtomicBool = AtomicBool::new(false);

fn main() {
    if let Err(err) = run() {
        eprintln!("nmux: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = args()?;
    if args.help {
        print!("{}", usage());
        return Ok(());
    }

    if args.live {
        return run_live(&args);
    }

    let mut client_state = load_client_state(args.state_path.as_deref())?;

    let iterations = if args.follow {
        args.iterations.unwrap_or(usize::MAX)
    } else {
        1
    };

    for iteration in 0..iterations {
        let rendered = attach_once(&args, &mut client_state)?;
        save_client_state(args.state_path.as_deref(), &client_state)?;
        print_rendered(rendered);
        flush_stdout()?;

        if args.follow && iteration + 1 < iterations {
            thread::sleep(Duration::from_millis(args.interval_ms));
        }
    }

    Ok(())
}

fn run_live(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let _raw_terminal = RawTerminalGuard::enable_if_needed(args.stdin_bytes, args.local_echo)?;
    let _redraw_terminal = RedrawTerminalGuard::enable_if_needed(args.redraw, stdout_is_tty())?;
    warn_if_interim_surface_fidelity_is_visible(args.stdin_bytes);
    let mut sigwinch_resize =
        SigwinchResize::enable_if_needed(args.stdin_bytes, args.live_resize.is_some())?;
    let mut client_state = load_client_state(args.state_path.as_deref())?;
    let mut stream = local::connect_to_daemon(&args.socket_path)?;
    stream.set_read_timeout(Some(Duration::from_millis(args.interval_ms)))?;
    let stdin = io::stdin();
    let mut stdin_lines = if args.stdin_input {
        Some(stdin.lock().lines())
    } else {
        None
    };
    let stdin_bytes = if args.stdin_bytes {
        Some(spawn_stdin_byte_reader())
    } else {
        None
    };
    let mut stdin_bytes_closed = false;
    let mut detach_requested = false;

    let mut options = local::AttachOptions {
        input_text: args.input_text.clone(),
        scrollback_start_line: args.scrollback_start_line,
        scrollback_line_count: args.scrollback_line_count,
        ..local::AttachOptions::default()
    };
    if args.stdin_input || args.stdin_bytes {
        options.request.mode = AttachMode::ReadWrite;
    } else if options.input_text.is_none() {
        options.request.mode = AttachMode::ReadOnly;
    }

    options.request.known_surfaces = client_state.known_surfaces();
    local::write_attach_request(&mut stream, &options.request)?;
    let snapshot = local::attach_from_stream(&mut stream)?;
    let mut rendered = client_state.render_attach(snapshot)?;
    if rendered.surface_text.is_none() {
        rendered.surface_text = client_state.cached_surface_text(&rendered.workspace.pane_id);
    }
    warn_if_resize_intent_conflicts_with_policy(args.live_resize, rendered.workspace.resize_policy);
    let mut current_workspace = rendered.workspace.clone();
    let mut current_surface_text = rendered
        .surface_text
        .clone()
        .unwrap_or_else(|| current_workspace.display_line());
    let scrollback = initial_live_scrollback(args, &mut stream)?;
    print_live_rendered(rendered, args.redraw, scrollback);
    flush_stdout()?;

    let cycle_limit = args.iterations.or_else(|| {
        (!args.stdin_input && !args.stdin_bytes && options.request.mode == AttachMode::ReadWrite)
            .then_some(1)
    });
    let mut cycles = 0;
    loop {
        if cycle_limit.is_some_and(|iterations| cycles >= iterations) {
            break;
        }

        if options.request.mode == AttachMode::ReadWrite {
            if let Some((cols, rows)) = args.live_resize {
                local::send_resize_intent(&mut stream, "pane-1", cols, rows)?;
            } else if let Some((cols, rows)) = sigwinch_resize.next_resize()? {
                local::send_resize_intent(&mut stream, "pane-1", cols, rows)?;
            }
            let input_text = if let Some(receiver) = stdin_bytes.as_ref() {
                match receiver.try_recv() {
                    Ok(StdinByteRead::Input(input)) => {
                        let (input, detach) = split_stdin_bytes_for_detach(&input);
                        if let Some(input) = input {
                            local::send_raw_input(&mut stream, "pane-1", &input)?;
                        }
                        detach_requested = detach;
                        None
                    }
                    Ok(StdinByteRead::Closed) => {
                        stdin_bytes_closed = true;
                        None
                    }
                    Ok(StdinByteRead::Error(err)) => return Err(err.into()),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => {
                        stdin_bytes_closed = true;
                        None
                    }
                }
            } else if args.stdin_input {
                let stdin_line = next_stdin_line(stdin_lines.as_mut())?;
                match stdin_line {
                    Some(line) => Some(line),
                    None if args.iterations.is_none() => {
                        eprintln!("nmux: stdin EOF; detached");
                        break;
                    }
                    None => None,
                }
            } else {
                options.input_text.as_deref().map(ToOwned::to_owned)
            };
            if let Some(input_text) = input_text.as_deref() {
                local::send_key_input(&mut stream, "pane-1", input_text)?;
            }
        }

        loop {
            match local::read_live_surface_update_from_stream(&mut stream)? {
                local::LiveSurfaceRead::Workspace(workspace) => {
                    current_workspace = workspace;
                    if args.redraw {
                        print_live_surface(&current_workspace, &current_surface_text, args.redraw);
                    } else {
                        println!("{}", current_workspace.display_line());
                    }
                    flush_stdout()?;
                }
                local::LiveSurfaceRead::Update(update) => {
                    current_surface_text = client_state.render_surface_update(&update)?;
                    print_live_surface(&current_workspace, &current_surface_text, args.redraw);
                    flush_stdout()?;
                }
                local::LiveSurfaceRead::NoFrame => break,
                local::LiveSurfaceRead::Closed => {
                    eprintln!("nmux: live server closed connection");
                    return save_live_state(args, &client_state);
                }
            }
        }
        if detach_requested {
            eprintln!("nmux: detached by local Ctrl-]");
            break;
        }
        if stdin_bytes_closed && args.iterations.is_none() {
            eprintln!("nmux: stdin EOF; detached");
            break;
        }
        cycles += 1;
    }

    save_live_state(args, &client_state)
}

fn flush_stdout() -> io::Result<()> {
    io::stdout().flush()
}

fn warn_if_resize_intent_conflicts_with_policy(
    live_resize: Option<(u32, u32)>,
    resize_policy: protocol::ResizePolicy,
) {
    if let Some(message) = resize_policy_warning(live_resize, resize_policy) {
        eprintln!("{message}");
    }
}

fn warn_if_interim_surface_fidelity_is_visible(stdin_bytes: bool) {
    if interim_surface_fidelity_warning_needed(stdin_bytes, stdin_is_tty(), stdout_is_tty()) {
        eprintln!("{}", INTERIM_SURFACE_FIDELITY_WARNING);
    }
}

const INTERIM_SURFACE_FIDELITY_WARNING: &str = "nmux: interim text surface; ANSI styles, alternate screen, cursor motion, images, and full VT fidelity are unsupported";

fn interim_surface_fidelity_warning_needed(
    stdin_bytes: bool,
    stdin_is_tty: bool,
    stdout_is_tty: bool,
) -> bool {
    stdin_bytes && stdin_is_tty && stdout_is_tty
}

fn resize_policy_warning(
    live_resize: Option<(u32, u32)>,
    resize_policy: protocol::ResizePolicy,
) -> Option<&'static str> {
    (live_resize.is_some() && resize_policy == protocol::ResizePolicy::Manual)
        .then_some("nmux: resize request ignored by manual resize policy")
}

fn save_live_state(
    args: &Args,
    client_state: &local::ClientAttachState,
) -> Result<(), Box<dyn std::error::Error>> {
    save_client_state(args.state_path.as_deref(), client_state)
}

fn load_client_state(
    path: Option<&Path>,
) -> Result<local::ClientAttachState, Box<dyn std::error::Error>> {
    let Some(path) = path else {
        return Ok(local::ClientAttachState::default());
    };
    local::ClientAttachState::load(path)
        .map_err(|err| format!("failed to load client state {}: {err}", path.display()).into())
}

fn save_client_state(
    path: Option<&Path>,
    client_state: &local::ClientAttachState,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = path else {
        return Ok(());
    };
    client_state
        .save(path)
        .map_err(|err| format!("failed to save client state {}: {err}", path.display()).into())
}

fn initial_live_scrollback(
    args: &Args,
    stream: &mut UnixStream,
) -> Result<Option<local::ScrollbackChunkSummary>, Box<dyn std::error::Error>> {
    local::send_scrollback_fetch(
        stream,
        "pane-1",
        args.scrollback_start_line,
        args.scrollback_line_count,
    )?;
    Ok(Some(local::read_scrollback_chunk_from_stream(stream)?))
}

fn next_stdin_line(
    stdin_lines: Option<&mut io::Lines<io::StdinLock<'_>>>,
) -> io::Result<Option<String>> {
    let Some(stdin_lines) = stdin_lines else {
        return Ok(None);
    };
    let Some(line) = stdin_lines.next() else {
        return Ok(None);
    };
    let mut line = line?;
    line.push('\n');
    Ok(Some(line))
}

fn spawn_stdin_byte_reader() -> mpsc::Receiver<StdinByteRead> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buffer = [0_u8; 1024];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) => {
                    let _ = tx.send(StdinByteRead::Closed);
                    break;
                }
                Ok(count) => {
                    let input = buffer[..count].to_vec();
                    if tx.send(StdinByteRead::Input(input)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(StdinByteRead::Error(err.to_string()));
                    break;
                }
            }
        }
    });
    rx
}

enum StdinByteRead {
    Input(Vec<u8>),
    Closed,
    Error(String),
}

fn split_stdin_bytes_for_detach(input: &[u8]) -> (Option<Vec<u8>>, bool) {
    let Some(index) = input.iter().position(|byte| *byte == STDIN_BYTES_DETACH) else {
        return (Some(input.to_vec()), false);
    };

    let before_detach = &input[..index];
    if before_detach.is_empty() {
        (None, true)
    } else {
        (Some(before_detach.to_vec()), true)
    }
}

struct RawTerminalGuard {
    original: libc::termios,
}

impl RawTerminalGuard {
    fn enable_if_needed(stdin_bytes: bool, local_echo: LocalEcho) -> io::Result<Option<Self>> {
        if !raw_terminal_mode_needed(stdin_bytes, stdin_is_tty()) {
            return Ok(None);
        }

        let mut original = empty_termios();
        // Safety: STDIN_FILENO is a valid process file descriptor when isatty
        // succeeded, and original points to valid writable storage.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }

        let mut raw = original;
        raw.c_lflag = raw_terminal_lflag(raw.c_lflag, local_echo);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;

        // Safety: raw was derived from a valid termios fetched from stdin.
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Some(Self { original }))
    }
}

impl Drop for RawTerminalGuard {
    fn drop(&mut self) {
        // Safety: original was captured from STDIN_FILENO by tcgetattr. Drop
        // must not panic, so restoration errors are intentionally ignored.
        let _ = unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original) };
    }
}

fn raw_terminal_mode_needed(stdin_bytes: bool, stdin_is_tty: bool) -> bool {
    stdin_bytes && stdin_is_tty
}

struct RedrawTerminalGuard;

impl RedrawTerminalGuard {
    fn enable_if_needed(redraw: bool, stdout_is_tty: bool) -> io::Result<Option<Self>> {
        if !redraw_terminal_guard_needed(redraw, stdout_is_tty) {
            return Ok(None);
        }

        print!("{REDRAW_TERMINAL_ENTER}");
        flush_stdout()?;
        Ok(Some(Self))
    }
}

impl Drop for RedrawTerminalGuard {
    fn drop(&mut self) {
        print!("{REDRAW_TERMINAL_EXIT}");
        let _ = flush_stdout();
    }
}

fn redraw_terminal_guard_needed(redraw: bool, stdout_is_tty: bool) -> bool {
    redraw && stdout_is_tty
}

struct SigwinchResize {
    _guard: Option<SigwinchGuard>,
    last_size: Option<(u32, u32)>,
}

impl SigwinchResize {
    fn enable_if_needed(stdin_bytes: bool, explicit_resize: bool) -> io::Result<Self> {
        if !sigwinch_resize_needed(stdin_bytes, explicit_resize, stdin_is_tty()) {
            return Ok(Self {
                _guard: None,
                last_size: None,
            });
        }

        let guard = SigwinchGuard::install()?;
        SIGWINCH_RECEIVED.store(true, Ordering::SeqCst);
        Ok(Self {
            _guard: Some(guard),
            last_size: None,
        })
    }

    fn next_resize(&mut self) -> io::Result<Option<(u32, u32)>> {
        if self._guard.is_none() || !SIGWINCH_RECEIVED.swap(false, Ordering::SeqCst) {
            return Ok(None);
        }

        let Some(size) = stdin_terminal_size()? else {
            return Ok(None);
        };
        if self.last_size == Some(size) {
            return Ok(None);
        }

        self.last_size = Some(size);
        Ok(Some(size))
    }
}

struct SigwinchGuard {
    previous: libc::sighandler_t,
}

impl SigwinchGuard {
    fn install() -> io::Result<Self> {
        // Safety: installing a process signal handler is inherently global.
        // The handler only stores to an AtomicBool, which is signal-safe.
        let handler = handle_sigwinch as *const () as libc::sighandler_t;
        let previous = unsafe { libc::signal(libc::SIGWINCH, handler) };
        if previous == libc::SIG_ERR {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { previous })
    }
}

impl Drop for SigwinchGuard {
    fn drop(&mut self) {
        // Safety: previous was returned by signal during install. Drop must not
        // panic, so restoration errors are intentionally ignored.
        let _ = unsafe { libc::signal(libc::SIGWINCH, self.previous) };
    }
}

extern "C" fn handle_sigwinch(_: libc::c_int) {
    SIGWINCH_RECEIVED.store(true, Ordering::SeqCst);
}

fn sigwinch_resize_needed(stdin_bytes: bool, explicit_resize: bool, stdin_is_tty: bool) -> bool {
    stdin_bytes && !explicit_resize && stdin_is_tty
}

fn stdin_terminal_size() -> io::Result<Option<(u32, u32)>> {
    let mut size = empty_winsize();
    // Safety: STDIN_FILENO is a process file descriptor and size points to
    // valid writable storage for TIOCGWINSZ.
    if unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut size) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(terminal_size_from_winsize(size))
}

fn terminal_size_from_winsize(size: libc::winsize) -> Option<(u32, u32)> {
    if size.ws_col == 0 || size.ws_row == 0 {
        return None;
    }
    Some((u32::from(size.ws_col), u32::from(size.ws_row)))
}

fn raw_terminal_lflag(flags: libc::tcflag_t, local_echo: LocalEcho) -> libc::tcflag_t {
    let flags = flags & !libc::ICANON;
    match local_echo {
        LocalEcho::Off => flags & !libc::ECHO,
        LocalEcho::Tty => flags,
    }
}

fn stdin_is_tty() -> bool {
    // Safety: isatty only inspects the file descriptor.
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

fn stdout_is_tty() -> bool {
    // Safety: isatty only inspects the file descriptor.
    unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 }
}

fn empty_termios() -> libc::termios {
    // Safety: termios is a plain C struct that is immediately initialized by
    // tcgetattr before use.
    unsafe { std::mem::zeroed() }
}

fn empty_winsize() -> libc::winsize {
    // Safety: winsize is a plain C struct that is immediately initialized by
    // ioctl before use.
    unsafe { std::mem::zeroed() }
}

fn attach_once(
    args: &Args,
    client_state: &mut local::ClientAttachState,
) -> Result<local::RenderedAttach, Box<dyn std::error::Error>> {
    let mut options = local::AttachOptions {
        input_text: args.input_text.clone(),
        scrollback_start_line: args.scrollback_start_line,
        scrollback_line_count: args.scrollback_line_count,
        ..local::AttachOptions::default()
    };
    if args.follow || options.input_text.is_none() {
        options.request.mode = AttachMode::ReadOnly;
        options.input_text = None;
    }

    local::attach_render_once(&args.socket_path, options, client_state)
}

fn print_rendered(rendered: local::RenderedAttach) {
    println!("{}", rendered.workspace.display_line());
    if let Some(surface_text) = rendered.surface_text {
        println!("{surface_text}");
    }
    if let Some(scrollback) = rendered.scrollback {
        print_scrollback(scrollback);
    }
}

fn print_live_rendered(
    rendered: local::RenderedAttach,
    redraw: bool,
    initial_scrollback: Option<local::ScrollbackChunkSummary>,
) {
    if redraw {
        let surface_text = rendered
            .surface_text
            .unwrap_or_else(|| rendered.workspace.display_line());
        let redraw_text =
            redraw_text_with_context(&rendered.workspace, &surface_text, initial_scrollback);
        redraw_terminal(&redraw_text);
        return;
    }

    print_rendered(rendered);
    if let Some(scrollback) = initial_scrollback {
        print_scrollback(scrollback);
    }
}

fn print_live_surface(workspace: &local::WorkspaceSummary, surface_text: &str, redraw: bool) {
    if redraw {
        redraw_terminal(&redraw_text_with_context(workspace, surface_text, None));
    } else {
        println!("{surface_text}");
    }
}

fn redraw_terminal(surface_text: &str) {
    print!("\x1b[2J\x1b[H{surface_text}");
    if !surface_text.ends_with('\n') {
        println!();
    }
}

fn print_scrollback(scrollback: local::ScrollbackChunkSummary) {
    print!("{}", format_scrollback(&scrollback));
}

fn redraw_text_with_context(
    workspace: &local::WorkspaceSummary,
    surface_text: &str,
    scrollback: Option<local::ScrollbackChunkSummary>,
) -> String {
    let mut text = workspace.display_line();
    text.push('\n');
    if let Some(scrollback) = scrollback {
        text.push_str(&format_scrollback(&scrollback));
    }
    text.push_str(surface_text);
    text
}

fn format_scrollback(scrollback: &local::ScrollbackChunkSummary) -> String {
    let mut text = format!(
        "scrollback {}..{}:",
        scrollback.start_line, scrollback.total_lines
    );
    text.push('\n');
    for line in &scrollback.lines {
        text.push_str(&line.text);
        text.push('\n');
    }
    text
}

struct Args {
    help: bool,
    socket_path: PathBuf,
    input_text: Option<String>,
    scrollback_start_line: u64,
    scrollback_line_count: u32,
    state_path: Option<PathBuf>,
    follow: bool,
    live: bool,
    stdin_input: bool,
    stdin_bytes: bool,
    local_echo: LocalEcho,
    redraw: bool,
    live_resize: Option<(u32, u32)>,
    interval_ms: u64,
    iterations: Option<usize>,
}

fn args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut help = false;
    let mut socket_path = local::default_socket_path();
    let mut input_text = Some("a".to_owned());
    let mut scrollback_start_line = 1;
    let mut scrollback_line_count = 2;
    let mut state_path = None;
    let mut follow = false;
    let mut live = false;
    let mut stdin_input = false;
    let mut stdin_bytes = false;
    let mut local_echo = LocalEcho::Off;
    let mut redraw = false;
    let mut live_cols = None;
    let mut live_rows = None;
    let mut interval_ms = 1000;
    let mut iterations = None;
    let mut local_echo_set = false;
    let mut key_set = false;
    let mut no_input_set = false;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                help = true;
            }
            "--socket" => {
                socket_path = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--socket requires a path")?;
            }
            "--key" => {
                key_set = true;
                input_text = Some(args.next().ok_or("--key requires text")?);
            }
            "--no-input" => {
                no_input_set = true;
                input_text = None;
            }
            "--scrollback-start" => {
                scrollback_start_line = args
                    .next()
                    .ok_or("--scrollback-start requires a line")?
                    .parse()?;
            }
            "--scrollback-count" => {
                scrollback_line_count = args
                    .next()
                    .ok_or("--scrollback-count requires a count")?
                    .parse()?;
            }
            "--state" => {
                state_path = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or("--state requires a path")?,
                );
            }
            "--follow" => {
                follow = true;
                input_text = None;
            }
            "--live" => {
                live = true;
            }
            "--stdin" => {
                stdin_input = true;
            }
            "--stdin-bytes" => {
                stdin_bytes = true;
            }
            "--local-echo" => {
                local_echo_set = true;
                local_echo =
                    parse_local_echo(&args.next().ok_or("--local-echo requires off or tty")?)
                        .map_err(|err| format!("--local-echo {err}"))?;
            }
            "--redraw" => {
                redraw = true;
            }
            "--cols" => {
                live_cols = Some(args.next().ok_or("--cols requires a count")?.parse()?);
            }
            "--rows" => {
                live_rows = Some(args.next().ok_or("--rows requires a count")?.parse()?);
            }
            "--interval-ms" => {
                interval_ms = args
                    .next()
                    .ok_or("--interval-ms requires milliseconds")?
                    .parse()?;
            }
            "--iterations" => {
                iterations = Some(
                    args.next()
                        .ok_or("--iterations requires a count")?
                        .parse()?,
                );
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    let live_resize = match (live_cols, live_rows) {
        (Some(cols), Some(rows)) => Some((cols, rows)),
        (None, None) => None,
        _ => return Err("--cols and --rows must be provided together".into()),
    };
    if stdin_input && stdin_bytes {
        return Err("--stdin and --stdin-bytes cannot be used together".into());
    }
    validate_positive_numeric_args(
        scrollback_start_line,
        scrollback_line_count,
        live_resize,
        interval_ms,
    )?;
    validate_explicit_input_modes(key_set, no_input_set, stdin_input, stdin_bytes)?;
    validate_mode_args(
        live,
        follow,
        stdin_input,
        stdin_bytes,
        local_echo_set,
        redraw,
        live_resize,
        iterations,
    )?;

    Ok(Args {
        help,
        socket_path,
        input_text,
        scrollback_start_line,
        scrollback_line_count,
        state_path,
        follow,
        live,
        stdin_input,
        stdin_bytes,
        local_echo,
        redraw,
        live_resize,
        interval_ms,
        iterations,
    })
}

fn validate_positive_numeric_args(
    scrollback_start_line: u64,
    scrollback_line_count: u32,
    live_resize: Option<(u32, u32)>,
    interval_ms: u64,
) -> Result<(), &'static str> {
    if scrollback_start_line == 0 {
        return Err("--scrollback-start must be greater than 0");
    }
    if scrollback_line_count == 0 {
        return Err("--scrollback-count must be greater than 0");
    }
    if interval_ms == 0 {
        return Err("--interval-ms must be greater than 0");
    }
    if live_resize.is_some_and(|(cols, rows)| {
        cols == 0 || rows == 0 || cols > u16::MAX as u32 || rows > u16::MAX as u32
    }) {
        return Err("--cols and --rows must be between 1 and 65535");
    }
    Ok(())
}

fn validate_explicit_input_modes(
    key_set: bool,
    no_input_set: bool,
    stdin_input: bool,
    stdin_bytes: bool,
) -> Result<(), &'static str> {
    if key_set && no_input_set {
        return Err("--key cannot be combined with --no-input");
    }
    if key_set && stdin_input {
        return Err("--key cannot be combined with --stdin");
    }
    if key_set && stdin_bytes {
        return Err("--key cannot be combined with --stdin-bytes");
    }
    if no_input_set && stdin_input {
        return Err("--no-input cannot be combined with --stdin");
    }
    if no_input_set && stdin_bytes {
        return Err("--no-input cannot be combined with --stdin-bytes");
    }
    Ok(())
}

fn validate_mode_args(
    live: bool,
    follow: bool,
    stdin_input: bool,
    stdin_bytes: bool,
    local_echo_set: bool,
    redraw: bool,
    live_resize: Option<(u32, u32)>,
    iterations: Option<usize>,
) -> Result<(), &'static str> {
    if live && follow {
        return Err("--follow cannot be combined with --live");
    }
    if stdin_input && !live {
        return Err("--stdin requires --live");
    }
    if stdin_bytes && !live {
        return Err("--stdin-bytes requires --live");
    }
    if local_echo_set && !stdin_bytes {
        return Err("--local-echo requires --stdin-bytes");
    }
    if redraw && !live {
        return Err("--redraw requires --live");
    }
    if live_resize.is_some() && !live {
        return Err("--cols and --rows require --live");
    }
    if iterations.is_some() && !live && !follow {
        return Err("--iterations requires --live or --follow");
    }
    if iterations == Some(0) {
        return Err("--iterations must be greater than 0");
    }
    Ok(())
}

fn usage() -> &'static str {
    "\
nmux - attach to an nmux daemon over a local Unix socket

Usage:
  nmux [OPTIONS]

Options:
  --socket PATH              Unix socket path
  --key TEXT                 Text input to send for read-write attach
  --no-input                 Attach read-only
  --scrollback-start LINE    First scrollback line to request
  --scrollback-count COUNT   Number of scrollback lines to request
  --state PATH               Persist client-side pane surface cache
  --follow                   Reconnect in a polling loop
  --live                     Keep one attach connection open
  --stdin                    Stream newline-delimited stdin in live mode
  --stdin-bytes              Stream raw stdin chunks in live mode
  --local-echo off|tty       Local TTY echo policy for --stdin-bytes
  --redraw                   Repaint the current live surface in place
  --cols COUNT               Desired live pane columns
  --rows COUNT               Desired live pane rows
  --interval-ms MS           Poll/read timeout in milliseconds
  --iterations COUNT         Bounded follow/live cycle count
  -h, --help                 Show this help

Notes:
  Default socket: valid absolute $XDG_RUNTIME_DIR/nmux/nmuxd.sock, else /tmp/nmux-$UID/nmuxd.sock.
  The current renderer uses an interim text surface, not a VT-correct terminal emulator.

Examples:
  nmux --no-input
  nmux --live --iterations 2 --key 'ping\n'
  nmux --live --stdin-bytes --redraw
"
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalEcho {
    Off,
    Tty,
}

fn parse_local_echo(value: &str) -> Result<LocalEcho, &'static str> {
    match value {
        "off" => Ok(LocalEcho::Off),
        "tty" => Ok(LocalEcho::Tty),
        _ => Err("requires off or tty"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LocalEcho, interim_surface_fidelity_warning_needed, parse_local_echo, raw_terminal_lflag,
        raw_terminal_mode_needed, redraw_terminal_guard_needed, resize_policy_warning,
        sigwinch_resize_needed, split_stdin_bytes_for_detach, terminal_size_from_winsize, usage,
        validate_explicit_input_modes, validate_mode_args, validate_positive_numeric_args,
    };

    #[test]
    fn raw_terminal_mode_is_only_needed_for_stdin_bytes_on_tty() {
        assert!(raw_terminal_mode_needed(true, true));
        assert!(!raw_terminal_mode_needed(true, false));
        assert!(!raw_terminal_mode_needed(false, true));
        assert!(!raw_terminal_mode_needed(false, false));
    }

    #[test]
    fn redraw_terminal_guard_is_only_needed_for_redraw_on_tty() {
        assert!(redraw_terminal_guard_needed(true, true));
        assert!(!redraw_terminal_guard_needed(true, false));
        assert!(!redraw_terminal_guard_needed(false, true));
        assert!(!redraw_terminal_guard_needed(false, false));
    }

    #[test]
    fn interim_surface_fidelity_warning_is_only_for_interactive_byte_mode() {
        assert!(interim_surface_fidelity_warning_needed(true, true, true));
        assert!(!interim_surface_fidelity_warning_needed(true, true, false));
        assert!(!interim_surface_fidelity_warning_needed(true, false, true));
        assert!(!interim_surface_fidelity_warning_needed(false, true, true));
    }

    #[test]
    fn sigwinch_resize_is_only_needed_for_interactive_byte_mode_without_explicit_size() {
        assert!(sigwinch_resize_needed(true, false, true));
        assert!(!sigwinch_resize_needed(true, true, true));
        assert!(!sigwinch_resize_needed(true, false, false));
        assert!(!sigwinch_resize_needed(false, false, true));
    }

    #[test]
    fn terminal_size_from_winsize_rejects_zero_dimensions() {
        let mut size = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        assert_eq!(terminal_size_from_winsize(size), Some((80, 24)));

        size.ws_col = 0;
        assert_eq!(terminal_size_from_winsize(size), None);
        size.ws_col = 80;
        size.ws_row = 0;
        assert_eq!(terminal_size_from_winsize(size), None);
    }

    #[test]
    fn raw_terminal_echo_choice_controls_echo_flag() {
        let flags = raw_terminal_lflag(libc::ICANON | libc::ECHO, LocalEcho::Off);
        assert_eq!(flags & libc::ICANON, 0);
        assert_eq!(flags & libc::ECHO, 0);

        let flags = raw_terminal_lflag(libc::ICANON | libc::ECHO, LocalEcho::Tty);
        assert_eq!(flags & libc::ICANON, 0);
        assert_eq!(flags & libc::ECHO, libc::ECHO);

        let flags = raw_terminal_lflag(libc::ICANON, LocalEcho::Tty);
        assert_eq!(flags & libc::ICANON, 0);
        assert_eq!(flags & libc::ECHO, 0);
    }

    #[test]
    fn local_echo_arg_accepts_explicit_choices() {
        assert_eq!(parse_local_echo("off"), Ok(LocalEcho::Off));
        assert_eq!(parse_local_echo("tty"), Ok(LocalEcho::Tty));
        assert!(parse_local_echo("auto").is_err());
    }

    #[test]
    fn mode_validation_rejects_ignored_or_conflicting_flags() {
        assert_eq!(
            validate_mode_args(true, true, false, false, false, false, None, None),
            Err("--follow cannot be combined with --live")
        );
        assert_eq!(
            validate_mode_args(false, false, true, false, false, false, None, None),
            Err("--stdin requires --live")
        );
        assert_eq!(
            validate_mode_args(false, false, false, true, false, false, None, None),
            Err("--stdin-bytes requires --live")
        );
        assert_eq!(
            validate_mode_args(true, false, false, false, true, false, None, None),
            Err("--local-echo requires --stdin-bytes")
        );
        assert_eq!(
            validate_mode_args(false, false, false, false, false, true, None, None),
            Err("--redraw requires --live")
        );
        assert_eq!(
            validate_mode_args(
                false,
                false,
                false,
                false,
                false,
                false,
                Some((80, 24)),
                None
            ),
            Err("--cols and --rows require --live")
        );
        assert_eq!(
            validate_mode_args(false, false, false, false, false, false, None, Some(1)),
            Err("--iterations requires --live or --follow")
        );
        assert_eq!(
            validate_mode_args(true, false, false, false, false, false, None, Some(0)),
            Err("--iterations must be greater than 0")
        );
        assert!(
            validate_mode_args(
                true,
                false,
                false,
                true,
                true,
                true,
                Some((80, 24)),
                Some(1)
            )
            .is_ok()
        );
        assert!(validate_mode_args(false, true, false, false, false, false, None, Some(1)).is_ok());
    }

    #[test]
    fn input_mode_validation_rejects_explicit_conflicts() {
        assert_eq!(
            validate_explicit_input_modes(true, true, false, false),
            Err("--key cannot be combined with --no-input")
        );
        assert_eq!(
            validate_explicit_input_modes(true, false, true, false),
            Err("--key cannot be combined with --stdin")
        );
        assert_eq!(
            validate_explicit_input_modes(true, false, false, true),
            Err("--key cannot be combined with --stdin-bytes")
        );
        assert_eq!(
            validate_explicit_input_modes(false, true, true, false),
            Err("--no-input cannot be combined with --stdin")
        );
        assert_eq!(
            validate_explicit_input_modes(false, true, false, true),
            Err("--no-input cannot be combined with --stdin-bytes")
        );
        assert!(validate_explicit_input_modes(false, false, true, false).is_ok());
        assert!(validate_explicit_input_modes(false, false, false, true).is_ok());
        assert!(validate_explicit_input_modes(true, false, false, false).is_ok());
        assert!(validate_explicit_input_modes(false, true, false, false).is_ok());
    }

    #[test]
    fn numeric_validation_rejects_zero_live_loop_values() {
        assert_eq!(
            validate_positive_numeric_args(1, 2, None, 0),
            Err("--interval-ms must be greater than 0")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 2, Some((0, 24)), 1000),
            Err("--cols and --rows must be between 1 and 65535")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 2, Some((80, 0)), 1000),
            Err("--cols and --rows must be between 1 and 65535")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 2, Some((65536, 24)), 1000),
            Err("--cols and --rows must be between 1 and 65535")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 2, Some((80, 65536)), 1000),
            Err("--cols and --rows must be between 1 and 65535")
        );
        assert!(validate_positive_numeric_args(1, 2, None, 1000).is_ok());
        assert!(validate_positive_numeric_args(1, 2, Some((65535, 65535)), 1000).is_ok());
    }

    #[test]
    fn numeric_validation_rejects_zero_scrollback_values() {
        assert_eq!(
            validate_positive_numeric_args(0, 2, None, 1000),
            Err("--scrollback-start must be greater than 0")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 0, None, 1000),
            Err("--scrollback-count must be greater than 0")
        );
    }

    #[test]
    fn usage_mentions_live_interactive_flags() {
        let usage = usage();
        assert!(usage.contains("--stdin-bytes"));
        assert!(usage.contains("--local-echo off|tty"));
        assert!(usage.contains("--redraw"));
        assert!(usage.contains("--cols COUNT"));
        assert!(usage.contains("interim text surface"));
    }

    #[test]
    fn resize_policy_warning_only_applies_to_manual_policy_with_resize_request() {
        assert_eq!(
            resize_policy_warning(None, nmux_proto::protocol::ResizePolicy::Manual),
            None
        );
        assert_eq!(
            resize_policy_warning(Some((100, 30)), nmux_proto::protocol::ResizePolicy::Fixed),
            None
        );
        assert_eq!(
            resize_policy_warning(Some((100, 30)), nmux_proto::protocol::ResizePolicy::Manual),
            Some("nmux: resize request ignored by manual resize policy")
        );
    }

    #[test]
    fn stdin_bytes_detach_splits_before_ctrl_right_bracket() {
        assert_eq!(
            split_stdin_bytes_for_detach(b"ping\n"),
            (Some(b"ping\n".to_vec()), false)
        );
        assert_eq!(
            split_stdin_bytes_for_detach(b"ping\n\x1dignored"),
            (Some(b"ping\n".to_vec()), true)
        );
        assert_eq!(split_stdin_bytes_for_detach(b"\x1d"), (None, true));
    }
}
