use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{ArgAction, Parser};
use nmux_cli::local;
use nmux_core::session::AttachMode;
use nmux_proto::protocol;

const STDIN_BYTES_DETACH: u8 = 0x1d;
const REDRAW_TERMINAL_ENTER: &str = "\x1b[?1049h\x1b[?25l";
const REDRAW_TERMINAL_EXIT: &str = "\x1b[?25h\x1b[?1049l";
const VERSION: &str = env!("CARGO_PKG_VERSION");
static SIGWINCH_RECEIVED: AtomicBool = AtomicBool::new(false);
const SUPPORTED_KEY_NAMES: &[&str] = &[
    "numpad-enter",
    "numpad-0",
    "numpad-1",
    "numpad-2",
    "numpad-3",
    "numpad-4",
    "numpad-5",
    "numpad-6",
    "numpad-7",
    "numpad-8",
    "numpad-9",
    "arrow-up",
    "arrow-down",
    "arrow-right",
    "arrow-left",
    "enter",
    "tab",
    "space",
    "backspace",
    "escape",
    "insert",
    "delete",
    "home",
    "end",
    "page-up",
    "page-down",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];
const KEY_NAME_ALIASES: &[(&str, &str)] = &[
    ("keypad-enter", "numpad-enter"),
    ("keypad-0", "numpad-0"),
    ("keypad-1", "numpad-1"),
    ("keypad-2", "numpad-2"),
    ("keypad-3", "numpad-3"),
    ("keypad-4", "numpad-4"),
    ("keypad-5", "numpad-5"),
    ("keypad-6", "numpad-6"),
    ("keypad-7", "numpad-7"),
    ("keypad-8", "numpad-8"),
    ("keypad-9", "numpad-9"),
    ("kp-enter", "numpad-enter"),
    ("kp-0", "numpad-0"),
    ("kp-1", "numpad-1"),
    ("kp-2", "numpad-2"),
    ("kp-3", "numpad-3"),
    ("kp-4", "numpad-4"),
    ("kp-5", "numpad-5"),
    ("kp-6", "numpad-6"),
    ("kp-7", "numpad-7"),
    ("kp-8", "numpad-8"),
    ("kp-9", "numpad-9"),
    ("up", "arrow-up"),
    ("down", "arrow-down"),
    ("right", "arrow-right"),
    ("left", "arrow-left"),
    ("return", "enter"),
    ("esc", "escape"),
    ("ins", "insert"),
    ("del", "delete"),
    ("pgup", "page-up"),
    ("pageup", "page-up"),
    ("pgdn", "page-down"),
    ("pagedown", "page-down"),
    ("bs", "backspace"),
];
const KEY_MODIFIER_NAMES: &[&str] = &["shift", "ctrl", "alt", "super"];
const FOCUS_EVENT_NAMES: &[&str] = &["gained", "lost"];
const MOUSE_ACTION_NAMES: &[&str] = &["press", "release", "motion"];
const MOUSE_BUTTON_NAMES: &[&str] = &["none", "left", "middle", "right", "wheel-up", "wheel-down"];
const LOCAL_ECHO_NAMES: &[&str] = &["off", "tty"];
const DEFAULT_MANAGED_STARTUP_TIMEOUT_MS: u64 = 5000;

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

    if args.version || args.version_json {
        if args.version_json {
            println!("{}", local::version_json("nmux", VERSION));
        } else {
            println!("nmux {VERSION}");
        }
        return Ok(());
    }

    if args.list_key_names || args.list_key_names_json {
        print_key_names(args.list_key_names_json);
        return Ok(());
    }

    if args.list_input_choices_json {
        println!("{}", format_input_choices_json());
        return Ok(());
    }

    if args.print_context || args.print_context_json {
        if let Err(err) = print_context(args.print_context_json) {
            report_cli_error(&args, err.as_ref())?;
            return Err(err);
        }
        return Ok(());
    }

    if args.print_socket || args.print_socket_json {
        if args.print_socket_json {
            println!(
                "{}",
                local::socket_path_json(&args.socket_path, args.socket_source)
            );
        } else {
            println!("{}", args.socket_path.display());
        }
        return Ok(());
    }

    if args.state_info || args.state_info_json {
        if let Err(err) = print_state_info(&args) {
            report_cli_error(&args, err.as_ref())?;
            return Err(err);
        }
        return Ok(());
    }

    if args.start {
        return run_managed(args);
    }

    if args.live {
        return run_live(&args);
    }

    run_attach_loop(&args)
}

fn run_attach_loop(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let mut client_state = match load_client_state(args.state_path.as_deref()) {
        Ok(state) => state,
        Err(err) => {
            report_cli_error(&args, err.as_ref())?;
            return Err(err);
        }
    };

    let iterations = if args.follow {
        args.iterations.unwrap_or(usize::MAX)
    } else {
        1
    };

    for iteration in 0..iterations {
        let rendered = match attach_once(&args, &mut client_state) {
            Ok(rendered) => rendered,
            Err(err) => {
                if args.output_json {
                    println!("{}", format_cli_error_json(err.as_ref()));
                    flush_stdout()?;
                }
                return Err(err);
            }
        };
        if let Err(err) = save_client_state(args.state_path.as_deref(), &client_state) {
            report_cli_error(&args, err.as_ref())?;
            return Err(err);
        }
        if args.output_json {
            println!("{}", format_rendered_attach_json(&rendered));
        } else {
            print_rendered(rendered);
        }
        flush_stdout()?;

        if args.follow && iteration + 1 < iterations {
            thread::sleep(Duration::from_millis(args.interval_ms));
        }
    }

    Ok(())
}

fn run_live(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let _raw_terminal = match RawTerminalGuard::enable_if_needed(args.stdin_bytes, args.local_echo)
    {
        Ok(guard) => guard,
        Err(err) => {
            report_live_setup_error(args, &err)?;
            return Err(err.into());
        }
    };
    let _redraw_terminal = match RedrawTerminalGuard::enable_if_needed(args.redraw, stdout_is_tty())
    {
        Ok(guard) => guard,
        Err(err) => {
            report_live_setup_error(args, &err)?;
            return Err(err.into());
        }
    };
    warn_if_interim_surface_fidelity_is_visible(args.stdin_bytes);
    let mut sigwinch_resize =
        match SigwinchResize::enable_if_needed(args.stdin_bytes, args.live_resize.is_some()) {
            Ok(resize) => resize,
            Err(err) => {
                report_live_setup_error(args, &err)?;
                return Err(err.into());
            }
        };
    let mut client_state = match load_client_state(args.state_path.as_deref()) {
        Ok(state) => state,
        Err(err) => {
            report_live_setup_error(args, err.as_ref())?;
            return Err(err);
        }
    };
    let mut stream = match connect_to_daemon(args) {
        Ok(stream) => stream,
        Err(err) => {
            report_live_setup_error(args, err.as_ref())?;
            return Err(err);
        }
    };
    let socket_scope = local::socket_identity(&args.socket_path).ok();
    if let Err(err) = stream.set_read_timeout(Some(Duration::from_millis(args.interval_ms))) {
        report_live_setup_error(args, &err)?;
        return Err(err.into());
    }
    let stdin_lines = if args.stdin_input {
        Some(spawn_stdin_line_reader())
    } else {
        None
    };
    let stdin_bytes = if args.stdin_bytes {
        Some(spawn_stdin_byte_reader())
    } else {
        None
    };
    let mut stdin_closed = false;
    let mut stdin_bytes_closed = false;
    let mut detach_requested = false;
    let mut client_sequence = local::ClientFrameSequence::default();

    let mut options = local::AttachOptions {
        input_text: args.input_text.clone(),
        key_name: args.key_name.clone(),
        key_names: args.key_names.clone(),
        key_modifiers: args.key_modifiers,
        paste_text: args.paste_text.clone(),
        focus: args.focus_event.map(FocusEvent::focused),
        mouse: args.mouse_event.map(|mouse| local::AttachMouseInput {
            row: mouse.row,
            col: mouse.col,
            pixel_x: mouse.pixel_x,
            pixel_y: mouse.pixel_y,
            button: mouse.button,
            action: mouse.action,
            modifiers: mouse.modifiers,
        }),
        scrollback_start_line: args.scrollback_start_line,
        scrollback_line_count: args.scrollback_line_count,
        scrollback_tail_count: args.scrollback_tail_count,
        fetch_scrollback: !args.no_scrollback,
        connect_timeout: connect_timeout_duration(args),
        ..local::AttachOptions::default()
    };
    if args.stdin_input || args.stdin_bytes || (args.live_resize.is_some() && !args.no_input) {
        options.request.mode = AttachMode::ReadWrite;
    } else if options.input_text.is_none()
        && args.key_names.is_empty()
        && options.paste_text.is_none()
        && args.focus_event.is_none()
        && args.mouse_event.is_none()
    {
        options.request.mode = AttachMode::ReadOnly;
    }

    options.request.known_surfaces = client_state.known_surfaces_for_scope(socket_scope);
    if let Err(err) = local::write_attach_request(&mut stream, &options.request) {
        report_live_setup_error(args, &err)?;
        return Err(err.into());
    }
    let snapshot = match local::attach_from_stream(&mut stream) {
        Ok(snapshot) => snapshot,
        Err(err) => {
            report_live_setup_error(args, err.as_ref())?;
            return Err(err);
        }
    };
    let attached_pane_id = snapshot.status.pane_id.clone();
    client_state.apply_scope(local::socket_identity(&args.socket_path).ok());
    let mut rendered = match client_state.render_attach(snapshot) {
        Ok(rendered) => rendered,
        Err(err) => {
            report_live_setup_error(args, err.as_ref())?;
            return Err(err);
        }
    };
    if rendered.surface_text.is_none() {
        rendered.surface_text = client_state.cached_surface_text(&attached_pane_id);
        if let Some(surface) = client_state.cached_surface_summary(&attached_pane_id) {
            rendered.surface_kind = surface.surface_kind;
            rendered.cursor = surface.cursor;
            rendered.modes = surface.modes;
        }
        rendered.surface_metadata = client_state
            .cached_surface_metadata(&attached_pane_id)
            .unwrap_or_default();
    }
    let mut current_workspace = rendered.workspace.clone();
    let mut current_surface_metadata = rendered.surface_metadata.clone();
    let mut current_surface_text = rendered
        .surface_text
        .clone()
        .unwrap_or_else(|| current_workspace.display_line());
    let scrollback = match initial_live_scrollback(
        args,
        &mut stream,
        &mut client_sequence,
        &attached_pane_id,
        &client_state,
        socket_scope,
    ) {
        Ok(scrollback) => scrollback,
        Err(err) => {
            report_live_setup_error(args, err.as_ref())?;
            return Err(err);
        }
    };
    if let Some(scrollback) = scrollback.as_ref() {
        client_state.cache_scrollback_chunk(scrollback);
    }
    if args.output_json {
        rendered.scrollback = scrollback;
        println!("{}", format_live_attach_json(&rendered));
    } else {
        print_live_rendered(rendered, args.redraw, scrollback);
    }
    flush_stdout()?;

    let cycle_limit = args.iterations.or_else(|| {
        (!args.stdin_input && !args.stdin_bytes && options.request.mode == AttachMode::ReadWrite)
            .then_some(1)
    });
    let mut cycles = 0;
    let detach_reason = loop {
        if cycle_limit.is_some_and(|iterations| cycles >= iterations) {
            break LiveDetachReason::IterationLimit;
        }

        if options.request.mode == AttachMode::ReadWrite {
            if let Some((cols, rows)) = args.live_resize {
                local::send_resize_intent_with_reason_and_sequence(
                    &mut stream,
                    &mut client_sequence,
                    &attached_pane_id,
                    cols,
                    rows,
                    protocol::ResizeReason::UserCommand,
                )?;
            } else if let Some((cols, rows)) = sigwinch_resize.next_resize()? {
                local::send_resize_intent_with_reason_and_sequence(
                    &mut stream,
                    &mut client_sequence,
                    &attached_pane_id,
                    cols,
                    rows,
                    protocol::ResizeReason::FrontendViewport,
                )?;
            }
            let input_text = if let Some(receiver) = stdin_bytes.as_ref() {
                match receiver.try_recv() {
                    Ok(StdinByteRead::Input(input)) => {
                        let (input, detach) = split_stdin_bytes_for_detach(&input);
                        if let Some(input) = input {
                            local::send_raw_input_with_sequence(
                                &mut stream,
                                &mut client_sequence,
                                &attached_pane_id,
                                &input,
                            )?;
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
                match stdin_lines.as_ref().map(|receiver| receiver.try_recv()) {
                    Some(Ok(StdinLineRead::Input(line))) => Some(line),
                    Some(Ok(StdinLineRead::Closed)) => {
                        stdin_closed = true;
                        None
                    }
                    Some(Ok(StdinLineRead::Error(err))) => return Err(err.into()),
                    Some(Err(TryRecvError::Empty)) | None => None,
                    Some(Err(TryRecvError::Disconnected)) => {
                        stdin_closed = true;
                        None
                    }
                }
            } else {
                options.input_text.as_deref().map(ToOwned::to_owned)
            };
            if let Some(key_name) = args.key_name.as_deref() {
                for key_name in args
                    .key_names
                    .iter()
                    .map(String::as_str)
                    .chain(args.key_names.is_empty().then_some(key_name).into_iter())
                {
                    local::send_named_key_input_with_modifiers_and_sequence(
                        &mut stream,
                        &mut client_sequence,
                        &attached_pane_id,
                        key_name,
                        args.key_modifiers,
                    )?;
                }
            } else if let Some(mouse_event) = args.mouse_event {
                local::send_mouse_input_with_sequence(
                    &mut stream,
                    &mut client_sequence,
                    &attached_pane_id,
                    mouse_event.row,
                    mouse_event.col,
                    mouse_event.pixel_x,
                    mouse_event.pixel_y,
                    mouse_event.button,
                    mouse_event.action,
                    mouse_event.modifiers,
                )?;
            } else if let Some(focus_event) = args.focus_event {
                local::send_focus_input_with_sequence(
                    &mut stream,
                    &mut client_sequence,
                    &attached_pane_id,
                    focus_event.focused(),
                )?;
            } else if let Some(paste_text) = options.paste_text.as_deref() {
                local::send_paste_input_with_sequence(
                    &mut stream,
                    &mut client_sequence,
                    &attached_pane_id,
                    paste_text,
                )?;
            } else if let Some(input_text) = input_text.as_deref() {
                local::send_key_input_with_sequence(
                    &mut stream,
                    &mut client_sequence,
                    &attached_pane_id,
                    input_text,
                )?;
            }
        }

        loop {
            match local::read_live_surface_update_from_stream(&mut stream)? {
                local::LiveSurfaceRead::Workspace(workspace) => {
                    current_workspace = workspace;
                    if args.output_json {
                        println!("{}", format_live_workspace_json(&current_workspace));
                    } else if args.redraw {
                        print_live_surface(
                            &current_workspace,
                            &current_surface_metadata,
                            &current_surface_text,
                            args.redraw,
                        );
                    } else {
                        println!("{}", current_workspace.display_line());
                    }
                    flush_stdout()?;
                }
                local::LiveSurfaceRead::Update(update) => {
                    let previous_metadata = current_surface_metadata.clone();
                    current_surface_metadata = local::TerminalMetadataSummary {
                        title: update.title.clone(),
                        working_directory: update.working_directory.clone(),
                    };
                    current_surface_text = client_state.render_surface_update(&update)?;
                    if args.output_json {
                        println!(
                            "{}",
                            format_live_surface_update_json(
                                &current_workspace,
                                &current_surface_metadata,
                                &current_surface_text,
                                &update,
                            )
                        );
                    } else {
                        print_live_update(
                            &current_workspace,
                            &previous_metadata,
                            &current_surface_metadata,
                            &current_surface_text,
                            &update,
                            args.redraw,
                        );
                    }
                    flush_stdout()?;
                }
                local::LiveSurfaceRead::Error(error) => {
                    if args.output_json {
                        println!("{}", format_live_error_json(&error));
                        flush_stdout()?;
                    }
                    return Err(format!("live server error: {error}").into());
                }
                local::LiveSurfaceRead::NoFrame => break,
                local::LiveSurfaceRead::Closed => {
                    eprintln!("nmux: live server closed connection");
                    return finish_live(args, &client_state, LiveDetachReason::ServerClosed);
                }
            }
        }
        if detach_requested {
            eprintln!("nmux: detached by local Ctrl-]");
            break LiveDetachReason::LocalDetach;
        }
        if stdin_closed && args.iterations.is_none() {
            eprintln!("nmux: stdin EOF; detached");
            break LiveDetachReason::StdinEof;
        }
        if stdin_bytes_closed && args.iterations.is_none() {
            eprintln!("nmux: stdin EOF; detached");
            break LiveDetachReason::StdinEof;
        }
        cycles += 1;
    };

    finish_live(args, &client_state, detach_reason)
}

fn run_managed(mut args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = ManagedWorkspacePaths::new()?;
    if args.socket_source != local::SocketPathSource::Explicit {
        args.socket_path = workspace.socket_path.clone();
        args.socket_source = local::SocketPathSource::Explicit;
    }
    if args.state_path.is_none() {
        args.state_path = Some(workspace.state_path.clone());
    }
    if args.connect_timeout_ms.is_none() {
        args.connect_timeout_ms = Some(DEFAULT_MANAGED_STARTUP_TIMEOUT_MS);
    }
    let command = args
        .start_command
        .clone()
        .or_else(|| std::env::var("SHELL").ok())
        .filter(|command| !command.trim().is_empty())
        .unwrap_or_else(|| "sh".to_owned());
    let daemon_mode = if args.live {
        ManagedDaemonMode::LiveForever
    } else {
        ManagedDaemonMode::OneShot
    };
    let daemon = match ManagedDaemon::start(
        &args.socket_path,
        daemon_mode,
        &command,
        args.start_working_dir.as_deref(),
        &args.start_env,
        Duration::from_millis(args.startup_timeout_ms),
    ) {
        Ok(daemon) => daemon,
        Err(err) => {
            if args.live {
                report_live_setup_error(&args, err.as_ref())?;
            } else {
                report_cli_error(&args, err.as_ref())?;
            }
            return Err(err);
        }
    };
    if args.live {
        run_live(&args)?;
    } else {
        run_attach_loop(&args)?;
    }
    drop(daemon);
    Ok(())
}

struct ManagedWorkspacePaths {
    root: PathBuf,
    socket_path: PathBuf,
    state_path: PathBuf,
}

impl ManagedWorkspacePaths {
    fn new() -> io::Result<Self> {
        let root = PathBuf::from("/tmp").join(format!(
            "nmux-managed-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;
        Ok(Self {
            socket_path: root.join("nmuxd.sock"),
            state_path: root.join("state.nmux"),
            root,
        })
    }
}

impl Drop for ManagedWorkspacePaths {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct ManagedDaemon {
    child: Child,
}

#[derive(Clone, Copy)]
enum ManagedDaemonMode {
    OneShot,
    LiveForever,
}

impl ManagedDaemonMode {
    fn flag(self) -> &'static str {
        match self {
            Self::OneShot => "--one-shot",
            Self::LiveForever => "--live-forever",
        }
    }
}

impl ManagedDaemon {
    fn start(
        socket_path: &Path,
        mode: ManagedDaemonMode,
        command: &str,
        working_dir: Option<&str>,
        env: &[(String, String)],
        startup_timeout: Duration,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let nmuxd = nmuxd_binary_path()?;
        let socket_path = socket_path
            .to_str()
            .ok_or("managed socket path is not UTF-8")?;
        let mut command_args = vec![
            "--socket".to_owned(),
            socket_path.to_owned(),
            "--ready-json".to_owned(),
            mode.flag().to_owned(),
            "--command".to_owned(),
            command.to_owned(),
        ];
        if let Some(working_dir) = working_dir {
            command_args.push("--cwd".to_owned());
            command_args.push(working_dir.to_owned());
        }
        for (key, value) in env {
            command_args.push("--env".to_owned());
            command_args.push(format!("{key}={value}"));
        }
        let mut child = Command::new(nmuxd)
            .args(command_args)
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to start managed nmuxd: {err}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or("managed nmuxd stdout was not captured")?;
        let (ready_tx, ready_rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            let result = match reader.read_line(&mut line) {
                Ok(0) => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "managed nmuxd exited before readiness",
                )),
                Ok(_) => Ok(line),
                Err(err) => Err(err),
            };
            let _ = ready_tx.send(result);
        });

        let line = match ready_rx.recv_timeout(startup_timeout) {
            Ok(Ok(line)) => line,
            Ok(Err(err)) if err.kind() == io::ErrorKind::UnexpectedEof => {
                let _ = child.wait();
                return Err("managed nmuxd exited before readiness".into());
            }
            Ok(Err(err)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed to read managed nmuxd readiness: {err}").into());
            }
            Err(RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "managed nmuxd did not become ready within {} ms",
                    startup_timeout.as_millis()
                )
                .into());
            }
            Err(RecvTimeoutError::Disconnected) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("managed nmuxd readiness reader stopped unexpectedly".into());
            }
        };
        if line.contains("\"event\":\"ready\"") {
            return Ok(Self { child });
        }
        let _ = child.wait();
        Err(format!("managed nmuxd startup failed: {}", line.trim()).into())
    }
}

impl Drop for ManagedDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn nmuxd_binary_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = option_env!("CARGO_BIN_EXE_nmuxd") {
        return Ok(PathBuf::from(path));
    }
    let mut path = std::env::current_exe()?;
    path.set_file_name("nmuxd");
    Ok(path)
}

fn flush_stdout() -> io::Result<()> {
    io::stdout().flush()
}

fn warn_if_interim_surface_fidelity_is_visible(stdin_bytes: bool) {
    if interim_surface_fidelity_warning_needed(InterimSurfaceFidelityWarningContext {
        stdin_bytes,
        stdin_is_tty: stdin_is_tty(),
        stdout_is_tty: stdout_is_tty(),
    }) {
        eprintln!("{}", INTERIM_SURFACE_FIDELITY_WARNING);
    }
}

const INTERIM_SURFACE_FIDELITY_WARNING: &str = "nmux: interim text surface; ANSI styles, alternate screen, cursor motion, images, and full VT fidelity are unsupported";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct InterimSurfaceFidelityWarningContext {
    stdin_bytes: bool,
    stdin_is_tty: bool,
    stdout_is_tty: bool,
}

fn interim_surface_fidelity_warning_needed(context: InterimSurfaceFidelityWarningContext) -> bool {
    context.stdin_bytes && context.stdin_is_tty && context.stdout_is_tty
}

fn save_live_state(
    args: &Args,
    client_state: &local::ClientAttachState,
) -> Result<(), Box<dyn std::error::Error>> {
    save_client_state(args.state_path.as_deref(), client_state)
}

fn finish_live(
    args: &Args,
    client_state: &local::ClientAttachState,
    reason: LiveDetachReason,
) -> Result<(), Box<dyn std::error::Error>> {
    save_live_state(args, client_state)?;
    if args.output_json {
        println!("{}", format_live_detach_json(reason));
        flush_stdout()?;
    }
    Ok(())
}

fn report_live_setup_error(
    args: &Args,
    error: &(dyn std::error::Error + 'static),
) -> Result<(), Box<dyn std::error::Error>> {
    if args.output_json {
        println!("{}", format_live_cli_error_json(error));
        flush_stdout()?;
    }
    Ok(())
}

fn report_cli_error(
    args: &Args,
    error: &(dyn std::error::Error + 'static),
) -> Result<(), Box<dyn std::error::Error>> {
    if args.output_json || args.state_info_json || args.print_context_json {
        println!("{}", format_cli_error_json(error));
        flush_stdout()?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LiveDetachReason {
    IterationLimit,
    StdinEof,
    LocalDetach,
    ServerClosed,
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
    sequence: &mut local::ClientFrameSequence,
    pane_id: &str,
    client_state: &local::ClientAttachState,
    socket_scope: Option<local::SocketIdentity>,
) -> Result<Option<local::ScrollbackChunkSummary>, Box<dyn std::error::Error>> {
    if args.no_scrollback {
        return Ok(None);
    }
    Ok(Some(local::fetch_scrollback_chunk_with_selection(
        stream,
        sequence,
        pane_id,
        args.scrollback_start_line,
        args.scrollback_line_count,
        args.scrollback_tail_count,
        |start_line, line_count| {
            client_state
                .cached_scrollback_version_for_scope(socket_scope, pane_id, start_line, line_count)
                .unwrap_or(0)
        },
    )?))
}

fn spawn_stdin_line_reader() -> mpsc::Receiver<StdinLineRead> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(mut line) => {
                    line.push('\n');
                    if tx.send(StdinLineRead::Input(line)).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let _ = tx.send(StdinLineRead::Error(err.to_string()));
                    return;
                }
            }
        }
        let _ = tx.send(StdinLineRead::Closed);
    });
    rx
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

enum StdinLineRead {
    Input(String),
    Closed,
    Error(String),
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
        if !sigwinch_resize_needed(SigwinchResizeContext {
            stdin_bytes,
            explicit_resize,
            stdin_is_tty: stdin_is_tty(),
        }) {
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SigwinchResizeContext {
    stdin_bytes: bool,
    explicit_resize: bool,
    stdin_is_tty: bool,
}

fn sigwinch_resize_needed(context: SigwinchResizeContext) -> bool {
    context.stdin_bytes && !context.explicit_resize && context.stdin_is_tty
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
        key_name: args.key_name.clone(),
        key_names: args.key_names.clone(),
        key_modifiers: args.key_modifiers,
        paste_text: args.paste_text.clone(),
        focus: args.focus_event.map(FocusEvent::focused),
        mouse: args.mouse_event.map(|mouse| local::AttachMouseInput {
            row: mouse.row,
            col: mouse.col,
            pixel_x: mouse.pixel_x,
            pixel_y: mouse.pixel_y,
            button: mouse.button,
            action: mouse.action,
            modifiers: mouse.modifiers,
        }),
        scrollback_start_line: args.scrollback_start_line,
        scrollback_line_count: args.scrollback_line_count,
        scrollback_tail_count: args.scrollback_tail_count,
        fetch_scrollback: !args.no_scrollback,
        connect_timeout: connect_timeout_duration(args),
        ..local::AttachOptions::default()
    };
    if args.follow
        || (options.input_text.is_none()
            && options.key_name.is_none()
            && options.key_names.is_empty()
            && options.paste_text.is_none()
            && options.focus.is_none()
            && options.mouse.is_none())
    {
        options.request.mode = AttachMode::ReadOnly;
        options.input_text = None;
        options.key_name = None;
        options.key_names.clear();
        options.key_modifiers = 0;
        options.paste_text = None;
        options.focus = None;
        options.mouse = None;
    }

    local::attach_render_once(&args.socket_path, options, client_state)
}

fn connect_to_daemon(args: &Args) -> Result<UnixStream, Box<dyn std::error::Error>> {
    match connect_timeout_duration(args) {
        Some(timeout) => local::connect_to_daemon_with_timeout(&args.socket_path, timeout),
        None => local::connect_to_daemon(&args.socket_path),
    }
}

fn connect_timeout_duration(args: &Args) -> Option<Duration> {
    args.connect_timeout_ms.map(Duration::from_millis)
}

fn print_rendered(rendered: local::RenderedAttach) {
    println!("{}", rendered.workspace.display_line());
    print_terminal_metadata(&rendered.surface_metadata);
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
        let redraw_text = redraw_text_with_context(
            &rendered.workspace,
            &rendered.surface_metadata,
            &surface_text,
            initial_scrollback,
        );
        redraw_terminal(&redraw_text);
        return;
    }

    print_rendered(rendered);
    if let Some(scrollback) = initial_scrollback {
        print_scrollback(scrollback);
    }
}

fn print_live_surface(
    workspace: &local::WorkspaceSummary,
    metadata: &local::TerminalMetadataSummary,
    surface_text: &str,
    redraw: bool,
) {
    if redraw {
        redraw_terminal(&redraw_text_with_context(
            workspace,
            metadata,
            surface_text,
            None,
        ));
    } else {
        print_terminal_metadata(metadata);
        println!("{surface_text}");
    }
}

fn print_live_update(
    workspace: &local::WorkspaceSummary,
    previous_metadata: &local::TerminalMetadataSummary,
    metadata: &local::TerminalMetadataSummary,
    surface_text: &str,
    update: &local::SurfaceUpdate,
    redraw: bool,
) {
    match live_update_print_kind(previous_metadata, metadata, update, redraw) {
        LiveUpdatePrintKind::Surface => {
            print_live_surface(workspace, metadata, surface_text, redraw)
        }
        LiveUpdatePrintKind::Metadata => print_terminal_metadata(metadata),
        LiveUpdatePrintKind::None => {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveUpdatePrintKind {
    Surface,
    Metadata,
    None,
}

fn live_update_print_kind(
    previous_metadata: &local::TerminalMetadataSummary,
    metadata: &local::TerminalMetadataSummary,
    update: &local::SurfaceUpdate,
    redraw: bool,
) -> LiveUpdatePrintKind {
    if redraw
        || update.kind == local::SurfaceUpdateKind::Snapshot
        || update.patch_kind == Some(protocol::PatchKind::ReplaceRows)
    {
        LiveUpdatePrintKind::Surface
    } else if metadata != previous_metadata {
        LiveUpdatePrintKind::Metadata
    } else {
        LiveUpdatePrintKind::None
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
    metadata: &local::TerminalMetadataSummary,
    surface_text: &str,
    scrollback: Option<local::ScrollbackChunkSummary>,
) -> String {
    let mut text = workspace.display_line();
    text.push('\n');
    append_terminal_metadata(&mut text, metadata);
    if let Some(scrollback) = scrollback {
        text.push_str(&format_scrollback(&scrollback));
    }
    text.push_str(surface_text);
    text
}

fn print_terminal_metadata(metadata: &local::TerminalMetadataSummary) {
    for line in metadata.display_lines() {
        println!("{line}");
    }
}

fn append_terminal_metadata(text: &mut String, metadata: &local::TerminalMetadataSummary) {
    for line in metadata.display_lines() {
        text.push_str(&line);
        text.push('\n');
    }
}

fn format_scrollback(scrollback: &local::ScrollbackChunkSummary) -> String {
    let mut text = format!("scrollback {}:", scrollback_range_label(scrollback));
    text.push('\n');
    for line in &scrollback.lines {
        text.push_str(&line.text);
        text.push('\n');
    }
    text
}

fn scrollback_range_label(scrollback: &local::ScrollbackChunkSummary) -> String {
    let Some(first_line) = scrollback.lines.first().map(|line| line.line) else {
        return format!(
            "empty from {} of {}",
            scrollback.start_line, scrollback.total_lines
        );
    };
    let last_line = scrollback
        .lines
        .last()
        .map(|line| line.line)
        .unwrap_or(first_line);
    if last_line == scrollback.total_lines {
        format!("{first_line}..{last_line}")
    } else {
        format!("{first_line}..{last_line} of {}", scrollback.total_lines)
    }
}

#[derive(Clone)]
struct Args {
    help: bool,
    version: bool,
    version_json: bool,
    list_key_names: bool,
    list_key_names_json: bool,
    list_input_choices_json: bool,
    output_json: bool,
    print_context: bool,
    print_context_json: bool,
    print_socket: bool,
    print_socket_json: bool,
    state_info: bool,
    state_info_json: bool,
    socket_path: PathBuf,
    socket_source: local::SocketPathSource,
    input_text: Option<String>,
    key_name: Option<String>,
    key_names: Vec<String>,
    key_modifiers: u32,
    paste_text: Option<String>,
    focus_event: Option<FocusEvent>,
    mouse_event: Option<MouseEvent>,
    scrollback_start_line: u64,
    scrollback_line_count: u32,
    scrollback_tail_count: Option<u32>,
    no_scrollback: bool,
    state_path: Option<PathBuf>,
    follow: bool,
    live: bool,
    start: bool,
    start_command: Option<String>,
    start_working_dir: Option<String>,
    start_env: Vec<(String, String)>,
    startup_timeout_ms: u64,
    stdin_input: bool,
    stdin_bytes: bool,
    no_input: bool,
    local_echo: LocalEcho,
    redraw: bool,
    live_resize: Option<(u32, u32)>,
    interval_ms: u64,
    connect_timeout_ms: Option<u64>,
    iterations: Option<usize>,
}

#[derive(Debug, Parser)]
#[command(
    name = "nmux",
    disable_help_flag = true,
    disable_version_flag = true,
    args_override_self = true
)]
struct RawArgs {
    #[arg(short = 'h', long = "help", action = ArgAction::SetTrue)]
    help: bool,
    #[arg(short = 'V', long = "version", action = ArgAction::SetTrue)]
    version: bool,
    #[arg(long = "version-json", action = ArgAction::SetTrue)]
    version_json: bool,
    #[arg(long = "list-key-names", action = ArgAction::SetTrue)]
    list_key_names: bool,
    #[arg(long = "list-key-names-json", action = ArgAction::SetTrue)]
    list_key_names_json: bool,
    #[arg(long = "list-input-choices-json", action = ArgAction::SetTrue)]
    list_input_choices_json: bool,
    #[arg(long = "json", action = ArgAction::SetTrue)]
    output_json: bool,
    #[arg(long = "print-context", action = ArgAction::SetTrue)]
    print_context: bool,
    #[arg(long = "print-context-json", action = ArgAction::SetTrue)]
    print_context_json: bool,
    #[arg(long = "print-socket", action = ArgAction::SetTrue)]
    print_socket: bool,
    #[arg(long = "print-socket-json", action = ArgAction::SetTrue)]
    print_socket_json: bool,
    #[arg(long = "state-info", action = ArgAction::SetTrue)]
    state_info: bool,
    #[arg(long = "state-info-json", action = ArgAction::SetTrue)]
    state_info_json: bool,
    #[arg(long = "socket", value_name = "PATH")]
    socket_path: Option<PathBuf>,
    #[arg(long = "key", value_name = "TEXT", allow_hyphen_values = true)]
    key_text: Option<String>,
    #[arg(
        long = "key-name",
        value_name = "NAME",
        action = ArgAction::Append,
        allow_hyphen_values = true
    )]
    key_names: Vec<String>,
    #[arg(
        long = "key-modifiers",
        value_name = "MODS",
        allow_hyphen_values = true
    )]
    key_modifiers: Option<String>,
    #[arg(long = "paste", value_name = "TEXT", allow_hyphen_values = true)]
    paste_text: Option<String>,
    #[arg(long = "focus", value_name = "gained|lost", allow_hyphen_values = true)]
    focus_event: Option<String>,
    #[arg(
        long = "mouse",
        value_name = "action:button:row:col",
        allow_hyphen_values = true
    )]
    mouse_event: Option<String>,
    #[arg(
        long = "mouse-modifiers",
        value_name = "MODS",
        allow_hyphen_values = true
    )]
    mouse_modifiers: Option<String>,
    #[arg(long = "mouse-pixels", value_name = "x:y", allow_hyphen_values = true)]
    mouse_pixels: Option<String>,
    #[arg(long = "no-input", action = ArgAction::SetTrue)]
    no_input: bool,
    #[arg(long = "scrollback-start", value_name = "LINE")]
    scrollback_start: Option<String>,
    #[arg(long = "scrollback-count", value_name = "COUNT")]
    scrollback_count: Option<String>,
    #[arg(long = "scrollback-tail", value_name = "COUNT")]
    scrollback_tail: Option<String>,
    #[arg(long = "no-scrollback", action = ArgAction::SetTrue)]
    no_scrollback: bool,
    #[arg(long = "state", value_name = "PATH")]
    state_path: Option<PathBuf>,
    #[arg(long = "follow", action = ArgAction::SetTrue)]
    follow: bool,
    #[arg(long = "live", action = ArgAction::SetTrue)]
    live: bool,
    #[arg(long = "start", action = ArgAction::SetTrue)]
    start: bool,
    #[arg(long = "shell", action = ArgAction::SetTrue)]
    shell: bool,
    #[arg(long = "command", value_name = "SHELL", allow_hyphen_values = true)]
    start_command: Option<String>,
    #[arg(long = "cwd", value_name = "DIR", allow_hyphen_values = true)]
    start_working_dir: Option<String>,
    #[arg(
        long = "env",
        value_name = "KEY=VALUE",
        value_parser = parse_env_assignment,
        action = ArgAction::Append,
        allow_hyphen_values = true
    )]
    start_env: Vec<(String, String)>,
    #[arg(long = "startup-timeout-ms", value_name = "MS")]
    startup_timeout_ms: Option<String>,
    #[arg(long = "stdin", action = ArgAction::SetTrue)]
    stdin_input: bool,
    #[arg(long = "stdin-bytes", action = ArgAction::SetTrue)]
    stdin_bytes: bool,
    #[arg(
        long = "local-echo",
        value_name = "off|tty",
        allow_hyphen_values = true
    )]
    local_echo: Option<String>,
    #[arg(long = "redraw", action = ArgAction::SetTrue)]
    redraw: bool,
    #[arg(long = "cols", value_name = "COUNT")]
    live_cols: Option<String>,
    #[arg(long = "rows", value_name = "COUNT")]
    live_rows: Option<String>,
    #[arg(long = "interval-ms", value_name = "MS")]
    interval_ms: Option<String>,
    #[arg(long = "connect-timeout-ms", value_name = "MS")]
    connect_timeout_ms: Option<String>,
    #[arg(long = "iterations", value_name = "COUNT")]
    iterations: Option<String>,
}

fn args() -> Result<Args, Box<dyn std::error::Error>> {
    args_from_iter(std::env::args().skip(1))
}

fn args_from_iter<I, S>(args: I) -> Result<Args, Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    let raw = RawArgs::try_parse_from(
        std::iter::once(std::ffi::OsString::from("nmux")).chain(args.into_iter().map(Into::into)),
    )
    .map_err(clap_error_message)?;
    let (socket_path, socket_source) = match raw.socket_path {
        Some(path) => (path, local::SocketPathSource::Explicit),
        None => local::default_socket_path_and_source(),
    };
    let key_set = raw.key_text.is_some();
    let key_name_set = !raw.key_names.is_empty();
    let key_modifiers_set = raw.key_modifiers.is_some();
    let paste_set = raw.paste_text.is_some();
    let focus_set = raw.focus_event.is_some();
    let mouse_set = raw.mouse_event.is_some();
    let mouse_modifiers_set = raw.mouse_modifiers.is_some();
    let mouse_pixels_set = raw.mouse_pixels.is_some();
    let local_echo_set = raw.local_echo.is_some();
    let start = raw.start || raw.shell;
    let live = raw.live || raw.shell;
    let stdin_bytes = raw.stdin_bytes || raw.shell;
    let redraw = raw.redraw || raw.shell;
    let scrollback_start_set = raw.scrollback_start.is_some();
    let scrollback_count_set = raw.scrollback_count.is_some();
    let scrollback_tail_set = raw.scrollback_tail.is_some();
    let startup_timeout_set = raw.startup_timeout_ms.is_some();
    let scrollback_start_line = raw
        .scrollback_start
        .map(|value| parse_numeric_arg("--scrollback-start", value))
        .transpose()?
        .unwrap_or(1);
    let scrollback_line_count = raw
        .scrollback_count
        .map(|value| parse_numeric_arg("--scrollback-count", value))
        .transpose()?
        .unwrap_or(2);
    let scrollback_tail_count = raw
        .scrollback_tail
        .map(|value| parse_numeric_arg("--scrollback-tail", value))
        .transpose()?;
    let live_cols = raw
        .live_cols
        .map(|value| parse_numeric_arg("--cols", value))
        .transpose()?;
    let live_rows = raw
        .live_rows
        .map(|value| parse_numeric_arg("--rows", value))
        .transpose()?;
    let interval_ms = raw
        .interval_ms
        .map(|value| parse_numeric_arg("--interval-ms", value))
        .transpose()?
        .unwrap_or(1000);
    let connect_timeout_ms = raw
        .connect_timeout_ms
        .map(|value| parse_numeric_arg("--connect-timeout-ms", value))
        .transpose()?;
    let startup_timeout_ms = raw
        .startup_timeout_ms
        .map(|value| parse_numeric_arg("--startup-timeout-ms", value))
        .transpose()?
        .unwrap_or(DEFAULT_MANAGED_STARTUP_TIMEOUT_MS);
    let iterations = raw
        .iterations
        .map(|value| parse_numeric_arg("--iterations", value))
        .transpose()?;
    let mut key_name = None;
    let mut key_names = Vec::new();
    for raw_key_name in &raw.key_names {
        let parsed = parse_key_name(raw_key_name)?;
        if key_name.is_none() {
            key_name = Some(parsed.clone());
        }
        key_names.push(parsed);
    }
    let key_modifiers = raw
        .key_modifiers
        .as_deref()
        .map(parse_key_modifiers)
        .transpose()
        .map_err(|err| format!("--key-modifiers {err}"))?
        .unwrap_or(0);
    let mut mouse_event = raw
        .mouse_event
        .as_deref()
        .map(parse_mouse_event)
        .transpose()?;
    let mouse_modifiers = raw
        .mouse_modifiers
        .as_deref()
        .map(parse_key_modifiers)
        .transpose()
        .map_err(|err| format!("--mouse-modifiers {err}"))?
        .unwrap_or(0);
    let mouse_pixels = raw
        .mouse_pixels
        .as_deref()
        .map(parse_mouse_pixels)
        .transpose()?;
    let focus_event = raw
        .focus_event
        .as_deref()
        .map(parse_focus_event)
        .transpose()?;
    let local_echo = raw
        .local_echo
        .as_deref()
        .map(parse_local_echo)
        .transpose()
        .map_err(|err| format!("--local-echo {err}"))?
        .unwrap_or(LocalEcho::Off);
    let start_working_dir = match raw.start_working_dir {
        Some(value) if value.is_empty() => {
            return Err("--cwd requires a non-empty directory path".into());
        }
        value => value,
    };
    let mut input_text = raw.key_text;
    if key_name_set || paste_set || focus_set || mouse_set || raw.no_input || raw.follow {
        input_text = None;
    }
    let exits_before_attach = raw.help
        || raw.version
        || raw.version_json
        || raw.list_key_names
        || raw.list_key_names_json
        || raw.list_input_choices_json
        || raw.print_context
        || raw.print_context_json
        || raw.print_socket
        || raw.print_socket_json
        || raw.state_info
        || raw.state_info_json;
    let live_resize = if exits_before_attach {
        match (live_cols, live_rows) {
            (Some(cols), Some(rows)) => Some((cols, rows)),
            _ => None,
        }
    } else {
        match (live_cols, live_rows) {
            (Some(cols), Some(rows)) => Some((cols, rows)),
            (None, None) => None,
            _ => return Err("--cols and --rows must be provided together".into()),
        }
    };
    if !exits_before_attach {
        if raw.stdin_input && stdin_bytes {
            return Err("--stdin and --stdin-bytes cannot be used together".into());
        }
        validate_positive_numeric_args(
            scrollback_start_line,
            scrollback_line_count,
            scrollback_tail_count,
            live_resize,
            interval_ms,
            connect_timeout_ms,
            startup_timeout_ms,
        )?;
        validate_scrollback_selection_args(ScrollbackSelectionArgFlags {
            no_scrollback_set: raw.no_scrollback,
            scrollback_tail_set,
            scrollback_start_set,
            scrollback_count_set,
        })?;
        validate_explicit_input_modes(ExplicitInputModeArgs {
            key_set,
            key_name_set,
            paste_set,
            focus_set,
            mouse_set,
            no_input_set: raw.no_input,
            stdin_input: raw.stdin_input,
            stdin_bytes,
        })?;
        validate_no_input_resize_args(raw.no_input, live_resize)?;
        validate_mode_args(ClientModeArgs {
            live,
            follow: raw.follow,
            stdin_input: raw.stdin_input,
            stdin_bytes,
            local_echo_set,
            redraw,
            live_resize,
            iterations,
            output_json: raw.output_json,
            key_set,
            paste_set,
            focus_set,
            key_name_set,
            key_modifiers_set,
            mouse_set,
            mouse_modifiers_set,
            mouse_pixels_set,
            start,
            start_command_set: raw.start_command.is_some(),
            start_working_dir_set: start_working_dir.is_some(),
            start_env_set: !raw.start_env.is_empty(),
            startup_timeout_set,
        })?;
    }
    if let Some(mouse_event) = mouse_event.as_mut() {
        mouse_event.modifiers = mouse_modifiers;
        if let Some((pixel_x, pixel_y)) = mouse_pixels {
            mouse_event.pixel_x = Some(pixel_x);
            mouse_event.pixel_y = Some(pixel_y);
        }
    }

    Ok(Args {
        help: raw.help,
        version: raw.version,
        version_json: raw.version_json,
        list_key_names: raw.list_key_names,
        list_key_names_json: raw.list_key_names_json,
        list_input_choices_json: raw.list_input_choices_json,
        output_json: raw.output_json,
        print_context: raw.print_context,
        print_context_json: raw.print_context_json,
        print_socket: raw.print_socket,
        print_socket_json: raw.print_socket_json,
        state_info: raw.state_info,
        state_info_json: raw.state_info_json,
        socket_path,
        socket_source,
        input_text,
        key_name,
        key_names,
        key_modifiers,
        paste_text: raw.paste_text,
        focus_event,
        mouse_event,
        scrollback_start_line,
        scrollback_line_count,
        scrollback_tail_count,
        no_scrollback: raw.no_scrollback,
        state_path: raw.state_path,
        follow: raw.follow,
        live,
        start,
        start_command: raw.start_command,
        start_working_dir,
        start_env: raw.start_env,
        startup_timeout_ms,
        stdin_input: raw.stdin_input,
        stdin_bytes,
        no_input: raw.no_input,
        local_echo,
        redraw,
        live_resize,
        interval_ms,
        connect_timeout_ms,
        iterations,
    })
}

fn print_context(json: bool) -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("NMUX").ok().as_deref() != Some("1") {
        return Err("not running inside an nmux pane (NMUX=1 is not set)".into());
    }

    let session_id = required_context_env("NMUX_SESSION_ID")?;
    let pane_id = required_context_env("NMUX_PANE_ID")?;
    let socket = required_context_env("NMUX_SOCKET")?;
    let origin = required_context_env("NMUX_ORIGIN")?;

    if json {
        println!(
            "{}",
            format_context_json(&session_id, &pane_id, &socket, &origin)
        );
    } else {
        println!("NMUX=1");
        println!("NMUX_SESSION_ID={session_id}");
        println!("NMUX_PANE_ID={pane_id}");
        println!("NMUX_SOCKET={socket}");
        println!("NMUX_ORIGIN={origin}");
    }
    Ok(())
}

fn print_state_info(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = args.state_path.as_deref() else {
        return Err("--state-info requires --state PATH".into());
    };
    let exists = path.exists();
    let socket_identity = local::socket_identity(&args.socket_path)
        .ok()
        .map(local::SocketIdentitySummary::from);
    let state = local::ClientAttachState::load(path)
        .map_err(|err| format!("failed to load client state {}: {err}", path.display()))?;
    let summary = state.summary();
    let socket_info = StateInfoSocketSummary {
        path: &args.socket_path,
        exists: args.socket_path.exists(),
        scope_matches_socket: match (summary.scope, socket_identity) {
            (Some(scope), Some(identity)) => Some(scope == identity),
            _ => None,
        },
    };
    if args.state_info_json {
        println!(
            "{}",
            format_state_info_json(path, exists, &socket_info, &summary)
        );
    } else {
        print!(
            "{}",
            format_state_info_text(path, exists, &socket_info, &summary)
        );
    }
    Ok(())
}

struct StateInfoSocketSummary<'a> {
    path: &'a Path,
    exists: bool,
    scope_matches_socket: Option<bool>,
}

fn print_key_names(json: bool) {
    if json {
        println!("{}", format_key_names_json());
    } else {
        for key_name in SUPPORTED_KEY_NAMES {
            println!("{key_name}");
        }
        for (alias, canonical) in KEY_NAME_ALIASES {
            println!("{alias} -> {canonical}");
        }
    }
}

fn format_key_names_json() -> String {
    let names = format_json_string_array(SUPPORTED_KEY_NAMES);
    let aliases = KEY_NAME_ALIASES
        .iter()
        .map(|(alias, canonical)| {
            format!(
                "{{\"alias\":{},\"canonical\":{}}}",
                local::json_string(alias),
                local::json_string(canonical)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{\"names\":[{names}],\"aliases\":[{aliases}]}}")
}

fn format_input_choices_json() -> String {
    format!(
        "{{\"key_names\":{},\"key_modifiers\":{},\"focus_events\":{},\"mouse_actions\":{},\"mouse_buttons\":{},\"local_echo\":{}}}",
        format_key_names_json(),
        format_json_string_array(KEY_MODIFIER_NAMES),
        format_json_string_array(FOCUS_EVENT_NAMES),
        format_json_string_array(MOUSE_ACTION_NAMES),
        format_json_string_array(MOUSE_BUTTON_NAMES),
        format_json_string_array(LOCAL_ECHO_NAMES)
    )
}

fn format_rendered_attach_json(rendered: &local::RenderedAttach) -> String {
    let surface_text = rendered
        .surface_text
        .as_ref()
        .map(|text| local::json_string(text))
        .unwrap_or_else(|| "null".to_owned());
    let scrollback = rendered
        .scrollback
        .as_ref()
        .map(format_scrollback_json)
        .unwrap_or_else(|| "null".to_owned());
    format!(
        "{{\"workspace\":{},\"attach_status\":{},\"terminal\":{},\"surface\":{},\"surface_text\":{surface_text},\"scrollback\":{scrollback}}}",
        format_workspace_json(&rendered.workspace),
        format_attach_status_json(&rendered.status),
        format_rendered_terminal_json(rendered),
        format_rendered_surface_json(&rendered.surface),
    )
}

fn format_live_attach_json(rendered: &local::RenderedAttach) -> String {
    format!(
        "{{\"event\":\"attach\",\"attach\":{}}}",
        format_rendered_attach_json(rendered)
    )
}

fn format_live_workspace_json(workspace: &local::WorkspaceSummary) -> String {
    format!(
        "{{\"event\":\"workspace\",\"workspace\":{}}}",
        format_workspace_json(workspace)
    )
}

fn format_live_surface_update_json(
    workspace: &local::WorkspaceSummary,
    metadata: &local::TerminalMetadataSummary,
    surface_text: &str,
    update: &local::SurfaceUpdate,
) -> String {
    let base_version = update
        .base_version
        .map(|version| version.to_string())
        .unwrap_or_else(|| "null".to_owned());
    let patch_kind = update
        .patch_kind
        .map(patch_kind_name)
        .map(local::json_string)
        .unwrap_or_else(|| "null".to_owned());
    format!(
        "{{\"event\":\"surface\",\"workspace\":{},\"terminal\":{},\"surface_text\":{},\"update\":{{\"pane_id\":{},\"kind\":{},\"version\":{},\"base_version\":{base_version},\"patch_kind\":{patch_kind},\"rows\":{},\"styles\":{},\"hyperlinks\":{}}}}}",
        format_workspace_json(workspace),
        format_surface_update_terminal_json(metadata, update),
        local::json_string(surface_text),
        local::json_string(&update.pane_id),
        local::json_string(surface_update_kind_name(update.kind)),
        update.version,
        format_surface_rows_json(&update.row_updates),
        format_styles_json(&update.styles),
        format_hyperlinks_json(&update.hyperlinks)
    )
}

fn format_live_error_json(error: &local::ErrorSummary) -> String {
    format!(
        "{{\"event\":\"error\",\"error\":{}}}",
        format_error_summary_json(error)
    )
}

fn format_live_detach_json(reason: LiveDetachReason) -> String {
    format!(
        "{{\"event\":\"detach\",\"reason\":{}}}",
        local::json_string(live_detach_reason_name(reason))
    )
}

fn format_cli_error_json(error: &(dyn std::error::Error + 'static)) -> String {
    format!("{{\"error\":{}}}", format_cli_error_body_json(error))
}

fn format_live_cli_error_json(error: &(dyn std::error::Error + 'static)) -> String {
    format!(
        "{{\"event\":\"error\",\"error\":{}}}",
        format_cli_error_body_json(error)
    )
}

fn format_cli_error_body_json(error: &(dyn std::error::Error + 'static)) -> String {
    if let Some(error) = error.downcast_ref::<local::ServerError>() {
        return format_error_summary_json(&error.error);
    }
    format!("{{\"message\":{}}}", local::json_string(&error.to_string()))
}

fn format_error_summary_json(error: &local::ErrorSummary) -> String {
    let pane_id = error
        .pane_id
        .as_ref()
        .map(|pane_id| local::json_string(pane_id))
        .unwrap_or_else(|| "null".to_owned());
    format!(
        "{{\"code\":{},\"message\":{},\"retryable\":{},\"pane_id\":{pane_id},\"input_seq\":{}}}",
        local::json_string(error_code_name(error.code)),
        local::json_string(&error.message),
        error.retryable,
        error.input_seq
    )
}

fn format_workspace_json(workspace: &local::WorkspaceSummary) -> String {
    format!(
        "{{\"session_id\":{},\"tab_id\":{},\"pane_id\":{},\"cols\":{},\"rows\":{},\"resize_policy\":{}}}",
        local::json_string(&workspace.session_id),
        local::json_string(&workspace.tab_id),
        local::json_string(&workspace.pane_id),
        workspace.cols,
        workspace.rows,
        local::json_string(resize_policy_name(workspace.resize_policy))
    )
}

fn format_attach_status_json(status: &local::AttachStatusSummary) -> String {
    format!(
        "{{\"pane_id\":{},\"surface_version\":{},\"surface_state\":{}}}",
        local::json_string(&status.pane_id),
        status.surface_version,
        local::json_string(attach_surface_state_name(status.surface_state))
    )
}

fn format_rendered_terminal_json(rendered: &local::RenderedAttach) -> String {
    format!(
        "{{\"title\":{},\"working_directory\":{},\"surface_kind\":{},\"cursor\":{},\"modes\":{}}}",
        local::json_string(&rendered.surface_metadata.title),
        local::json_string(&rendered.surface_metadata.working_directory),
        local::json_string(surface_kind_name(rendered.surface_kind)),
        format_cursor_json(rendered.cursor),
        format_terminal_modes_json(rendered.modes)
    )
}

fn format_surface_update_terminal_json(
    metadata: &local::TerminalMetadataSummary,
    update: &local::SurfaceUpdate,
) -> String {
    let surface_kind = update
        .surface
        .map(surface_kind_name)
        .map(local::json_string)
        .unwrap_or_else(|| "null".to_owned());
    format!(
        "{{\"title\":{},\"working_directory\":{},\"surface_kind\":{surface_kind},\"cursor\":{},\"modes\":{}}}",
        local::json_string(&metadata.title),
        local::json_string(&metadata.working_directory),
        format_cursor_json(update.cursor),
        format_terminal_modes_json(update.modes)
    )
}

fn format_cursor_json(cursor: Option<local::CursorSummary>) -> String {
    let Some(cursor) = cursor else {
        return "null".to_owned();
    };
    format!(
        "{{\"row\":{},\"col\":{},\"visible\":{},\"shape\":{},\"blinking\":{}}}",
        cursor.row,
        cursor.col,
        cursor.visible,
        local::json_string(cursor_shape_name(cursor.shape)),
        cursor.blinking
    )
}

fn format_terminal_modes_json(modes: local::TerminalModeSummary) -> String {
    format!(
        "{{\"bracketed_paste\":{},\"mouse_tracking\":{},\"focus_reporting\":{},\"application_keypad\":{},\"application_cursor\":{},\"origin\":{},\"wraparound\":{},\"mouse_tracking_mode\":{},\"mouse_format\":{}}}",
        modes.bracketed_paste,
        modes.mouse_tracking,
        modes.focus_reporting,
        modes.application_keypad,
        modes.application_cursor,
        modes.origin,
        modes.wraparound,
        local::json_string(mouse_tracking_mode_name(modes.mouse_tracking_mode)),
        local::json_string(mouse_format_name(modes.mouse_format))
    )
}

fn format_rendered_surface_json(surface: &local::RenderedSurfaceSummary) -> String {
    format!(
        "{{\"pane_id\":{},\"version\":{},\"cols\":{},\"rows\":{},\"colors\":{},\"styles\":{},\"hyperlinks\":{},\"row_updates\":{}}}",
        local::json_string(&surface.pane_id),
        surface.version,
        surface.cols,
        surface.rows,
        format_terminal_colors_json(&surface.colors),
        format_styles_json(&surface.styles),
        format_hyperlinks_json(&surface.hyperlinks),
        format_surface_rows_json(&surface.row_updates)
    )
}

fn format_surface_rows_json(rows: &[local::SurfaceRowUpdate]) -> String {
    let rows = rows
        .iter()
        .map(|row| {
            format!(
                "{{\"row\":{},\"text\":{},\"dirty_hash\":{},\"row_state_hash\":{},\"semantic_prompt\":{},\"dirty\":{},\"kitty_virtual_placeholder\":{},\"runs\":{}}}",
                row.row,
                local::json_string(&row.text),
                row.dirty_hash,
                row.row_state_hash,
                local::json_string(row_semantic_prompt_name(row.semantic_prompt)),
                row.dirty,
                row.kitty_virtual_placeholder,
                format_cell_runs_json(&row.runs)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{rows}]")
}

fn format_cell_runs_json(runs: &[local::CellRunSummary]) -> String {
    let runs = runs
        .iter()
        .map(|run| {
            format!(
                "{{\"text\":{},\"cell_widths\":{},\"style_id\":{},\"flags\":{},\"hyperlink_id\":{},\"semantic_content\":{}}}",
                local::json_string(&run.text),
                format_u8_array_json(&run.cell_widths),
                run.style_id,
                run.flags,
                run.hyperlink_id,
                local::json_string(cell_semantic_content_name(run.semantic_content))
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{runs}]")
}

fn format_styles_json(styles: &[local::StyleSummary]) -> String {
    let styles = styles
        .iter()
        .map(|style| {
            format!(
                "{{\"fg_rgba\":{},\"bg_rgba\":{},\"underline_rgba\":{},\"flags\":{}}}",
                style.fg_rgba, style.bg_rgba, style.underline_rgba, style.flags
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{styles}]")
}

fn format_hyperlinks_json(hyperlinks: &[local::HyperlinkSummary]) -> String {
    let hyperlinks = hyperlinks
        .iter()
        .map(|hyperlink| {
            format!(
                "{{\"id\":{},\"uri\":{},\"osc8_id\":{},\"params\":{}}}",
                hyperlink.id,
                local::json_string(&hyperlink.uri),
                local::json_string(&hyperlink.osc8_id),
                local::json_string(&hyperlink.params)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{hyperlinks}]")
}

fn format_u8_array_json(values: &[u8]) -> String {
    let values = values
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("[{values}]")
}

fn format_scrollback_json(scrollback: &local::ScrollbackChunkSummary) -> String {
    let lines = scrollback
        .lines
        .iter()
        .map(|line| {
            format!(
                "{{\"line\":{},\"text\":{},\"dirty_hash\":{},\"row_state_hash\":{},\"semantic_prompt\":{},\"dirty\":{},\"kitty_virtual_placeholder\":{},\"runs\":{}}}",
                line.line,
                local::json_string(&line.text),
                line.dirty_hash,
                line.row_state_hash,
                local::json_string(row_semantic_prompt_name(line.semantic_prompt)),
                line.dirty,
                line.kitty_virtual_placeholder,
                format_cell_runs_json(&line.runs)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"pane_id\":{},\"scrollback_version\":{},\"start_line\":{},\"total_lines\":{},\"colors\":{},\"styles\":{},\"hyperlinks\":{},\"lines\":[{lines}]}}",
        local::json_string(&scrollback.pane_id),
        scrollback.scrollback_version,
        scrollback.start_line,
        scrollback.total_lines,
        format_terminal_colors_json(&scrollback.colors),
        format_styles_json(&scrollback.styles),
        format_hyperlinks_json(&scrollback.hyperlinks)
    )
}

fn format_terminal_colors_json(colors: &local::TerminalColorSummary) -> String {
    format!(
        "{{\"default_fg_rgba\":{},\"default_bg_rgba\":{},\"cursor_rgba\":{},\"cursor_rgba_set\":{},\"palette_rgba\":{},\"palette_diff_start\":{},\"palette_diff_rgba\":{}}}",
        colors.default_fg_rgba,
        colors.default_bg_rgba,
        colors.cursor_rgba,
        colors.cursor_rgba_set,
        format_u32_array_json(&colors.palette_rgba),
        colors
            .palette_diff_start
            .map(|start| start.to_string())
            .unwrap_or_else(|| "null".to_owned()),
        format_u32_array_json(&colors.palette_diff_rgba)
    )
}

fn format_u32_array_json(values: &[u32]) -> String {
    let values = values
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("[{values}]")
}

fn resize_policy_name(policy: protocol::ResizePolicy) -> &'static str {
    match policy {
        protocol::ResizePolicy::Fixed => "fixed",
        protocol::ResizePolicy::Leader => "leader",
        protocol::ResizePolicy::ActiveClient => "active-client",
        protocol::ResizePolicy::Manual => "manual",
        _ => "unknown",
    }
}

fn surface_update_kind_name(kind: local::SurfaceUpdateKind) -> &'static str {
    match kind {
        local::SurfaceUpdateKind::Snapshot => "snapshot",
        local::SurfaceUpdateKind::Patch => "patch",
    }
}

fn patch_kind_name(kind: protocol::PatchKind) -> &'static str {
    match kind {
        protocol::PatchKind::ReplaceRows => "replace-rows",
        protocol::PatchKind::CursorOnly => "cursor-only",
        protocol::PatchKind::ModeOnly => "mode-only",
        protocol::PatchKind::ColorOnly => "color-only",
        _ => "unknown",
    }
}

fn live_detach_reason_name(reason: LiveDetachReason) -> &'static str {
    match reason {
        LiveDetachReason::IterationLimit => "iteration-limit",
        LiveDetachReason::StdinEof => "stdin-eof",
        LiveDetachReason::LocalDetach => "local-detach",
        LiveDetachReason::ServerClosed => "server-closed",
    }
}

fn attach_surface_state_name(state: protocol::AttachSurfaceState) -> &'static str {
    match state {
        protocol::AttachSurfaceState::Current => "current",
        protocol::AttachSurfaceState::Snapshot => "snapshot",
        protocol::AttachSurfaceState::Patch => "patch",
        _ => "unknown",
    }
}

fn surface_kind_name(kind: protocol::SurfaceKind) -> &'static str {
    match kind {
        protocol::SurfaceKind::Main => "main",
        protocol::SurfaceKind::Alternate => "alternate",
        _ => "unknown",
    }
}

fn cursor_shape_name(shape: protocol::CursorShape) -> &'static str {
    match shape {
        protocol::CursorShape::Block => "block",
        protocol::CursorShape::Beam => "beam",
        protocol::CursorShape::Underline => "underline",
        _ => "unknown",
    }
}

fn mouse_tracking_mode_name(mode: protocol::MouseTrackingMode) -> &'static str {
    match mode {
        protocol::MouseTrackingMode::None => "none",
        protocol::MouseTrackingMode::X10 => "x10",
        protocol::MouseTrackingMode::Normal => "normal",
        protocol::MouseTrackingMode::Button => "button",
        protocol::MouseTrackingMode::Any => "any",
        _ => "unknown",
    }
}

fn mouse_format_name(format: protocol::MouseFormat) -> &'static str {
    match format {
        protocol::MouseFormat::X10 => "x10",
        protocol::MouseFormat::Utf8 => "utf8",
        protocol::MouseFormat::Sgr => "sgr",
        protocol::MouseFormat::Urxvt => "urxvt",
        protocol::MouseFormat::SgrPixels => "sgr-pixels",
        _ => "unknown",
    }
}

fn row_semantic_prompt_name(prompt: protocol::RowSemanticPrompt) -> &'static str {
    match prompt {
        protocol::RowSemanticPrompt::None => "none",
        protocol::RowSemanticPrompt::Prompt => "prompt",
        protocol::RowSemanticPrompt::Continuation => "continuation",
        _ => "unknown",
    }
}

fn cell_semantic_content_name(content: protocol::CellSemanticContent) -> &'static str {
    match content {
        protocol::CellSemanticContent::Output => "output",
        protocol::CellSemanticContent::Prompt => "prompt",
        protocol::CellSemanticContent::Input => "input",
        _ => "unknown",
    }
}

fn error_code_name(code: protocol::ErrorCode) -> &'static str {
    match code {
        protocol::ErrorCode::Unknown => "unknown",
        protocol::ErrorCode::ProtocolVersionUnsupported => "protocol-version-unsupported",
        protocol::ErrorCode::SessionNotFound => "session-not-found",
        protocol::ErrorCode::PaneNotFound => "pane-not-found",
        protocol::ErrorCode::PermissionDenied => "permission-denied",
        protocol::ErrorCode::StaleVersion => "stale-version",
        _ => "unknown",
    }
}

fn format_json_string_array(values: &[&str]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| local::json_string(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn format_context_json(session_id: &str, pane_id: &str, socket: &str, origin: &str) -> String {
    format!(
        "{{\"NMUX\":\"1\",\"NMUX_SESSION_ID\":{},\"NMUX_PANE_ID\":{},\"NMUX_SOCKET\":{},\"NMUX_ORIGIN\":{}}}",
        local::json_string(session_id),
        local::json_string(pane_id),
        local::json_string(socket),
        local::json_string(origin)
    )
}

fn format_state_info_text(
    path: &Path,
    exists: bool,
    socket_info: &StateInfoSocketSummary<'_>,
    summary: &local::ClientStateSummary,
) -> String {
    let mut output = String::new();
    output.push_str("state=");
    output.push_str(&path.display().to_string());
    output.push('\n');
    output.push_str("exists=");
    output.push_str(if exists { "true" } else { "false" });
    output.push('\n');
    output.push_str("socket=");
    output.push_str(&socket_info.path.display().to_string());
    output.push('\n');
    output.push_str("socket_exists=");
    output.push_str(if socket_info.exists { "true" } else { "false" });
    output.push('\n');
    output.push_str("scope_matches_socket=");
    output.push_str(match socket_info.scope_matches_socket {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    });
    output.push('\n');
    match summary.scope {
        Some(scope) => {
            output.push_str(&format!(
                "scope=socket dev={} ino={} ctime={}.{}\n",
                scope.dev, scope.ino, scope.ctime, scope.ctime_nsec
            ));
        }
        None => output.push_str("scope=none\n"),
    }
    output.push_str(&format!("surfaces={}\n", summary.surfaces.len()));
    for surface in &summary.surfaces {
        output.push_str(&format!(
            "surface pane={} version={} size={}x{} kind={}",
            surface.pane_id,
            surface.version,
            surface.cols,
            surface.rows,
            surface_kind_name(surface.surface_kind)
        ));
        if !surface.title.is_empty() {
            output.push_str(" title=");
            output.push_str(&surface.title);
        }
        if !surface.working_directory.is_empty() {
            output.push_str(" working_directory=");
            output.push_str(&surface.working_directory);
        }
        output.push('\n');
    }
    output.push_str(&format!("scrollbacks={}\n", summary.scrollbacks.len()));
    for scrollback in &summary.scrollbacks {
        let last_line = u64::from(scrollback.line_count)
            .checked_sub(1)
            .and_then(|offset| scrollback.start_line.checked_add(offset))
            .unwrap_or(scrollback.start_line);
        output.push_str(&format!(
            "scrollback pane={} version={} range={}..{} total={}\n",
            scrollback.pane_id,
            scrollback.version,
            scrollback.start_line,
            last_line,
            scrollback.total_lines
        ));
    }
    output
}

fn format_state_info_json(
    path: &Path,
    exists: bool,
    socket_info: &StateInfoSocketSummary<'_>,
    summary: &local::ClientStateSummary,
) -> String {
    let scope = summary
        .scope
        .map(|scope| {
            format!(
                "{{\"kind\":\"socket\",\"dev\":{},\"ino\":{},\"ctime\":{},\"ctime_nsec\":{}}}",
                scope.dev, scope.ino, scope.ctime, scope.ctime_nsec
            )
        })
        .unwrap_or_else(|| "null".to_owned());
    let scope_matches_socket = socket_info
        .scope_matches_socket
        .map(|matches| if matches { "true" } else { "false" })
        .unwrap_or("null");
    let surfaces = summary
        .surfaces
        .iter()
        .map(|surface| {
            format!(
                "{{\"pane_id\":{},\"version\":{},\"cols\":{},\"rows\":{},\"surface_kind\":{},\"title\":{},\"working_directory\":{},\"cursor\":{},\"modes\":{}}}",
                local::json_string(&surface.pane_id),
                surface.version,
                surface.cols,
                surface.rows,
                local::json_string(surface_kind_name(surface.surface_kind)),
                local::json_string(&surface.title),
                local::json_string(&surface.working_directory),
                format_cursor_json(surface.cursor),
                format_terminal_modes_json(surface.modes)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let scrollbacks = summary
        .scrollbacks
        .iter()
        .map(|scrollback| {
            format!(
                "{{\"pane_id\":{},\"version\":{},\"start_line\":{},\"line_count\":{},\"total_lines\":{}}}",
                local::json_string(&scrollback.pane_id),
                scrollback.version,
                scrollback.start_line,
                scrollback.line_count,
                scrollback.total_lines
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"path\":{},\"exists\":{},\"socket_path\":{},\"socket_exists\":{},\"scope_matches_socket\":{},\"scope\":{scope},\"surfaces\":[{surfaces}],\"scrollbacks\":[{scrollbacks}]}}",
        local::json_string(&path.display().to_string()),
        exists,
        local::json_string(&socket_info.path.display().to_string()),
        socket_info.exists,
        scope_matches_socket
    )
}

fn required_context_env(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        _ => Err(
            format!("not running inside a complete nmux pane context ({name} is not set)").into(),
        ),
    }
}

fn parse_numeric_arg<T>(flag: &str, value: String) -> Result<T, String>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|err| format!("{flag} requires a valid number: {err}"))
}

fn clap_error_message(error: clap::Error) -> String {
    let first_line = error.to_string();
    first_line
        .lines()
        .next()
        .unwrap_or("invalid command line")
        .trim_start_matches("error: ")
        .to_owned()
}

fn parse_env_assignment(value: &str) -> Result<(String, String), String> {
    let Some((key, value)) = value.split_once('=') else {
        return Err("--env requires KEY=VALUE".to_owned());
    };
    if key.is_empty() {
        return Err("--env requires a non-empty key".to_owned());
    }
    if key.contains('\0') || value.contains('\0') {
        return Err("--env cannot contain NUL bytes".to_owned());
    }
    Ok((key.to_owned(), value.to_owned()))
}

fn validate_no_input_resize_args(
    no_input_set: bool,
    live_resize: Option<(u32, u32)>,
) -> Result<(), &'static str> {
    if no_input_set && live_resize.is_some() {
        return Err("--no-input cannot be combined with --cols/--rows");
    }
    Ok(())
}

fn validate_positive_numeric_args(
    scrollback_start_line: u64,
    scrollback_line_count: u32,
    scrollback_tail_count: Option<u32>,
    live_resize: Option<(u32, u32)>,
    interval_ms: u64,
    connect_timeout_ms: Option<u64>,
    startup_timeout_ms: u64,
) -> Result<(), &'static str> {
    if scrollback_start_line == 0 {
        return Err("--scrollback-start must be greater than 0");
    }
    if scrollback_line_count == 0 {
        return Err("--scrollback-count must be greater than 0");
    }
    if scrollback_tail_count == Some(0) {
        return Err("--scrollback-tail must be greater than 0");
    }
    if interval_ms == 0 {
        return Err("--interval-ms must be greater than 0");
    }
    if connect_timeout_ms == Some(0) {
        return Err("--connect-timeout-ms must be greater than 0");
    }
    if startup_timeout_ms == 0 {
        return Err("--startup-timeout-ms must be greater than 0");
    }
    if live_resize.is_some_and(|(cols, rows)| {
        cols == 0 || rows == 0 || cols > u16::MAX as u32 || rows > u16::MAX as u32
    }) {
        return Err("--cols and --rows must be between 1 and 65535");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ScrollbackSelectionArgFlags {
    no_scrollback_set: bool,
    scrollback_tail_set: bool,
    scrollback_start_set: bool,
    scrollback_count_set: bool,
}

fn validate_scrollback_selection_args(
    args: ScrollbackSelectionArgFlags,
) -> Result<(), &'static str> {
    if args.no_scrollback_set && args.scrollback_tail_set {
        return Err("--no-scrollback cannot be combined with --scrollback-tail");
    }
    if args.no_scrollback_set && args.scrollback_start_set {
        return Err("--no-scrollback cannot be combined with --scrollback-start");
    }
    if args.no_scrollback_set && args.scrollback_count_set {
        return Err("--no-scrollback cannot be combined with --scrollback-count");
    }
    if args.scrollback_tail_set && args.scrollback_start_set {
        return Err("--scrollback-tail cannot be combined with --scrollback-start");
    }
    if args.scrollback_tail_set && args.scrollback_count_set {
        return Err("--scrollback-tail cannot be combined with --scrollback-count");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
struct ExplicitInputModeArgs {
    key_set: bool,
    key_name_set: bool,
    paste_set: bool,
    focus_set: bool,
    mouse_set: bool,
    no_input_set: bool,
    stdin_input: bool,
    stdin_bytes: bool,
}

fn validate_explicit_input_modes(args: ExplicitInputModeArgs) -> Result<(), &'static str> {
    if args.key_set && args.no_input_set {
        return Err("--key cannot be combined with --no-input");
    }
    if args.key_set && args.key_name_set {
        return Err("--key cannot be combined with --key-name");
    }
    if args.key_set && args.paste_set {
        return Err("--key cannot be combined with --paste");
    }
    if args.key_set && args.focus_set {
        return Err("--key cannot be combined with --focus");
    }
    if args.key_set && args.mouse_set {
        return Err("--key cannot be combined with --mouse");
    }
    if args.key_name_set && args.paste_set {
        return Err("--key-name cannot be combined with --paste");
    }
    if args.key_name_set && args.focus_set {
        return Err("--key-name cannot be combined with --focus");
    }
    if args.key_name_set && args.mouse_set {
        return Err("--key-name cannot be combined with --mouse");
    }
    if args.key_name_set && args.no_input_set {
        return Err("--key-name cannot be combined with --no-input");
    }
    if args.paste_set && args.focus_set {
        return Err("--paste cannot be combined with --focus");
    }
    if args.paste_set && args.mouse_set {
        return Err("--paste cannot be combined with --mouse");
    }
    if args.paste_set && args.no_input_set {
        return Err("--paste cannot be combined with --no-input");
    }
    if args.focus_set && args.no_input_set {
        return Err("--focus cannot be combined with --no-input");
    }
    if args.focus_set && args.mouse_set {
        return Err("--focus cannot be combined with --mouse");
    }
    if args.mouse_set && args.no_input_set {
        return Err("--mouse cannot be combined with --no-input");
    }
    if args.key_set && args.stdin_input {
        return Err("--key cannot be combined with --stdin");
    }
    if args.key_set && args.stdin_bytes {
        return Err("--key cannot be combined with --stdin-bytes");
    }
    if args.key_name_set && args.stdin_input {
        return Err("--key-name cannot be combined with --stdin");
    }
    if args.key_name_set && args.stdin_bytes {
        return Err("--key-name cannot be combined with --stdin-bytes");
    }
    if args.paste_set && args.stdin_input {
        return Err("--paste cannot be combined with --stdin");
    }
    if args.paste_set && args.stdin_bytes {
        return Err("--paste cannot be combined with --stdin-bytes");
    }
    if args.focus_set && args.stdin_input {
        return Err("--focus cannot be combined with --stdin");
    }
    if args.focus_set && args.stdin_bytes {
        return Err("--focus cannot be combined with --stdin-bytes");
    }
    if args.mouse_set && args.stdin_input {
        return Err("--mouse cannot be combined with --stdin");
    }
    if args.mouse_set && args.stdin_bytes {
        return Err("--mouse cannot be combined with --stdin-bytes");
    }
    if args.no_input_set && args.stdin_input {
        return Err("--no-input cannot be combined with --stdin");
    }
    if args.no_input_set && args.stdin_bytes {
        return Err("--no-input cannot be combined with --stdin-bytes");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
struct ClientModeArgs {
    live: bool,
    follow: bool,
    stdin_input: bool,
    stdin_bytes: bool,
    local_echo_set: bool,
    redraw: bool,
    live_resize: Option<(u32, u32)>,
    iterations: Option<usize>,
    output_json: bool,
    key_set: bool,
    paste_set: bool,
    focus_set: bool,
    key_name_set: bool,
    key_modifiers_set: bool,
    mouse_set: bool,
    mouse_modifiers_set: bool,
    mouse_pixels_set: bool,
    start: bool,
    start_command_set: bool,
    start_working_dir_set: bool,
    start_env_set: bool,
    startup_timeout_set: bool,
}

fn validate_mode_args(args: ClientModeArgs) -> Result<(), &'static str> {
    if args.start && args.follow {
        return Err("--start cannot be combined with --follow");
    }
    if args.start_command_set && !args.start {
        return Err("--command requires --start");
    }
    if args.start_working_dir_set && !args.start {
        return Err("--cwd requires --start");
    }
    if args.start_env_set && !args.start {
        return Err("--env requires --start");
    }
    if args.startup_timeout_set && !args.start {
        return Err("--startup-timeout-ms requires --start");
    }
    if args.live && args.follow {
        return Err("--follow cannot be combined with --live");
    }
    if args.follow && args.key_set {
        return Err("--follow cannot be combined with --key");
    }
    if args.follow && args.paste_set {
        return Err("--follow cannot be combined with --paste");
    }
    if args.follow && args.key_name_set {
        return Err("--follow cannot be combined with --key-name");
    }
    if args.follow && args.focus_set {
        return Err("--follow cannot be combined with --focus");
    }
    if args.follow && args.mouse_set {
        return Err("--follow cannot be combined with --mouse");
    }
    if args.stdin_input && !args.live {
        return Err("--stdin requires --live");
    }
    if args.stdin_bytes && !args.live {
        return Err("--stdin-bytes requires --live");
    }
    if args.local_echo_set && !args.stdin_bytes {
        return Err("--local-echo requires --stdin-bytes");
    }
    if args.redraw && !args.live {
        return Err("--redraw requires --live");
    }
    if args.live_resize.is_some() && !args.live {
        return Err("--cols and --rows require --live");
    }
    if args.output_json && args.follow {
        return Err("--json cannot be combined with --follow");
    }
    if args.output_json && args.redraw {
        return Err("--json cannot be combined with --redraw");
    }
    if args.key_modifiers_set && !args.key_name_set {
        return Err("--key-modifiers requires --key-name");
    }
    if args.mouse_modifiers_set && !args.mouse_set {
        return Err("--mouse-modifiers requires --mouse");
    }
    if args.mouse_pixels_set && !args.mouse_set {
        return Err("--mouse-pixels requires --mouse");
    }
    if args.iterations.is_some() && !args.live && !args.follow {
        return Err("--iterations requires --live or --follow");
    }
    if args.iterations == Some(0) {
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
  --print-context            Print inherited nmux pane context and exit
  --print-context-json       Print inherited nmux pane context as JSON and exit
  --print-socket             Print the resolved socket path and exit
  --print-socket-json        Print the resolved socket path as JSON and exit
  --connect-timeout-ms MS    Wait up to this long for the daemon socket
  --startup-timeout-ms MS    Wait up to this long for managed nmuxd readiness
  --state-info               Inspect --state cache without connecting
  --state-info-json          Inspect --state cache as JSON without connecting
  --key TEXT                 Text input to send; opts into read-write attach
  --key-name NAME            Send a supported named key; repeat for a sequence
  --list-key-names           List supported --key-name values and aliases
  --list-key-names-json      List supported --key-name values as JSON
  --list-input-choices-json  List structured input choices as JSON
  --json                     Print attach output as JSON; live uses JSON lines
  --key-modifiers MODS       Modifiers for --key-name: shift,ctrl,alt,super
  --paste TEXT               Paste UTF-8 text through PasteInput
  --focus gained|lost        Send focus input; daemon rejects if reporting is off
  --mouse A:B:R:C            Send mouse press/release/motion input
  --mouse-modifiers MODS     Modifiers for --mouse: shift,ctrl,alt,super
  --mouse-pixels X:Y         Pixel coordinates for --mouse SGR-pixels mode
  --no-input                 Attach read-only
  --scrollback-start LINE    First scrollback line to request
  --scrollback-count COUNT   Number of scrollback lines to request
  --scrollback-tail COUNT    Request the last COUNT scrollback lines
  --no-scrollback            Skip the post-attach scrollback fetch
  --state PATH               Persist client-side pane surface cache
  --follow                   Reconnect in a polling loop
  --live                     Keep one attach connection open
  --start                    Start a private local nmuxd before attaching
  --shell                    Start a private live shell with stdin-bytes redraw
  --command SHELL            Managed nmuxd pane command for --start
  --cwd DIR                  Managed nmuxd pane working directory for --start
  --env KEY=VALUE            Managed nmuxd pane environment for --start
  --stdin                    Stream newline-delimited stdin in live mode
  --stdin-bytes              Stream raw stdin chunks in live mode
  --local-echo off|tty       Local TTY echo policy for --stdin-bytes
  --redraw                   Repaint the current live surface in place
  --cols COUNT               Live ResizeIntent columns; both dimensions required
  --rows COUNT               Live ResizeIntent rows; both dimensions required
  --interval-ms MS           Poll/read timeout in milliseconds
  --iterations COUNT         Bounded follow/live cycle count
  --version-json             Show version as JSON
  -V, --version              Show version
  -h, --help                 Show this help

Notes:
  Default socket: --socket, else valid absolute $NMUX_SOCKET, else valid absolute $XDG_RUNTIME_DIR/nmux/nmuxd.sock, else /tmp/nmux-$UID/nmuxd.sock.
  --print-context prints inherited NMUX_* pane identity without connecting.
  --print-context-json prints the same inherited context as a JSON object.
  --print-socket-json prints the resolved socket path and source as JSON.
  --state-info and --state-info-json require --state PATH and do not connect.
  --json emits one object for one-shot attach, or newline-delimited live events.
  --start waits for nmuxd --ready-json and cleans up the private daemon on exit.
  --startup-timeout-ms controls that managed readiness wait and defaults to 5000.
  --shell is shorthand for --start --live --stdin-bytes --redraw using $SHELL or sh.
  NMUX_ORIGIN records the local hop chain for nested nmux daemons.
  Informational flags exit before mode validation or socket/state work.
  Without an explicit input or resize flag, nmux attaches read-only.
  The current renderer uses an interim text surface, not a VT-correct terminal emulator.

Examples:
  nmux
  nmux --key 'ping\n'
  nmux --live --iterations 2 --key 'ping\n'
  nmux --live --cols 100 --rows 30
  nmux --live --no-input
  nmux --live --stdin-bytes --redraw
  nmux --shell
  nmux --start --cwd /tmp --env NMUX_DEMO=1 --command 'pwd; env | grep ^NMUX_DEMO=; cat >/dev/null'
  nmux --start --live --stdin-bytes --redraw --command '$SHELL'
"
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalEcho {
    Off,
    Tty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FocusEvent {
    Gained,
    Lost,
}

impl FocusEvent {
    fn focused(self) -> bool {
        matches!(self, Self::Gained)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MouseEvent {
    action: protocol::MouseAction,
    button: protocol::MouseButton,
    row: u32,
    col: u32,
    pixel_x: Option<u32>,
    pixel_y: Option<u32>,
    modifiers: u32,
}

fn parse_local_echo(value: &str) -> Result<LocalEcho, &'static str> {
    match value {
        "off" => Ok(LocalEcho::Off),
        "tty" => Ok(LocalEcho::Tty),
        _ => Err("requires off or tty"),
    }
}

fn parse_focus_event(value: &str) -> Result<FocusEvent, &'static str> {
    match value {
        "gained" => Ok(FocusEvent::Gained),
        "lost" => Ok(FocusEvent::Lost),
        _ => Err("--focus requires gained or lost"),
    }
}

fn parse_key_name(value: &str) -> Result<String, &'static str> {
    if SUPPORTED_KEY_NAMES.contains(&value) {
        return Ok(value.to_owned());
    }
    KEY_NAME_ALIASES
        .iter()
        .find_map(|(alias, canonical)| (*alias == value).then_some((*canonical).to_owned()))
        .ok_or("--key-name requires a supported named key")
}

fn parse_key_modifiers(value: &str) -> Result<u32, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        return Err("requires shift, ctrl, alt, super, or none");
    }
    if value == "none" {
        return Ok(0);
    }

    let mut modifiers = 0;
    for part in value.split([',', '+']) {
        match part.trim() {
            "shift" => modifiers |= 1,
            "ctrl" | "control" => modifiers |= 2,
            "alt" | "option" => modifiers |= 4,
            "super" | "cmd" | "command" | "meta" => modifiers |= 8,
            "" | "none" => return Err("requires shift, ctrl, alt, super, or none"),
            _ => return Err("contains an unsupported modifier"),
        }
    }
    Ok(modifiers)
}

fn parse_mouse_event(value: &str) -> Result<MouseEvent, &'static str> {
    let mut parts = value.split(':');
    let action = parse_mouse_action(
        parts
            .next()
            .ok_or("--mouse requires action:button:row:col")?,
    )?;
    let button = parse_mouse_button(
        parts
            .next()
            .ok_or("--mouse requires action:button:row:col")?,
    )?;
    let row = parse_one_based_cell(
        parts
            .next()
            .ok_or("--mouse requires action:button:row:col")?,
    )?;
    let col = parse_one_based_cell(
        parts
            .next()
            .ok_or("--mouse requires action:button:row:col")?,
    )?;
    if parts.next().is_some() {
        return Err("--mouse requires action:button:row:col");
    }

    Ok(MouseEvent {
        action,
        button,
        row,
        col,
        pixel_x: None,
        pixel_y: None,
        modifiers: 0,
    })
}

fn parse_mouse_pixels(value: &str) -> Result<(u32, u32), &'static str> {
    let mut parts = value.split(':');
    let pixel_x = parse_pixel_coordinate(parts.next().ok_or("--mouse-pixels requires x:y")?)?;
    let pixel_y = parse_pixel_coordinate(parts.next().ok_or("--mouse-pixels requires x:y")?)?;
    if parts.next().is_some() {
        return Err("--mouse-pixels requires x:y");
    }
    Ok((pixel_x, pixel_y))
}

fn parse_pixel_coordinate(value: &str) -> Result<u32, &'static str> {
    value
        .parse::<u32>()
        .map_err(|_| "--mouse-pixels x and y must be non-negative integers")
}

fn parse_mouse_action(value: &str) -> Result<protocol::MouseAction, &'static str> {
    match value {
        "press" => Ok(protocol::MouseAction::Press),
        "release" => Ok(protocol::MouseAction::Release),
        "motion" => Ok(protocol::MouseAction::Motion),
        _ => Err("--mouse action must be press, release, or motion"),
    }
}

fn parse_mouse_button(value: &str) -> Result<protocol::MouseButton, &'static str> {
    match value {
        "none" => Ok(protocol::MouseButton::None),
        "left" => Ok(protocol::MouseButton::Left),
        "middle" => Ok(protocol::MouseButton::Middle),
        "right" => Ok(protocol::MouseButton::Right),
        "wheel-up" => Ok(protocol::MouseButton::WheelUp),
        "wheel-down" => Ok(protocol::MouseButton::WheelDown),
        _ => Err("--mouse button must be none, left, middle, right, wheel-up, or wheel-down"),
    }
}

fn parse_one_based_cell(value: &str) -> Result<u32, &'static str> {
    let value = value
        .parse::<u32>()
        .map_err(|_| "--mouse row and col must be positive integers")?;
    value
        .checked_sub(1)
        .ok_or("--mouse row and col must be positive integers")
}

#[cfg(test)]
mod tests {
    use super::{
        ClientModeArgs, ExplicitInputModeArgs, FocusEvent, InterimSurfaceFidelityWarningContext,
        KEY_NAME_ALIASES, LiveDetachReason, LiveUpdatePrintKind, LocalEcho, MouseEvent,
        SUPPORTED_KEY_NAMES, ScrollbackSelectionArgFlags, SigwinchResizeContext,
        StateInfoSocketSummary, args_from_iter, format_cli_error_json, format_context_json,
        format_input_choices_json, format_key_names_json, format_live_attach_json,
        format_live_cli_error_json, format_live_detach_json, format_live_error_json,
        format_live_surface_update_json, format_live_workspace_json, format_rendered_attach_json,
        format_scrollback, format_state_info_json, format_state_info_text,
        interim_surface_fidelity_warning_needed, live_update_print_kind, parse_env_assignment,
        parse_focus_event, parse_key_modifiers, parse_key_name, parse_local_echo,
        parse_mouse_event, parse_mouse_pixels, parse_numeric_arg, raw_terminal_lflag,
        raw_terminal_mode_needed, redraw_terminal_guard_needed, sigwinch_resize_needed,
        split_stdin_bytes_for_detach, terminal_size_from_winsize, usage,
        validate_explicit_input_modes as super_validate_explicit_input_modes,
        validate_mode_args as super_validate_mode_args, validate_no_input_resize_args,
        validate_positive_numeric_args, validate_scrollback_selection_args,
    };
    use nmux_cli::local;
    use nmux_proto::protocol;
    use std::path::Path;

    fn test_surface_update(
        kind: local::SurfaceUpdateKind,
        patch_kind: Option<protocol::PatchKind>,
    ) -> local::SurfaceUpdate {
        local::SurfaceUpdate {
            kind,
            pane_id: "pane-1".to_owned(),
            version: 7,
            base_version: (kind == local::SurfaceUpdateKind::Patch).then_some(6),
            patch_kind,
            cols: (kind == local::SurfaceUpdateKind::Snapshot).then_some(80),
            rows: (kind == local::SurfaceUpdateKind::Snapshot).then_some(24),
            surface: (kind == local::SurfaceUpdateKind::Snapshot)
                .then_some(protocol::SurfaceKind::Main),
            cursor: None,
            modes: local::TerminalModeSummary::default(),
            title: String::new(),
            working_directory: String::new(),
            colors: None,
            row_updates: Vec::new(),
            styles: Vec::new(),
            hyperlinks: Vec::new(),
            text: String::new(),
        }
    }

    #[test]
    fn default_args_attach_read_only_without_implicit_input() {
        let args = args_from_iter(std::iter::empty::<&str>()).expect("args");
        assert_eq!(args.input_text, None);
        assert_eq!(args.key_name, None);
        assert!(args.key_names.is_empty());
        assert_eq!(args.paste_text, None);
        assert!(!args.stdin_input);
        assert!(!args.stdin_bytes);
        assert!(!args.no_input);
        assert!(!args.live);
        assert!(!args.version);
        assert!(!args.version_json);
        assert!(!args.list_key_names);
        assert!(!args.list_key_names_json);
        assert!(!args.list_input_choices_json);
        assert!(!args.output_json);
        assert!(!args.print_context);
        assert!(!args.print_context_json);
        assert!(!args.print_socket);
        assert!(!args.print_socket_json);
        assert!(!args.state_info);
        assert!(!args.state_info_json);
    }

    #[test]
    fn start_args_accept_managed_command() {
        let args = args_from_iter([
            "--start",
            "--command",
            "printf hi",
            "--cwd",
            "/tmp",
            "--env",
            "NMUX_DEMO=one=two",
        ])
        .expect("args");
        assert!(args.start);
        assert!(!args.live);
        assert_eq!(args.start_command.as_deref(), Some("printf hi"));
        assert_eq!(args.start_working_dir.as_deref(), Some("/tmp"));
        assert_eq!(
            args.start_env,
            vec![("NMUX_DEMO".to_owned(), "one=two".to_owned())]
        );
    }

    #[test]
    fn shell_arg_expands_to_private_interactive_workspace() {
        let args = args_from_iter(["--shell"]).expect("args");
        assert!(args.start);
        assert!(args.live);
        assert!(args.stdin_bytes);
        assert!(args.redraw);
        assert!(!args.stdin_input);
        assert!(args.start_command.is_none());
    }

    #[test]
    fn env_arg_accepts_key_value_with_equals_in_value() {
        assert_eq!(
            parse_env_assignment("NMUX_TEST=one=two"),
            Ok(("NMUX_TEST".to_owned(), "one=two".to_owned()))
        );
        assert_eq!(
            parse_env_assignment("=value"),
            Err("--env requires a non-empty key".to_owned())
        );
        assert_eq!(
            parse_env_assignment("missing"),
            Err("--env requires KEY=VALUE".to_owned())
        );
    }

    #[test]
    fn print_context_json_arg_exits_before_mode_validation() {
        let args = args_from_iter(["--print-context-json", "--cols", "80"]).expect("args");
        assert!(args.print_context_json);
    }

    #[test]
    fn version_json_arg_exits_before_mode_validation() {
        let args = args_from_iter(["--version-json", "--cols", "80"]).expect("args");
        assert!(args.version_json);
    }

    #[test]
    fn list_key_names_arg_exits_before_mode_validation() {
        let args = args_from_iter(["--list-key-names", "--cols", "80"]).expect("args");
        assert!(args.list_key_names);
    }

    #[test]
    fn list_key_names_json_arg_exits_before_mode_validation() {
        let args = args_from_iter(["--list-key-names-json", "--cols", "80"]).expect("args");
        assert!(args.list_key_names_json);
    }

    #[test]
    fn list_input_choices_json_arg_exits_before_mode_validation() {
        let args = args_from_iter(["--list-input-choices-json", "--cols", "80"]).expect("args");
        assert!(args.list_input_choices_json);
    }

    #[test]
    fn key_names_json_lists_names_and_aliases() {
        let json = format_key_names_json();
        assert!(json.starts_with("{\"names\":["));
        assert!(json.contains("\"numpad-enter\""));
        assert!(json.contains("\"space\""));
        assert!(json.contains("{\"alias\":\"esc\",\"canonical\":\"escape\"}"));
        assert!(json.contains("{\"alias\":\"kp-0\",\"canonical\":\"numpad-0\"}"));
    }

    #[test]
    fn input_choices_json_lists_structured_input_vocabularies() {
        let json = format_input_choices_json();
        assert!(json.contains("\"key_names\":{\"names\":["));
        assert!(json.contains("\"key_modifiers\":[\"shift\",\"ctrl\",\"alt\",\"super\"]"));
        assert!(json.contains("\"focus_events\":[\"gained\",\"lost\"]"));
        assert!(json.contains("\"mouse_actions\":[\"press\",\"release\",\"motion\"]"));
        assert!(json.contains(
            "\"mouse_buttons\":[\"none\",\"left\",\"middle\",\"right\",\"wheel-up\",\"wheel-down\"]"
        ));
        assert!(json.contains("\"local_echo\":[\"off\",\"tty\"]"));
    }

    #[test]
    fn context_json_escapes_values() {
        assert_eq!(local::json_string("pane\"1\\x\n"), "\"pane\\\"1\\\\x\\n\"");
        assert_eq!(
            format_context_json("session", "pane-1", "/tmp/nmux.sock", "root>child"),
            "{\"NMUX\":\"1\",\"NMUX_SESSION_ID\":\"session\",\"NMUX_PANE_ID\":\"pane-1\",\"NMUX_SOCKET\":\"/tmp/nmux.sock\",\"NMUX_ORIGIN\":\"root>child\"}"
        );
    }

    #[test]
    fn rendered_attach_json_reports_workspace_surface_and_scrollback() {
        let rendered = local::RenderedAttach {
            workspace: local::WorkspaceSummary {
                session_id: "session\"1".to_owned(),
                tab_id: "tab-1".to_owned(),
                pane_id: "pane-1".to_owned(),
                cols: 80,
                rows: 24,
                resize_policy: protocol::ResizePolicy::ActiveClient,
            },
            status: local::AttachStatusSummary {
                pane_id: "pane-1".to_owned(),
                surface_version: 17,
                surface_state: protocol::AttachSurfaceState::Snapshot,
            },
            surface_metadata: local::TerminalMetadataSummary {
                title: "build\nshell".to_owned(),
                working_directory: "/tmp/nmux".to_owned(),
            },
            surface_kind: protocol::SurfaceKind::Alternate,
            cursor: Some(local::CursorSummary {
                row: 3,
                col: 4,
                visible: true,
                shape: protocol::CursorShape::Beam,
                blinking: false,
            }),
            modes: local::TerminalModeSummary {
                bracketed_paste: true,
                mouse_tracking: true,
                focus_reporting: true,
                application_keypad: false,
                application_cursor: true,
                origin: false,
                wraparound: true,
                mouse_tracking_mode: protocol::MouseTrackingMode::Any,
                mouse_format: protocol::MouseFormat::SgrPixels,
            },
            surface: local::RenderedSurfaceSummary {
                pane_id: "pane-1".to_owned(),
                version: 17,
                cols: 80,
                rows: 24,
                colors: local::TerminalColorSummary {
                    default_fg_rgba: 21,
                    default_bg_rgba: 22,
                    cursor_rgba: 23,
                    cursor_rgba_set: true,
                    palette_rgba: vec![24, 25],
                    palette_diff_start: None,
                    palette_diff_rgba: Vec::new(),
                },
                styles: vec![local::StyleSummary {
                    fg_rgba: 31,
                    bg_rgba: 32,
                    underline_rgba: 33,
                    flags: 34,
                }],
                hyperlinks: vec![local::HyperlinkSummary {
                    id: 7,
                    uri: "https://example.test/surface".to_owned(),
                    osc8_id: "surface-id".to_owned(),
                    params: "id=surface-id".to_owned(),
                }],
                row_updates: vec![local::SurfaceRowUpdate {
                    row: 0,
                    text: "hello".to_owned(),
                    runs: vec![local::CellRunSummary {
                        text: "hello".to_owned(),
                        cell_widths: vec![1, 1, 1, 1, 1],
                        style_id: 0,
                        flags: 1,
                        hyperlink_id: 7,
                        semantic_content: protocol::CellSemanticContent::Prompt,
                    }],
                    dirty_hash: 41,
                    row_state_hash: 42,
                    semantic_prompt: protocol::RowSemanticPrompt::Prompt,
                    dirty: true,
                    kitty_virtual_placeholder: false,
                }],
            },
            surface_text: Some("hello\nworld".to_owned()),
            scrollback: Some(local::ScrollbackChunkSummary {
                pane_id: "pane-1".to_owned(),
                scrollback_version: 9,
                start_line: 1,
                total_lines: 2,
                styles: vec![local::StyleSummary {
                    fg_rgba: 1,
                    bg_rgba: 2,
                    underline_rgba: 3,
                    flags: 4,
                }],
                hyperlinks: vec![local::HyperlinkSummary {
                    id: 8,
                    uri: "https://example.test/scroll".to_owned(),
                    osc8_id: "scroll-id".to_owned(),
                    params: "id=scroll-id".to_owned(),
                }],
                colors: local::TerminalColorSummary {
                    default_fg_rgba: 5,
                    default_bg_rgba: 6,
                    cursor_rgba: 7,
                    cursor_rgba_set: true,
                    palette_rgba: vec![8, 9],
                    palette_diff_start: Some(1),
                    palette_diff_rgba: vec![10],
                },
                lines: vec![local::ScrollbackLine {
                    line: 1,
                    text: "older row".to_owned(),
                    runs: vec![local::CellRunSummary {
                        text: "older".to_owned(),
                        cell_widths: vec![1, 1, 1, 1, 1],
                        style_id: 0,
                        flags: 1,
                        hyperlink_id: 8,
                        semantic_content: protocol::CellSemanticContent::Input,
                    }],
                    dirty_hash: 12,
                    row_state_hash: 13,
                    semantic_prompt: protocol::RowSemanticPrompt::Prompt,
                    dirty: true,
                    kitty_virtual_placeholder: false,
                }],
            }),
        };

        let json = format_rendered_attach_json(&rendered);
        assert!(json.contains("\"session_id\":\"session\\\"1\""));
        assert!(json.contains("\"surface_version\":17"));
        assert!(json.contains("\"surface_state\":\"snapshot\""));
        assert!(json.contains("\"resize_policy\":\"active-client\""));
        assert!(json.contains("\"title\":\"build\\nshell\""));
        assert!(json.contains("\"surface_kind\":\"alternate\""));
        assert!(json.contains("\"cursor\":{\"row\":3,\"col\":4,\"visible\":true,\"shape\":\"beam\",\"blinking\":false}"));
        assert!(json.contains("\"bracketed_paste\":true"));
        assert!(json.contains("\"mouse_tracking_mode\":\"any\""));
        assert!(json.contains("\"mouse_format\":\"sgr-pixels\""));
        assert!(json.contains("\"surface\":{\"pane_id\":\"pane-1\",\"version\":17"));
        assert!(json.contains("\"row_updates\":[{\"row\":0,\"text\":\"hello\""));
        assert!(json.contains("\"dirty_hash\":41"));
        assert!(json.contains("\"uri\":\"https://example.test/surface\""));
        assert!(json.contains("\"surface_text\":\"hello\\nworld\""));
        assert!(json.contains("\"scrollback_version\":9"));
        assert!(json.contains("\"palette_rgba\":[8,9]"));
        assert!(json.contains("\"styles\":[{\"fg_rgba\":1"));
        assert!(json.contains("\"hyperlinks\":[{\"id\":8,\"uri\":\"https://example.test/scroll\""));
        assert!(json.contains("\"line\":1,\"text\":\"older row\""));
        assert!(json.contains("\"semantic_prompt\":\"prompt\""));
        assert!(json.contains("\"semantic_content\":\"input\""));
    }

    #[test]
    fn live_json_events_report_attach_workspace_and_surface_updates() {
        let workspace = local::WorkspaceSummary {
            session_id: "local".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            cols: 80,
            rows: 24,
            resize_policy: protocol::ResizePolicy::Fixed,
        };
        let rendered = local::RenderedAttach {
            workspace: workspace.clone(),
            status: local::AttachStatusSummary {
                pane_id: "pane-1".to_owned(),
                surface_version: 7,
                surface_state: protocol::AttachSurfaceState::Current,
            },
            surface_metadata: local::TerminalMetadataSummary::default(),
            surface_kind: protocol::SurfaceKind::Main,
            cursor: None,
            modes: local::TerminalModeSummary::default(),
            surface: local::RenderedSurfaceSummary {
                pane_id: "pane-1".to_owned(),
                version: 7,
                cols: 80,
                rows: 24,
                colors: local::TerminalColorSummary::default(),
                styles: Vec::new(),
                hyperlinks: Vec::new(),
                row_updates: Vec::new(),
            },
            surface_text: Some("initial".to_owned()),
            scrollback: None,
        };
        let mut update = test_surface_update(
            local::SurfaceUpdateKind::Patch,
            Some(protocol::PatchKind::ModeOnly),
        );
        update.text = "updated".to_owned();
        update.styles.push(local::StyleSummary {
            fg_rgba: 0xff00_00ff,
            bg_rgba: 0x0000_00ff,
            underline_rgba: 0,
            flags: 3,
        });
        update.hyperlinks.push(local::HyperlinkSummary {
            id: 4,
            uri: "https://example.test/live".to_owned(),
            osc8_id: "osc-id".to_owned(),
            params: "id=osc-id".to_owned(),
        });
        update.row_updates.push(local::SurfaceRowUpdate {
            row: 2,
            text: "styled".to_owned(),
            runs: vec![local::CellRunSummary {
                text: "sty".to_owned(),
                cell_widths: vec![1, 1, 1],
                style_id: 1,
                flags: 2,
                hyperlink_id: 4,
                semantic_content: protocol::CellSemanticContent::Prompt,
            }],
            dirty_hash: 11,
            row_state_hash: 12,
            semantic_prompt: protocol::RowSemanticPrompt::Continuation,
            dirty: true,
            kitty_virtual_placeholder: false,
        });

        assert!(format_live_attach_json(&rendered).starts_with("{\"event\":\"attach\""));
        assert_eq!(
            format_live_workspace_json(&workspace),
            "{\"event\":\"workspace\",\"workspace\":{\"session_id\":\"local\",\"tab_id\":\"tab-1\",\"pane_id\":\"pane-1\",\"cols\":80,\"rows\":24,\"resize_policy\":\"fixed\"}}"
        );
        let update_json = format_live_surface_update_json(
            &workspace,
            &local::TerminalMetadataSummary::default(),
            "updated",
            &update,
        );
        assert!(update_json.contains("\"event\":\"surface\""));
        assert!(update_json.contains("\"kind\":\"patch\""));
        assert!(update_json.contains("\"base_version\":6"));
        assert!(update_json.contains("\"patch_kind\":\"mode-only\""));
        assert!(update_json.contains("\"rows\":[{\"row\":2,\"text\":\"styled\""));
        assert!(update_json.contains("\"semantic_prompt\":\"continuation\""));
        assert!(update_json.contains("\"cell_widths\":[1,1,1]"));
        assert!(update_json.contains("\"semantic_content\":\"prompt\""));
        assert!(update_json.contains("\"styles\":[{\"fg_rgba\":4278190335"));
        assert!(
            update_json.contains("\"hyperlinks\":[{\"id\":4,\"uri\":\"https://example.test/live\"")
        );

        let error_json = format_live_error_json(&local::ErrorSummary {
            code: protocol::ErrorCode::PermissionDenied,
            message: "input rejected".to_owned(),
            retryable: false,
            pane_id: Some("pane-1".to_owned()),
            input_seq: 3,
        });
        assert_eq!(
            error_json,
            "{\"event\":\"error\",\"error\":{\"code\":\"permission-denied\",\"message\":\"input rejected\",\"retryable\":false,\"pane_id\":\"pane-1\",\"input_seq\":3}}"
        );
        assert_eq!(
            format_live_detach_json(LiveDetachReason::IterationLimit),
            "{\"event\":\"detach\",\"reason\":\"iteration-limit\"}"
        );
        assert_eq!(
            format_live_detach_json(LiveDetachReason::StdinEof),
            "{\"event\":\"detach\",\"reason\":\"stdin-eof\"}"
        );
        assert_eq!(
            format_live_detach_json(LiveDetachReason::LocalDetach),
            "{\"event\":\"detach\",\"reason\":\"local-detach\"}"
        );
        assert_eq!(
            format_live_detach_json(LiveDetachReason::ServerClosed),
            "{\"event\":\"detach\",\"reason\":\"server-closed\"}"
        );
        let server_error = local::ServerError {
            error: local::ErrorSummary {
                code: protocol::ErrorCode::PaneNotFound,
                message: "missing pane".to_owned(),
                retryable: false,
                pane_id: Some("pane-99".to_owned()),
                input_seq: 0,
            },
        };
        assert_eq!(
            format_cli_error_json(&server_error),
            "{\"error\":{\"code\":\"pane-not-found\",\"message\":\"missing pane\",\"retryable\":false,\"pane_id\":\"pane-99\",\"input_seq\":0}}"
        );
        assert_eq!(
            format_live_cli_error_json(&server_error),
            "{\"event\":\"error\",\"error\":{\"code\":\"pane-not-found\",\"message\":\"missing pane\",\"retryable\":false,\"pane_id\":\"pane-99\",\"input_seq\":0}}"
        );
        let io_error = std::io::Error::other("setup failed");
        assert_eq!(
            format_live_cli_error_json(&io_error),
            "{\"event\":\"error\",\"error\":{\"message\":\"setup failed\"}}"
        );
    }

    #[test]
    fn print_socket_json_arg_exits_before_mode_validation() {
        let args = args_from_iter([
            "--socket",
            "/tmp/nmux-json.sock",
            "--print-socket-json",
            "--cols",
            "80",
        ])
        .expect("args");
        assert!(args.print_socket_json);
        assert_eq!(args.socket_source, local::SocketPathSource::Explicit);
    }

    #[test]
    fn state_info_args_exit_before_mode_validation() {
        let args = args_from_iter(["--state", "/tmp/nmux.state", "--state-info", "--cols", "80"])
            .expect("args");
        assert!(args.state_info);
        let args = args_from_iter([
            "--state",
            "/tmp/nmux.state",
            "--state-info-json",
            "--cols",
            "80",
        ])
        .expect("args");
        assert!(args.state_info_json);
    }

    #[test]
    fn state_info_formatters_report_cached_objects() {
        let summary = local::ClientStateSummary {
            scope: Some(local::SocketIdentitySummary {
                dev: 1,
                ino: 2,
                ctime: 3,
                ctime_nsec: 4,
            }),
            surfaces: vec![local::ClientStateSurfaceSummary {
                pane_id: "pane-1".to_owned(),
                version: 7,
                cols: 80,
                rows: 24,
                surface_kind: protocol::SurfaceKind::Main,
                title: "shell".to_owned(),
                working_directory: "file://localhost/tmp".to_owned(),
                cursor: Some(local::CursorSummary {
                    row: 1,
                    col: 2,
                    visible: true,
                    shape: protocol::CursorShape::Beam,
                    blinking: false,
                }),
                modes: local::TerminalModeSummary::default(),
            }],
            scrollbacks: vec![local::ClientPaneScrollback {
                pane_id: "pane-1".to_owned(),
                version: 9,
                start_line: 6,
                line_count: 2,
                total_lines: 7,
            }],
        };
        let path = Path::new("/tmp/nmux.state");
        let socket_info = StateInfoSocketSummary {
            path: Path::new("/tmp/nmux.sock"),
            exists: true,
            scope_matches_socket: Some(true),
        };
        let text = format_state_info_text(path, true, &socket_info, &summary);
        assert!(text.contains("state=/tmp/nmux.state"));
        assert!(text.contains("exists=true"));
        assert!(text.contains("socket=/tmp/nmux.sock"));
        assert!(text.contains("socket_exists=true"));
        assert!(text.contains("scope_matches_socket=true"));
        assert!(text.contains("scope=socket dev=1 ino=2 ctime=3.4"));
        assert!(text.contains("surface pane=pane-1 version=7 size=80x24 kind=main"));
        assert!(text.contains("scrollback pane=pane-1 version=9 range=6..7 total=7"));
        let json = format_state_info_json(path, true, &socket_info, &summary);
        assert!(json.contains("\"path\":\"/tmp/nmux.state\""));
        assert!(json.contains("\"exists\":true"));
        assert!(json.contains("\"socket_path\":\"/tmp/nmux.sock\""));
        assert!(json.contains("\"socket_exists\":true"));
        assert!(json.contains("\"scope_matches_socket\":true"));
        assert!(json.contains("\"scope\":{\"kind\":\"socket\",\"dev\":1"));
        assert!(json.contains("\"surface_kind\":\"main\""));
        assert!(json.contains("\"scrollbacks\":[{\"pane_id\":\"pane-1\",\"version\":9"));
    }

    #[test]
    fn explicit_key_args_opt_into_text_input() {
        let args = args_from_iter(["--key", "ping\n"]).expect("args");
        assert_eq!(args.input_text.as_deref(), Some("ping\n"));
    }

    #[test]
    fn structured_input_args_do_not_require_live_mode() {
        let args =
            args_from_iter(["--key-name", "delete", "--key-modifiers", "ctrl"]).expect("args");
        assert_eq!(args.key_name.as_deref(), Some("delete"));
        assert_eq!(args.key_names, vec!["delete"]);
        assert_eq!(args.key_modifiers, 2);

        let args = args_from_iter(["--focus", "gained"]).expect("args");
        assert_eq!(args.focus_event, Some(FocusEvent::Gained));

        let args = args_from_iter(["--mouse", "press:left:1:2", "--mouse-modifiers", "alt"])
            .expect("args");
        assert_eq!(
            args.mouse_event,
            Some(MouseEvent {
                action: protocol::MouseAction::Press,
                button: protocol::MouseButton::Left,
                row: 0,
                col: 1,
                pixel_x: None,
                pixel_y: None,
                modifiers: 4,
            })
        );

        let args =
            args_from_iter(["--mouse", "press:left:1:2", "--mouse-pixels", "9:17"]).expect("args");
        assert_eq!(
            args.mouse_event,
            Some(MouseEvent {
                action: protocol::MouseAction::Press,
                button: protocol::MouseButton::Left,
                row: 0,
                col: 1,
                pixel_x: Some(9),
                pixel_y: Some(17),
                modifiers: 0,
            })
        );
    }

    #[test]
    fn repeated_key_name_args_preserve_sequence() {
        let args = args_from_iter(["--key-name", "esc", "--key-name", "return"]).expect("args");
        assert_eq!(args.key_name.as_deref(), Some("escape"));
        assert_eq!(args.key_names, vec!["escape", "enter"]);
    }

    #[test]
    fn live_update_print_kind_keeps_non_row_updates_quiet() {
        let previous = local::TerminalMetadataSummary {
            title: "old title".to_owned(),
            working_directory: "file://localhost/old".to_owned(),
        };
        let changed = local::TerminalMetadataSummary {
            title: "new title".to_owned(),
            working_directory: "file://localhost/new".to_owned(),
        };
        let cursor_only = test_surface_update(
            local::SurfaceUpdateKind::Patch,
            Some(protocol::PatchKind::CursorOnly),
        );
        let mode_only = test_surface_update(
            local::SurfaceUpdateKind::Patch,
            Some(protocol::PatchKind::ModeOnly),
        );
        let replace_rows = test_surface_update(
            local::SurfaceUpdateKind::Patch,
            Some(protocol::PatchKind::ReplaceRows),
        );
        let snapshot = test_surface_update(local::SurfaceUpdateKind::Snapshot, None);

        assert_eq!(
            live_update_print_kind(&previous, &previous, &cursor_only, false),
            LiveUpdatePrintKind::None
        );
        assert_eq!(
            live_update_print_kind(&previous, &changed, &cursor_only, false),
            LiveUpdatePrintKind::Metadata
        );
        assert_eq!(
            live_update_print_kind(&previous, &changed, &mode_only, false),
            LiveUpdatePrintKind::Metadata
        );
        assert_eq!(
            live_update_print_kind(&previous, &previous, &replace_rows, false),
            LiveUpdatePrintKind::Surface
        );
        assert_eq!(
            live_update_print_kind(&previous, &previous, &snapshot, false),
            LiveUpdatePrintKind::Surface
        );
        assert_eq!(
            live_update_print_kind(&previous, &previous, &cursor_only, true),
            LiveUpdatePrintKind::Surface
        );
    }

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
        let interactive_byte_mode = InterimSurfaceFidelityWarningContext {
            stdin_bytes: true,
            stdin_is_tty: true,
            stdout_is_tty: true,
        };

        assert!(interim_surface_fidelity_warning_needed(
            interactive_byte_mode
        ));
        assert!(!interim_surface_fidelity_warning_needed(
            InterimSurfaceFidelityWarningContext {
                stdout_is_tty: false,
                ..interactive_byte_mode
            }
        ));
        assert!(!interim_surface_fidelity_warning_needed(
            InterimSurfaceFidelityWarningContext {
                stdin_is_tty: false,
                ..interactive_byte_mode
            }
        ));
        assert!(!interim_surface_fidelity_warning_needed(
            InterimSurfaceFidelityWarningContext {
                stdin_bytes: false,
                ..interactive_byte_mode
            }
        ));
    }

    #[test]
    fn sigwinch_resize_is_only_needed_for_interactive_byte_mode_without_explicit_size() {
        let interactive_byte_mode = SigwinchResizeContext {
            stdin_bytes: true,
            explicit_resize: false,
            stdin_is_tty: true,
        };

        assert!(sigwinch_resize_needed(interactive_byte_mode));
        assert!(!sigwinch_resize_needed(SigwinchResizeContext {
            explicit_resize: true,
            ..interactive_byte_mode
        }));
        assert!(!sigwinch_resize_needed(SigwinchResizeContext {
            stdin_is_tty: false,
            ..interactive_byte_mode
        }));
        assert!(!sigwinch_resize_needed(SigwinchResizeContext {
            stdin_bytes: false,
            ..interactive_byte_mode
        }));
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
    fn focus_arg_accepts_explicit_choices() {
        assert_eq!(parse_focus_event("gained"), Ok(FocusEvent::Gained));
        assert_eq!(parse_focus_event("lost"), Ok(FocusEvent::Lost));
        assert!(parse_focus_event("blurred").is_err());
        assert!(FocusEvent::Gained.focused());
        assert!(!FocusEvent::Lost.focused());
    }

    #[test]
    fn key_name_arg_accepts_keypad_choices() {
        assert_eq!(
            parse_key_name("keypad-enter"),
            Ok("numpad-enter".to_owned())
        );
        assert_eq!(
            parse_key_name("numpad-enter"),
            Ok("numpad-enter".to_owned())
        );
        assert_eq!(parse_key_name("keypad-0"), Ok("numpad-0".to_owned()));
        assert_eq!(parse_key_name("kp-0"), Ok("numpad-0".to_owned()));
        assert_eq!(parse_key_name("numpad-0"), Ok("numpad-0".to_owned()));
        assert_eq!(parse_key_name("keypad-9"), Ok("numpad-9".to_owned()));
        assert_eq!(parse_key_name("kp-9"), Ok("numpad-9".to_owned()));
        assert_eq!(parse_key_name("numpad-9"), Ok("numpad-9".to_owned()));
        assert_eq!(parse_key_name("arrow-up"), Ok("arrow-up".to_owned()));
        assert_eq!(parse_key_name("up"), Ok("arrow-up".to_owned()));
        assert_eq!(parse_key_name("arrow-left"), Ok("arrow-left".to_owned()));
        assert_eq!(parse_key_name("enter"), Ok("enter".to_owned()));
        assert_eq!(parse_key_name("return"), Ok("enter".to_owned()));
        assert_eq!(parse_key_name("space"), Ok("space".to_owned()));
        assert_eq!(parse_key_name("esc"), Ok("escape".to_owned()));
        assert_eq!(parse_key_name("pgdn"), Ok("page-down".to_owned()));
        assert_eq!(parse_key_name("delete"), Ok("delete".to_owned()));
        assert_eq!(parse_key_name("page-down"), Ok("page-down".to_owned()));
        assert_eq!(parse_key_name("f12"), Ok("f12".to_owned()));
        assert!(parse_key_name("f13").is_err());
    }

    #[test]
    fn listed_key_names_parse_to_canonical_names() {
        for key_name in SUPPORTED_KEY_NAMES {
            assert_eq!(parse_key_name(key_name), Ok((*key_name).to_owned()));
        }
        for (alias, canonical) in KEY_NAME_ALIASES {
            assert_eq!(parse_key_name(alias), Ok((*canonical).to_owned()));
        }
    }

    #[test]
    fn key_modifiers_arg_accepts_named_modifier_bits() {
        assert_eq!(parse_key_modifiers("none"), Ok(0));
        assert_eq!(parse_key_modifiers("shift"), Ok(1));
        assert_eq!(parse_key_modifiers("ctrl"), Ok(2));
        assert_eq!(parse_key_modifiers("alt"), Ok(4));
        assert_eq!(parse_key_modifiers("super"), Ok(8));
        assert_eq!(parse_key_modifiers("ctrl+shift"), Ok(3));
        assert_eq!(parse_key_modifiers(" shift, alt "), Ok(5));
        assert_eq!(parse_key_modifiers("control+option+cmd"), Ok(14));
        assert!(parse_key_modifiers("").is_err());
        assert!(parse_key_modifiers("none+ctrl").is_err());
        assert!(parse_key_modifiers("hyper").is_err());
    }

    #[test]
    fn mouse_arg_accepts_action_button_and_one_based_cells() {
        assert_eq!(
            parse_mouse_event("press:left:1:2"),
            Ok(MouseEvent {
                action: protocol::MouseAction::Press,
                button: protocol::MouseButton::Left,
                row: 0,
                col: 1,
                pixel_x: None,
                pixel_y: None,
                modifiers: 0,
            })
        );
        assert_eq!(
            parse_mouse_event("release:none:24:80"),
            Ok(MouseEvent {
                action: protocol::MouseAction::Release,
                button: protocol::MouseButton::None,
                row: 23,
                col: 79,
                pixel_x: None,
                pixel_y: None,
                modifiers: 0,
            })
        );
        assert_eq!(parse_mouse_pixels("9:17"), Ok((9, 17)));
        assert!(parse_mouse_pixels("9").is_err());
        assert!(parse_mouse_pixels("x:17").is_err());
        assert!(parse_mouse_event("click:left:1:1").is_err());
        assert!(parse_mouse_event("press:left:0:1").is_err());
        assert!(parse_mouse_event("press:left:1").is_err());
    }

    #[test]
    fn mode_validation_rejects_ignored_or_conflicting_flags() {
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                live: true,
                follow: true,
                ..ClientModeArgs::default()
            }),
            Err("--follow cannot be combined with --live")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                follow: true,
                start: true,
                ..ClientModeArgs::default()
            }),
            Err("--start cannot be combined with --follow")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                start_command_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--command requires --start")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                start_working_dir_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--cwd requires --start")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                start_env_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--env requires --start")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                startup_timeout_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--startup-timeout-ms requires --start")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                follow: true,
                key_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--follow cannot be combined with --key")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                follow: true,
                paste_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--follow cannot be combined with --paste")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                follow: true,
                focus_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--follow cannot be combined with --focus")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                follow: true,
                key_name_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--follow cannot be combined with --key-name")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                follow: true,
                mouse_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--follow cannot be combined with --mouse")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                stdin_input: true,
                ..ClientModeArgs::default()
            }),
            Err("--stdin requires --live")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                stdin_bytes: true,
                ..ClientModeArgs::default()
            }),
            Err("--stdin-bytes requires --live")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                live: true,
                local_echo_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--local-echo requires --stdin-bytes")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                redraw: true,
                ..ClientModeArgs::default()
            }),
            Err("--redraw requires --live")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                live_resize: Some((80, 24)),
                ..ClientModeArgs::default()
            }),
            Err("--cols and --rows require --live")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                iterations: Some(1),
                ..ClientModeArgs::default()
            }),
            Err("--iterations requires --live or --follow")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                live: true,
                iterations: Some(0),
                ..ClientModeArgs::default()
            }),
            Err("--iterations must be greater than 0")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                key_modifiers_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--key-modifiers requires --key-name")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                mouse_modifiers_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--mouse-modifiers requires --mouse")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                mouse_pixels_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--mouse-pixels requires --mouse")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                follow: true,
                output_json: true,
                ..ClientModeArgs::default()
            }),
            Err("--json cannot be combined with --follow")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                live: true,
                redraw: true,
                output_json: true,
                ..ClientModeArgs::default()
            }),
            Err("--json cannot be combined with --redraw")
        );
        assert!(
            super_validate_mode_args(ClientModeArgs {
                live: true,
                iterations: Some(1),
                output_json: true,
                ..ClientModeArgs::default()
            })
            .is_ok()
        );
        assert!(
            super_validate_mode_args(ClientModeArgs {
                live: true,
                stdin_bytes: true,
                local_echo_set: true,
                redraw: true,
                live_resize: Some((80, 24)),
                iterations: Some(1),
                focus_set: true,
                key_name_set: true,
                ..ClientModeArgs::default()
            })
            .is_ok()
        );
        assert!(
            super_validate_mode_args(ClientModeArgs {
                focus_set: true,
                key_name_set: true,
                ..ClientModeArgs::default()
            })
            .is_ok()
        );
        assert!(
            super_validate_mode_args(ClientModeArgs {
                mouse_set: true,
                ..ClientModeArgs::default()
            })
            .is_ok()
        );
        assert!(
            super_validate_mode_args(ClientModeArgs {
                follow: true,
                iterations: Some(1),
                ..ClientModeArgs::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn input_mode_validation_rejects_explicit_conflicts() {
        let rejected = [
            (
                ExplicitInputModeArgs {
                    key_set: true,
                    no_input_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key cannot be combined with --no-input",
            ),
            (
                ExplicitInputModeArgs {
                    key_set: true,
                    key_name_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key cannot be combined with --key-name",
            ),
            (
                ExplicitInputModeArgs {
                    key_set: true,
                    paste_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key cannot be combined with --paste",
            ),
            (
                ExplicitInputModeArgs {
                    key_set: true,
                    focus_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key cannot be combined with --focus",
            ),
            (
                ExplicitInputModeArgs {
                    key_set: true,
                    mouse_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key cannot be combined with --mouse",
            ),
            (
                ExplicitInputModeArgs {
                    key_name_set: true,
                    paste_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key-name cannot be combined with --paste",
            ),
            (
                ExplicitInputModeArgs {
                    key_name_set: true,
                    focus_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key-name cannot be combined with --focus",
            ),
            (
                ExplicitInputModeArgs {
                    key_name_set: true,
                    mouse_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key-name cannot be combined with --mouse",
            ),
            (
                ExplicitInputModeArgs {
                    key_name_set: true,
                    no_input_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key-name cannot be combined with --no-input",
            ),
            (
                ExplicitInputModeArgs {
                    paste_set: true,
                    focus_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--paste cannot be combined with --focus",
            ),
            (
                ExplicitInputModeArgs {
                    paste_set: true,
                    mouse_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--paste cannot be combined with --mouse",
            ),
            (
                ExplicitInputModeArgs {
                    paste_set: true,
                    no_input_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--paste cannot be combined with --no-input",
            ),
            (
                ExplicitInputModeArgs {
                    focus_set: true,
                    no_input_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--focus cannot be combined with --no-input",
            ),
            (
                ExplicitInputModeArgs {
                    focus_set: true,
                    mouse_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--focus cannot be combined with --mouse",
            ),
            (
                ExplicitInputModeArgs {
                    mouse_set: true,
                    no_input_set: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--mouse cannot be combined with --no-input",
            ),
            (
                ExplicitInputModeArgs {
                    key_set: true,
                    stdin_input: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key cannot be combined with --stdin",
            ),
            (
                ExplicitInputModeArgs {
                    key_set: true,
                    stdin_bytes: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key cannot be combined with --stdin-bytes",
            ),
            (
                ExplicitInputModeArgs {
                    key_name_set: true,
                    stdin_input: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key-name cannot be combined with --stdin",
            ),
            (
                ExplicitInputModeArgs {
                    key_name_set: true,
                    stdin_bytes: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--key-name cannot be combined with --stdin-bytes",
            ),
            (
                ExplicitInputModeArgs {
                    paste_set: true,
                    stdin_input: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--paste cannot be combined with --stdin",
            ),
            (
                ExplicitInputModeArgs {
                    paste_set: true,
                    stdin_bytes: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--paste cannot be combined with --stdin-bytes",
            ),
            (
                ExplicitInputModeArgs {
                    focus_set: true,
                    stdin_input: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--focus cannot be combined with --stdin",
            ),
            (
                ExplicitInputModeArgs {
                    focus_set: true,
                    stdin_bytes: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--focus cannot be combined with --stdin-bytes",
            ),
            (
                ExplicitInputModeArgs {
                    mouse_set: true,
                    stdin_input: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--mouse cannot be combined with --stdin",
            ),
            (
                ExplicitInputModeArgs {
                    mouse_set: true,
                    stdin_bytes: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--mouse cannot be combined with --stdin-bytes",
            ),
            (
                ExplicitInputModeArgs {
                    no_input_set: true,
                    stdin_input: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--no-input cannot be combined with --stdin",
            ),
            (
                ExplicitInputModeArgs {
                    no_input_set: true,
                    stdin_bytes: true,
                    ..ExplicitInputModeArgs::default()
                },
                "--no-input cannot be combined with --stdin-bytes",
            ),
        ];
        for (args, error) in rejected {
            assert_eq!(super_validate_explicit_input_modes(args), Err(error));
        }

        let accepted = [
            ExplicitInputModeArgs {
                stdin_input: true,
                ..ExplicitInputModeArgs::default()
            },
            ExplicitInputModeArgs {
                stdin_bytes: true,
                ..ExplicitInputModeArgs::default()
            },
            ExplicitInputModeArgs {
                key_set: true,
                ..ExplicitInputModeArgs::default()
            },
            ExplicitInputModeArgs {
                key_name_set: true,
                ..ExplicitInputModeArgs::default()
            },
            ExplicitInputModeArgs {
                paste_set: true,
                ..ExplicitInputModeArgs::default()
            },
            ExplicitInputModeArgs {
                focus_set: true,
                ..ExplicitInputModeArgs::default()
            },
            ExplicitInputModeArgs {
                mouse_set: true,
                ..ExplicitInputModeArgs::default()
            },
        ];
        for args in accepted {
            assert!(super_validate_explicit_input_modes(args).is_ok());
        }
    }

    #[test]
    fn no_input_resize_validation_rejects_conflict() {
        assert_eq!(
            validate_no_input_resize_args(true, Some((80, 24))),
            Err("--no-input cannot be combined with --cols/--rows")
        );
        assert!(validate_no_input_resize_args(true, None).is_ok());
        assert!(validate_no_input_resize_args(false, Some((80, 24))).is_ok());
    }

    #[test]
    fn numeric_validation_rejects_zero_live_loop_values() {
        assert_eq!(
            validate_positive_numeric_args(1, 2, None, None, 0, None, 1),
            Err("--interval-ms must be greater than 0")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 2, None, Some((0, 24)), 1000, None, 1),
            Err("--cols and --rows must be between 1 and 65535")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 2, None, Some((80, 0)), 1000, None, 1),
            Err("--cols and --rows must be between 1 and 65535")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 2, None, Some((65536, 24)), 1000, None, 1),
            Err("--cols and --rows must be between 1 and 65535")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 2, None, Some((80, 65536)), 1000, None, 1),
            Err("--cols and --rows must be between 1 and 65535")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 2, None, None, 1000, Some(0), 1),
            Err("--connect-timeout-ms must be greater than 0")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 2, None, None, 1000, None, 0),
            Err("--startup-timeout-ms must be greater than 0")
        );
        assert!(validate_positive_numeric_args(1, 2, None, None, 1000, Some(1), 1).is_ok());
        assert!(validate_positive_numeric_args(1, 2, Some(1), None, 1000, None, 1).is_ok());
        assert!(validate_positive_numeric_args(1, 2, None, None, 1000, None, 1).is_ok());
        assert!(
            validate_positive_numeric_args(1, 2, None, Some((65535, 65535)), 1000, None, 1).is_ok()
        );
    }

    #[test]
    fn numeric_validation_rejects_zero_scrollback_values() {
        assert_eq!(
            validate_positive_numeric_args(0, 2, None, None, 1000, None, 1),
            Err("--scrollback-start must be greater than 0")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 0, None, None, 1000, None, 1),
            Err("--scrollback-count must be greater than 0")
        );
        assert_eq!(
            validate_positive_numeric_args(1, 2, Some(0), None, 1000, None, 1),
            Err("--scrollback-tail must be greater than 0")
        );
    }

    #[test]
    fn scrollback_selection_validation_rejects_ambiguous_tail_args() {
        let tail = ScrollbackSelectionArgFlags {
            scrollback_tail_set: true,
            ..ScrollbackSelectionArgFlags::default()
        };
        let range = ScrollbackSelectionArgFlags {
            scrollback_start_set: true,
            scrollback_count_set: true,
            ..ScrollbackSelectionArgFlags::default()
        };
        let no_scrollback = ScrollbackSelectionArgFlags {
            no_scrollback_set: true,
            ..ScrollbackSelectionArgFlags::default()
        };

        assert_eq!(
            validate_scrollback_selection_args(ScrollbackSelectionArgFlags {
                scrollback_start_set: true,
                ..tail
            }),
            Err("--scrollback-tail cannot be combined with --scrollback-start")
        );
        assert_eq!(
            validate_scrollback_selection_args(ScrollbackSelectionArgFlags {
                scrollback_count_set: true,
                ..tail
            }),
            Err("--scrollback-tail cannot be combined with --scrollback-count")
        );
        assert_eq!(
            validate_scrollback_selection_args(ScrollbackSelectionArgFlags {
                scrollback_tail_set: true,
                ..no_scrollback
            }),
            Err("--no-scrollback cannot be combined with --scrollback-tail")
        );
        assert_eq!(
            validate_scrollback_selection_args(ScrollbackSelectionArgFlags {
                scrollback_start_set: true,
                ..no_scrollback
            }),
            Err("--no-scrollback cannot be combined with --scrollback-start")
        );
        assert_eq!(
            validate_scrollback_selection_args(ScrollbackSelectionArgFlags {
                scrollback_count_set: true,
                ..no_scrollback
            }),
            Err("--no-scrollback cannot be combined with --scrollback-count")
        );
        assert!(validate_scrollback_selection_args(tail).is_ok());
        assert!(validate_scrollback_selection_args(range).is_ok());
        assert!(validate_scrollback_selection_args(no_scrollback).is_ok());
    }

    #[test]
    fn args_parse_scrollback_tail_count() {
        let args = args_from_iter(["--scrollback-tail", "5"]).expect("parse tail args");
        assert_eq!(args.scrollback_tail_count, Some(5));
        assert_eq!(args.scrollback_start_line, 1);
        assert_eq!(args.scrollback_line_count, 2);
    }

    #[test]
    fn no_scrollback_arg_skips_scrollback_fetch() {
        let args = args_from_iter(["--no-scrollback"]).expect("parse no-scrollback args");
        assert!(args.no_scrollback);
    }

    #[test]
    fn startup_timeout_arg_controls_managed_readiness_wait() {
        let args = args_from_iter(["--start", "--startup-timeout-ms", "123"])
            .expect("parse startup timeout args");
        assert_eq!(args.startup_timeout_ms, 123);
    }

    #[test]
    fn args_reject_scrollback_tail_with_explicit_range() {
        let err = match args_from_iter(["--scrollback-tail", "5", "--scrollback-start", "3"]) {
            Ok(_) => panic!("tail and start should conflict"),
            Err(err) => err.to_string(),
        };
        assert_eq!(
            err,
            "--scrollback-tail cannot be combined with --scrollback-start"
        );
        let err = match args_from_iter(["--scrollback-tail", "5", "--scrollback-count", "3"]) {
            Ok(_) => panic!("tail and count should conflict"),
            Err(err) => err.to_string(),
        };
        assert_eq!(
            err,
            "--scrollback-tail cannot be combined with --scrollback-count"
        );
    }

    #[test]
    fn numeric_args_report_flag_names_on_parse_errors() {
        let err = parse_numeric_arg::<u64>("--interval-ms", "slow".to_owned())
            .expect_err("invalid interval should include flag name");
        assert!(err.contains("--interval-ms requires a valid number"));

        let err = match args_from_iter(["--live", "--cols", "wide", "--rows", "24"]) {
            Ok(_) => panic!("invalid cols should include flag name"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("--cols requires a valid number"));

        let err = match args_from_iter(["--live", "--iterations", "many"]) {
            Ok(_) => panic!("invalid iterations should include flag name"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("--iterations requires a valid number"));
    }

    #[test]
    fn usage_mentions_live_interactive_flags() {
        let usage = usage();
        assert!(usage.contains("--stdin-bytes"));
        assert!(usage.contains("--print-context"));
        assert!(usage.contains("--print-context-json"));
        assert!(usage.contains("--print-socket"));
        assert!(usage.contains("--print-socket-json"));
        assert!(usage.contains("--state-info"));
        assert!(usage.contains("--state-info-json"));
        assert!(usage.contains("--start"));
        assert!(usage.contains("--command SHELL"));
        assert!(usage.contains("--cwd DIR"));
        assert!(usage.contains("--env KEY=VALUE"));
        assert!(usage.contains("--version-json"));
        assert!(usage.contains("-V, --version"));
        assert!(usage.contains("--connect-timeout-ms MS"));
        assert!(usage.contains("--local-echo off|tty"));
        assert!(usage.contains("--key-modifiers MODS"));
        assert!(usage.contains("--mouse-modifiers MODS"));
        assert!(usage.contains("--mouse-pixels X:Y"));
        assert!(usage.contains("--redraw"));
        assert!(usage.contains("--cols COUNT"));
        assert!(usage.contains("--start waits for nmuxd --ready-json"));
        assert!(usage.contains("interim text surface"));
    }

    #[test]
    fn scrollback_header_reports_returned_range() {
        let scrollback = scrollback_summary(4, 9, &[(4, "four"), (5, "five")]);
        assert_eq!(
            format_scrollback(&scrollback),
            "scrollback 4..5 of 9:\nfour\nfive\n"
        );

        let tail = scrollback_summary(4, 5, &[(4, "four"), (5, "five")]);
        assert_eq!(format_scrollback(&tail), "scrollback 4..5:\nfour\nfive\n");

        let empty = scrollback_summary(10, 5, &[]);
        assert_eq!(
            format_scrollback(&empty),
            "scrollback empty from 10 of 5:\n"
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

    fn scrollback_summary(
        start_line: u64,
        total_lines: u64,
        lines: &[(u64, &str)],
    ) -> local::ScrollbackChunkSummary {
        local::ScrollbackChunkSummary {
            pane_id: "pane-1".to_owned(),
            scrollback_version: 1,
            start_line,
            total_lines,
            styles: Vec::new(),
            hyperlinks: Vec::new(),
            colors: local::TerminalColorSummary::default(),
            lines: lines
                .iter()
                .map(|(line, text)| local::ScrollbackLine {
                    line: *line,
                    text: (*text).to_owned(),
                    runs: Vec::new(),
                    dirty_hash: 0,
                    row_state_hash: 0,
                    semantic_prompt: protocol::RowSemanticPrompt::None,
                    dirty: false,
                    kitty_virtual_placeholder: false,
                })
                .collect(),
        }
    }
}
