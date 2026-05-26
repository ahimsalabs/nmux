use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use crossterm::{
    cursor, execute,
    style::{Attribute, SetAttribute},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
    tty::IsTty,
};
use nmux_cli::{daemon, local};
use nmux_core::session::AttachMode;
use nmux_proto::protocol;
use ratatui::{
    TerminalOptions, Viewport,
    buffer::Buffer,
    layout::Rect,
    prelude::{CrosstermBackend, Terminal},
    style::{Color, Modifier, Style},
};

#[path = "nmux/tui.rs"]
mod tui;

const STDIN_BYTES_DETACH: u8 = 0x1d;
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
const DETACH_KEY_NAMES: &[&str] = &["ctrl-]", "none"];
const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";
const SGR_MOUSE_START: &[u8] = b"\x1b[<";
const DEFAULT_MANAGED_STARTUP_TIMEOUT_MS: u64 = 5000;
const DEFAULT_REMOTE_PORT: u16 = 7007;
const LIVE_RTT_PING_INTERVAL: Duration = Duration::from_secs(1);
const LIVE_RTT_PING_TIMEOUT: Duration = Duration::from_secs(5);
const STATUS_FPS_WINDOW: Duration = Duration::from_secs(2);

fn main() {
    if let Err(err) = run() {
        eprintln!("nmux: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    nmux_cli::observability::init_from_env()?;
    let raw_args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if let Some(result) = run_builtin_subcommand(&raw_args) {
        return result;
    }
    let args = args()?;
    if args.help {
        print!("{}", usage());
        return Ok(());
    }

    if args.version || args.version_json {
        let build = local::BuildInfo::current();
        if args.version_json {
            println!("{}", local::version_json("nmux", build));
        } else {
            println!("{}", build.version_line("nmux"));
        }
        return Ok(());
    }

    if args.list_key_names || args.list_key_names_json {
        print_key_names(output_format(args.list_key_names_json));
        return Ok(());
    }

    if args.list_input_choices_json {
        println!("{}", format_input_choices_json());
        return Ok(());
    }

    if args.print_context || args.print_context_json {
        if let Err(err) = print_context(output_format(args.print_context_json)) {
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

    if matches!(args.script_command, Some(ScriptCommand::Replay)) {
        return run_replay(&args);
    }

    if args.auto_default {
        return run_default(args);
    }

    if args.start {
        return run_managed(args);
    }

    if args.live {
        return run_live(&args);
    }

    match args.script_command {
        Some(ScriptCommand::SessionList) => return run_session_list(&args),
        Some(ScriptCommand::PaneList) => return run_pane_list(&args),
        Some(ScriptCommand::PaneSend) => return run_pane_send(&args),
        Some(
            ScriptCommand::PaneSplit
            | ScriptCommand::TabNew
            | ScriptCommand::TabSwitch
            | ScriptCommand::TabClose
            | ScriptCommand::SessionNew
            | ScriptCommand::SessionKill,
        ) => return run_control_command(&args),
        Some(ScriptCommand::TabList) => return run_tab_list(&args),
        _ => {}
    }

    run_attach_loop(&args)
}

fn run_default(mut args: Args) -> Result<(), Box<dyn std::error::Error>> {
    if !stdin_is_tty() {
        return run_attach_loop(&args);
    }
    configure_default_live_args(&mut args);
    if args.socket_path.exists() && default_daemon_needs_restart(&args) {
        replace_default_daemon_socket(&args);
    }
    if !args.socket_path.exists() {
        if let Err(err) = start_default_daemon(&args) {
            report_live_setup_error(&args, err.as_ref())?;
            return Err(err);
        }
        if wait_for_default_daemon_attach(&args).is_err() {
            replace_default_daemon_socket(&args);
            if let Err(err) = start_default_daemon(&args) {
                report_live_setup_error(&args, err.as_ref())?;
                return Err(err);
            }
            wait_for_default_daemon_attach(&args)?;
        }
    }
    match run_live(&args) {
        Ok(()) => Ok(()),
        Err(err) if default_live_error_needs_restart(err.as_ref()) => {
            replace_default_daemon_socket(&args);
            if let Err(start_err) = start_default_daemon(&args) {
                report_live_setup_error(&args, start_err.as_ref())?;
                return Err(start_err);
            }
            run_live(&args)
        }
        Err(err) => Err(err),
    }
}

fn configure_default_live_args(args: &mut Args) {
    args.live = true;
    args.stdin_bytes = true;
    args.redraw = true;
    args.no_scrollback = true;
    args.interval_ms = 16;
    if args.connect_timeout_ms.is_none() {
        args.connect_timeout_ms = Some(args.startup_timeout_ms);
    }
}

fn start_default_daemon(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|command| !command.trim().is_empty())
        .unwrap_or_else(|| "sh".to_owned());
    let command = format!("exec {} -i", shell_quote_for_sh(&shell));
    PersistentDaemon::start(
        &args.socket_path,
        args.target_session_id.as_deref(),
        &command,
        terminal_size()?,
        Duration::from_millis(args.startup_timeout_ms),
    )
    .map(|_| ())
}

fn default_daemon_needs_restart(args: &Args) -> bool {
    default_daemon_health_probe_needs_restart(args)
}

fn wait_for_default_daemon_attach(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_millis(args.startup_timeout_ms);
    let request = local::AttachRequest {
        actor_id: args.actor_id.clone(),
        user_id: args.user_id.clone(),
        display_name: args.display_name.clone(),
        mode: AttachMode::ReadOnly,
        focused_pane_id: args
            .target_pane_id
            .clone()
            .or_else(|| args.target_tab_id.clone()),
        known_surfaces: Vec::new(),
        hostname: resolve_short_hostname(),
        client_kind: "nmux".to_owned(),
        subscribe_client_inventory: false,
    };
    let mut last_error: Option<Box<dyn std::error::Error>> = None;
    while Instant::now() < deadline {
        match local::connect_to_daemon_with_timeout(&args.socket_path, Duration::from_millis(100)) {
            Ok(mut stream) => {
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                if let Err(err) = local::write_attach_request_for_session(
                    &mut stream,
                    &request,
                    args.target_session_id.as_deref(),
                ) {
                    last_error = Some(err.into());
                } else {
                    match local::attach_from_stream(&mut stream) {
                        Ok(_) => return Ok(()),
                        Err(err) => last_error = Some(err),
                    }
                }
            }
            Err(err) => last_error = Some(err),
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err(last_error.unwrap_or_else(|| "default daemon did not become attachable".into()))
}

fn default_daemon_health_probe_needs_restart(args: &Args) -> bool {
    let mut stream = match local::connect_to_daemon_with_timeout(
        &args.socket_path,
        Duration::from_millis(args.startup_timeout_ms),
    ) {
        Ok(stream) => stream,
        Err(_) => return true,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(args.startup_timeout_ms)));
    let probe = local::PresenceSummary {
        actor_id: args.actor_id.clone(),
        user_id: args.user_id.clone(),
        display_name: args.display_name.clone(),
        mode: AttachMode::ReadWrite,
        kind: protocol::PresenceKind::HealthProbe,
        focused_pane_id: args
            .target_pane_id
            .clone()
            .or_else(|| args.target_tab_id.clone()),
    };
    if local::write_health_probe(&mut stream, &probe).is_err() {
        return true;
    }
    match local::read_health_probe_response(&mut stream) {
        Ok(heartbeat) => heartbeat.kind != protocol::PresenceKind::Heartbeat,
        Err(err) => default_attach_error_needs_restart(err.as_ref()),
    }
}

fn default_attach_error_needs_restart(error: &(dyn std::error::Error + 'static)) -> bool {
    error
        .downcast_ref::<local::ServerError>()
        .is_some_and(|error| error_summary_needs_default_restart(&error.error))
}

fn error_summary_needs_default_restart(error: &local::ErrorSummary) -> bool {
    error.message.contains("pane process is not running")
}

fn default_live_error_needs_restart(error: &(dyn std::error::Error + 'static)) -> bool {
    error.to_string().contains("pane process is not running")
        || error.to_string().contains("failed to fill whole buffer")
        || error.to_string().contains("Broken pipe")
}

fn replace_default_daemon_socket(args: &Args) {
    let command = local::ControlCommandSummary {
        actor_id: args.actor_id.clone(),
        command_seq: 1,
        kind: protocol::ControlCommandKind::SessionKill,
        pane_id: None,
        tab_id: None,
        split_axis: protocol::SplitAxis::None,
        title: None,
        session_id: args.target_session_id.clone(),
    };
    let _ = local::run_control_command(
        &args.socket_path,
        Some(Duration::from_millis(args.startup_timeout_ms)),
        command,
    );
    let _ = fs::remove_file(&args.socket_path);
}

fn shell_quote_for_sh(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn attach_for_listing(args: &Args) -> Result<local::RenderedAttach, Box<dyn std::error::Error>> {
    let mut client_state = load_client_state(args.state_path.as_deref()).inspect_err(|err| {
        report_cli_error(args, err.as_ref()).ok();
    })?;
    attach_once(args, &mut client_state).inspect_err(|err| {
        report_cli_error(args, err.as_ref()).ok();
    })
}

fn run_session_list(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let inventory = fetch_session_inventory(args)?;
    if args.output_json {
        println!("{}", format_session_inventory_json(&inventory));
    } else {
        for session in inventory.sessions {
            let active = if session.session_id == inventory.active_session_id {
                " active"
            } else {
                ""
            };
            println!("{}{} {}", session.session_id, active, session.title);
        }
    }
    Ok(())
}

fn fetch_session_inventory(
    args: &Args,
) -> Result<local::SessionInventorySummary, Box<dyn std::error::Error>> {
    let command = local::ControlCommandSummary {
        actor_id: args.actor_id.clone(),
        command_seq: 1,
        kind: protocol::ControlCommandKind::SessionList,
        pane_id: None,
        tab_id: None,
        split_axis: protocol::SplitAxis::None,
        title: None,
        session_id: args.target_session_id.clone(),
    };
    let stream = connect_to_daemon(args)?;
    local::run_session_inventory_command_on_stream(stream, command)
}

fn run_tab_list(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let rendered = attach_for_listing(args)?;
    let tabs = workspace_tabs(&rendered.workspace);
    if args.output_json {
        let tabs_json = tabs
            .iter()
            .map(|tab| {
                format!(
                    "{{\"tab_id\":{},\"title\":{},\"active\":{}}}",
                    local::json_string(&tab.tab_id),
                    local::json_string(&tab.title),
                    tab.tab_id == rendered.workspace.tab_id
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        println!("{{\"tabs\":[{tabs_json}]}}");
    } else {
        for tab in tabs {
            let active = if tab.tab_id == rendered.workspace.tab_id {
                " active"
            } else {
                ""
            };
            println!("{}{} {}", tab.tab_id, active, tab.title);
        }
    }
    Ok(())
}

fn workspace_tabs(workspace: &local::WorkspaceSummary) -> Vec<local::WorkspaceTabSummary> {
    if !workspace.tabs.is_empty() {
        return workspace.tabs.clone();
    }
    let root = workspace
        .pane_tree
        .clone()
        .unwrap_or_else(|| local::WorkspacePaneSummary {
            pane_id: workspace.pane_id.clone(),
            cols: workspace.cols,
            rows: workspace.rows,
            resize_policy: workspace.resize_policy,
            split_axis: protocol::SplitAxis::None,
            children: Vec::new(),
        });
    vec![local::WorkspaceTabSummary {
        tab_id: workspace.tab_id.clone(),
        title: workspace.tab_id.clone(),
        active_pane_id: workspace.pane_id.clone(),
        root,
    }]
}

fn run_pane_list(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let rendered = attach_for_listing(args)?;
    let panes = workspace_panes(&rendered.workspace);
    if args.output_json {
        let panes_json = panes
            .iter()
            .map(|pane| {
                format!(
                    "{{\"pane_id\":{},\"active\":{},\"cols\":{},\"rows\":{},\"resize_policy\":{}}}",
                    local::json_string(&pane.pane_id),
                    pane.pane_id == rendered.workspace.pane_id,
                    pane.cols,
                    pane.rows,
                    local::json_string(resize_policy_name(pane.resize_policy))
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        println!("{{\"panes\":[{panes_json}]}}");
    } else {
        for pane in panes {
            let active = if pane.pane_id == rendered.workspace.pane_id {
                " active"
            } else {
                ""
            };
            println!(
                "{}{} {}x{} resize={}",
                pane.pane_id,
                active,
                pane.cols,
                pane.rows,
                resize_policy_name(pane.resize_policy)
            );
        }
    }
    Ok(())
}

fn workspace_panes(workspace: &local::WorkspaceSummary) -> Vec<local::WorkspacePaneSummary> {
    let Some(root) = workspace.pane_tree.as_ref() else {
        return vec![local::WorkspacePaneSummary {
            pane_id: workspace.pane_id.clone(),
            cols: workspace.cols,
            rows: workspace.rows,
            resize_policy: workspace.resize_policy,
            split_axis: protocol::SplitAxis::None,
            children: Vec::new(),
        }];
    };
    let mut panes = Vec::new();
    collect_leaf_panes(root, &mut panes);
    panes
}

fn collect_leaf_panes(
    pane: &local::WorkspacePaneSummary,
    panes: &mut Vec<local::WorkspacePaneSummary>,
) {
    if pane.children.is_empty() {
        panes.push(pane.clone());
        return;
    }
    for child in &pane.children {
        collect_leaf_panes(child, panes);
    }
}

fn run_builtin_subcommand(
    raw_args: &[std::ffi::OsString],
) -> Option<Result<(), Box<dyn std::error::Error>>> {
    let first = raw_args.first()?.to_str()?;
    match first {
        "daemon" => {
            let argv = std::iter::once(std::ffi::OsString::from("nmux daemon"))
                .chain(raw_args.iter().skip(1).cloned());
            Some(daemon::run_from_iter(argv))
        }
        "version" => {
            let build = local::BuildInfo::current();
            if raw_args.len() == 1 {
                println!("{}", build.version_line("nmux"));
                Some(Ok(()))
            } else if raw_args.len() == 2 && raw_args[1].to_str() == Some("--json") {
                println!("{}", local::version_json("nmux", build));
                Some(Ok(()))
            } else {
                Some(Err("usage: nmux version [--json]".into()))
            }
        }
        _ => None,
    }
}

fn run_pane_send(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let mut client_state = load_client_state(args.state_path.as_deref()).inspect_err(|err| {
        report_cli_error(args, err.as_ref()).ok();
    })?;
    match attach_once(args, &mut client_state) {
        Ok(_) => save_client_state(args.state_path.as_deref(), &client_state),
        Err(err) => {
            report_cli_error(args, err.as_ref())?;
            Err(err)
        }
    }
}

fn run_control_command(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let command_kind = match args.script_command {
        Some(ScriptCommand::PaneSplit) => protocol::ControlCommandKind::PaneSplit,
        Some(ScriptCommand::TabNew) => protocol::ControlCommandKind::TabNew,
        Some(ScriptCommand::TabSwitch) => protocol::ControlCommandKind::TabSwitch,
        Some(ScriptCommand::TabClose) => protocol::ControlCommandKind::TabClose,
        Some(ScriptCommand::SessionNew) => protocol::ControlCommandKind::SessionNew,
        Some(ScriptCommand::SessionKill) => protocol::ControlCommandKind::SessionKill,
        _ => return Err("missing control command".into()),
    };
    let command = local::ControlCommandSummary {
        actor_id: args.actor_id.clone(),
        command_seq: 1,
        kind: command_kind,
        pane_id: args.target_pane_id.clone(),
        tab_id: args.target_tab_id.clone(),
        split_axis: args.script_split_axis,
        title: args.script_title.clone(),
        session_id: args.target_session_id.clone(),
    };
    let result = if args.tcp_endpoint.is_some() {
        connect_to_daemon(args)
            .and_then(|stream| local::run_control_command_on_stream(stream, command))
    } else {
        local::run_control_command(&args.socket_path, connect_timeout_duration(args), command)
    };
    match result {
        Ok(workspace) => {
            if args.output_json {
                println!("{{\"workspace\":{}}}", format_workspace_json(&workspace));
            } else if args.script_command == Some(ScriptCommand::SessionKill) {
                return Ok(());
            } else {
                println!("{}", workspace.display_line());
            }
            Ok(())
        }
        Err(err) => {
            report_cli_error(args, err.as_ref())?;
            Err(err)
        }
    }
}

fn run_replay(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let path = args.replay_path.as_deref().ok_or("missing replay path")?;
    let file = fs::File::open(path)
        .map_err(|err| format!("failed to open replay file {}: {err}", path.display()))?;
    let mut rendered_any = false;
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|err| {
            format!(
                "failed to read replay file {} line {}: {err}",
                path.display(),
                index + 1
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line).map_err(|err| {
            format!(
                "failed to parse replay file {} line {} as JSON: {err}",
                path.display(),
                index + 1
            )
        })?;
        let event = value.get("event").and_then(serde_json::Value::as_str);
        let surface_text = match event {
            Some("attach") => value
                .pointer("/attach/surface_text")
                .and_then(serde_json::Value::as_str),
            Some("surface") => value
                .get("surface_text")
                .and_then(serde_json::Value::as_str),
            _ => None,
        };
        if let Some(surface_text) = surface_text {
            if rendered_any {
                println!();
            }
            print!("{surface_text}");
            if !surface_text.ends_with('\n') {
                println!();
            }
            rendered_any = true;
        }
    }
    flush_stdout()?;
    Ok(())
}

fn run_attach_loop(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let mut client_state = match load_client_state(args.state_path.as_deref()) {
        Ok(state) => state,
        Err(err) => {
            report_cli_error(args, err.as_ref())?;
            return Err(err);
        }
    };

    let iterations = if args.follow {
        args.iterations.unwrap_or(usize::MAX)
    } else {
        1
    };

    for iteration in 0..iterations {
        let rendered = match attach_once(args, &mut client_state) {
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
            report_cli_error(args, err.as_ref())?;
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
    let mut recorder = match LiveRecorder::open(args.record_path.as_deref()) {
        Ok(recorder) => recorder,
        Err(err) => {
            report_live_setup_error(args, &err)?;
            return Err(err.into());
        }
    };
    let _raw_terminal = match RawTerminalGuard::enable_if_needed(
        RawTerminalModeContext {
            stdin_bytes: args.stdin_bytes,
            stdin_is_tty: stdin_is_tty(),
        },
        args.local_echo,
    ) {
        Ok(guard) => guard,
        Err(err) => {
            report_live_setup_error(args, &err)?;
            return Err(err.into());
        }
    };
    let _redraw_terminal = match RedrawTerminalGuard::enable_if_needed(RedrawTerminalContext {
        redraw: args.redraw,
        stdout_is_tty: stdout_is_tty(),
    }) {
        Ok(guard) => guard,
        Err(err) => {
            report_live_setup_error(args, &err)?;
            return Err(err.into());
        }
    };
    warn_if_interim_surface_fidelity_is_visible(args.stdin_bytes);
    let mut sigwinch_resize = match SigwinchResize::enable_if_needed(SigwinchResizeContext {
        stdin_bytes: args.stdin_bytes,
        explicit_resize: args.live_resize.is_some(),
        terminal_is_tty: stdin_is_tty() || stdout_is_tty(),
    }) {
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
    let live_poll_timeout = Duration::from_millis(args.interval_ms);
    let live_socket_read_timeout = live_poll_timeout;
    let stdout_tty = stdout_is_tty();
    let post_input_stream_grace = if stdout_tty && stdin_bytes_speculative_echo_enabled(args) {
        Duration::ZERO
    } else if args.stdin_bytes && !stdin_is_tty() {
        live_poll_timeout
    } else {
        live_poll_timeout.min(Duration::from_millis(2))
    };
    let setup_read_timeout = connect_timeout_duration(args)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_MANAGED_STARTUP_TIMEOUT_MS));
    if let Err(err) = stream.set_read_timeout(Some(setup_read_timeout)) {
        report_live_setup_error(args, &err)?;
        return Err(err.into());
    }
    let stdin_lines = if args.stdin_input {
        Some(spawn_stdin_line_reader())
    } else {
        None
    };
    let stdin_bytes = if args.stdin_bytes {
        Some(spawn_stdin_byte_reader()?)
    } else {
        None
    };
    let mut stdin_closed = false;
    let mut stdin_bytes_closed = false;
    let mut detach_requested = false;
    let mut client_sequence = local::ClientFrameSequence::default();
    let mut speculative_echo = local::SpeculativeEchoOverlay::default();
    let mut client_inventory = ClientInventoryCache::default();

    let mut options = local::AttachOptions {
        target_session_id: args.target_session_id.clone(),
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
    apply_client_identity(args, &mut options.request);
    options.request.hostname = resolve_short_hostname();
    options.request.client_kind = "nmux".to_owned();
    options.request.subscribe_client_inventory = args.redraw && stdout_is_tty();
    options.request.focused_pane_id = args
        .target_pane_id
        .clone()
        .or_else(|| args.target_tab_id.clone());
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
    if let Err(err) = local::write_attach_request_for_session(
        &mut stream,
        &options.request,
        args.target_session_id.as_deref(),
    ) {
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
    let initial_presence = snapshot.presence.clone();
    let mut attached_pane_id = snapshot.status.pane_id.clone();
    client_state.apply_scope(local::socket_identity(&args.socket_path).ok());
    let mut rendered = match client_state.render_attach(snapshot) {
        Ok(rendered) => rendered,
        Err(err) => {
            report_live_setup_error(args, err.as_ref())?;
            return Err(err);
        }
    };
    if let Err(err) = validate_target_session(args, &rendered.workspace.session_id) {
        report_live_setup_error(args, err.as_ref())?;
        return Err(err);
    }
    // Styled ANSI SGR output is only useful on real terminals. When stdout
    // is captured (tests, pipes), emit plain text for compatibility.
    let use_styled = stdout_is_tty();
    // Re-render with styles if the engine produced structured style data
    // and we're outputting to a real terminal.
    if use_styled && rendered.surface_text.is_some() {
        rendered.surface_text = client_state.cached_surface_text_styled(&attached_pane_id, true);
    }
    if rendered.surface_text.is_none() {
        rendered.surface_text =
            client_state.cached_surface_text_styled(&attached_pane_id, use_styled);
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
    let initial_surface_text = rendered
        .surface_text
        .clone()
        .unwrap_or_else(|| current_workspace.display_line());
    let mut initial_pane_surfaces = BTreeMap::new();
    initial_pane_surfaces.insert(attached_pane_id.clone(), initial_surface_text.clone());
    seed_cached_pane_surfaces(
        &mut initial_pane_surfaces,
        &current_workspace,
        &client_state,
        use_styled,
    );
    let mut initial_pane_surface_summaries = BTreeMap::new();
    initial_pane_surface_summaries.insert(attached_pane_id.clone(), rendered.surface.clone());
    seed_cached_pane_surface_summaries(
        &mut initial_pane_surface_summaries,
        &current_workspace,
        &client_state,
    );
    let mut initial_pane_modes = BTreeMap::new();
    initial_pane_modes.insert(attached_pane_id.clone(), rendered.modes);
    seed_cached_pane_modes(&mut initial_pane_modes, &current_workspace, &client_state);
    let mut surface_state = LiveSurfaceState {
        current_surface_metadata: rendered.surface_metadata.clone(),
        current_modes: rendered.modes,
        current_surface_text: initial_surface_text,
        current_pane_surfaces: initial_pane_surfaces,
        current_pane_surface_summaries: initial_pane_surface_summaries,
        current_pane_modes: initial_pane_modes,
        scrollback_views: BTreeMap::new(),
    };
    let (scrollback, pending_surface_updates, pending_live_reads) = match initial_live_scrollback(
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
    for pending in pending_live_reads {
        match pending {
            local::LiveSurfaceRead::ClientInventorySnapshot(snapshot) => {
                client_inventory.apply_snapshot(snapshot);
            }
            local::LiveSurfaceRead::ClientInventoryPatch(patch) => {
                let _ = client_inventory.apply_patch(patch);
            }
            _ => {}
        }
    }
    if let Some(scrollback) = scrollback.as_ref() {
        client_state.cache_scrollback_chunk(scrollback);
    }
    if args.live_resize.is_none()
        && let Some((cols, rows)) = sigwinch_resize.current_resize()?
    {
        let (pane_cols, pane_rows) = frontend_resize_pane_size(
            &current_workspace,
            &attached_pane_id,
            cols,
            rows,
            args.redraw && stdout_tty,
        );
        local::send_resize_intent_with_reason_and_sequence(
            &mut stream,
            &mut client_sequence,
            &attached_pane_id,
            pane_cols,
            pane_rows,
            protocol::ResizeReason::FrontendViewport,
        )?;
        apply_frontend_workspace_size(
            &mut current_workspace,
            &attached_pane_id,
            pane_cols,
            pane_rows,
        );
        recorder.record(&format_live_workspace_json(&current_workspace))?;
        let _ = stream.set_read_timeout(Some(live_socket_read_timeout));
        loop {
            if let Some(reader) = stdin_bytes.as_ref() {
                match poll_live_stream_or_stdin(
                    &stream,
                    reader.wake_reader(),
                    live_poll_timeout,
                    false,
                )? {
                    LiveLoopReadiness::Stdin | LiveLoopReadiness::Timeout => break,
                    LiveLoopReadiness::Stream => {}
                }
            }
            match local::read_live_surface_update_from_stream(&mut stream)? {
                local::LiveSurfaceRead::Workspace(workspace) => {
                    current_workspace = workspace;
                    preserve_live_client_focus(&mut current_workspace, &attached_pane_id);
                    recorder.record(&format_live_workspace_json(&current_workspace))?;
                }
                local::LiveSurfaceRead::Update(update) => {
                    speculative_echo.reconcile_update(&update);
                    let update_metadata = local::TerminalMetadataSummary {
                        title: update.title.clone(),
                        working_directory: update.working_directory.clone(),
                    };
                    let update_surface_text =
                        client_state.render_surface_update_styled(&update, use_styled)?;
                    surface_state
                        .current_pane_surfaces
                        .insert(update.pane_id.clone(), update_surface_text.clone());
                    if let Some(summary) =
                        client_state.cached_rendered_surface_summary(&update.pane_id)
                    {
                        surface_state
                            .current_pane_surface_summaries
                            .insert(update.pane_id.clone(), summary);
                    }
                    surface_state
                        .current_pane_modes
                        .insert(update.pane_id.clone(), update.modes);
                    surface_state.scrollback_views.remove(&update.pane_id);
                    if update.pane_id == current_workspace.pane_id {
                        surface_state.current_surface_metadata = update_metadata;
                        surface_state.current_modes = update.modes;
                        surface_state.current_surface_text = update_surface_text;
                    }
                }
                local::LiveSurfaceRead::Presence(presence) => {
                    recorder.record(&format_live_presence_json(&presence))?;
                }
                local::LiveSurfaceRead::ClientInventorySnapshot(snapshot) => {
                    client_inventory.apply_snapshot(snapshot);
                }
                local::LiveSurfaceRead::ClientInventoryPatch(patch) => {
                    let _ = client_inventory.apply_patch(patch);
                }
                local::LiveSurfaceRead::Pong(_) => {}
                local::LiveSurfaceRead::Error(error) => {
                    recorder.record(&format_live_error_json(&error))?;
                    return Err(format!("live server error: {error}").into());
                }
                local::LiveSurfaceRead::NoFrame | local::LiveSurfaceRead::Closed => break,
            }
        }
        let _ = stream.set_read_timeout(Some(setup_read_timeout));
        rendered.workspace = current_workspace.clone();
        rendered.surface_metadata = surface_state.current_surface_metadata.clone();
        rendered.surface_text = Some(surface_state.current_surface_text.clone());
        rendered.modes = surface_state.current_modes;
    }
    // Differential rendering with latency overlay is only useful on real
    // terminals. When stdout is captured (tests, pipes), fall back to the
    // legacy full-screen-clear path so output is plain text.
    let mut redraw_state = if args.redraw && stdout_tty {
        Some(RedrawState::new_with_terminal()?)
    } else {
        None
    };
    if let Some(state) = redraw_state.as_mut()
        && client_inventory.count() > 0
    {
        state.record_client_count(client_inventory.count());
    }
    let mut rtt_tracker = redraw_state
        .as_ref()
        .map(|_| RttTracker::new(options.request.actor_id.clone()));
    let mut host_mouse_modes = HostMouseModeMirror::enable_if_needed(HostMouseModeContext {
        stdin_bytes: args.stdin_bytes,
        redraw: args.redraw,
        stdout_is_tty: stdout_is_tty(),
    })?;
    if args.output_json {
        rendered.scrollback = scrollback;
        let event = format_live_attach_json(&rendered);
        recorder.record(&event)?;
        recorder.record(&format_live_presence_json(&initial_presence))?;
        println!("{event}");
    } else {
        recorder.record(&format_live_attach_json(&rendered))?;
        recorder.record(&format_live_presence_json(&initial_presence))?;
        if let Some(mouse_modes) = host_mouse_modes.as_mut() {
            mouse_modes.sync(surface_state.current_modes)?;
        }
        print_live_rendered(
            rendered,
            args.redraw,
            scrollback,
            redraw_state.as_mut(),
            Some(&surface_state.current_pane_surfaces),
            Some(&surface_state.current_pane_surface_summaries),
        );
    }
    flush_stdout()?;

    for update in pending_surface_updates {
        process_surface_update(
            &update,
            &mut surface_state,
            &mut speculative_echo,
            &mut client_state,
            &mut host_mouse_modes,
            &mut redraw_state,
            &mut recorder,
            &current_workspace,
            args,
            use_styled,
        )?;
    }
    if let Err(err) = stream.set_read_timeout(Some(live_socket_read_timeout)) {
        report_live_setup_error(args, &err)?;
        return Err(err.into());
    }

    let cycle_limit = args.iterations.or_else(|| {
        (!args.stdin_input && !args.stdin_bytes && options.request.mode == AttachMode::ReadWrite)
            .then_some(1)
    });
    let mut cycles = 0;
    let mut sent_explicit_live_resize = false;
    let mut active_overlay: Option<tui::TuiOverlay> = None;
    let mut active_menu_index: Option<usize> = None;
    let detach_reason = loop {
        if cycle_limit.is_some_and(|iterations| cycles >= iterations) {
            break LiveDetachReason::IterationLimit;
        }

        if let Some(tracker) = rtt_tracker.as_mut() {
            tracker.maybe_send_ping(&mut stream, &mut client_sequence)?;
        }

        let mut sent_stdin_bytes_this_cycle = false;
        let mut read_after_stdin_bytes_this_cycle = false;
        if options.request.mode == AttachMode::ReadWrite {
            if let Some((cols, rows)) = args.live_resize {
                if !sent_explicit_live_resize {
                    local::send_resize_intent_with_reason_and_sequence(
                        &mut stream,
                        &mut client_sequence,
                        &attached_pane_id,
                        cols,
                        rows,
                        protocol::ResizeReason::UserCommand,
                    )?;
                    sent_explicit_live_resize = true;
                }
            } else if let Some((cols, rows)) = sigwinch_resize.next_resize()? {
                let (pane_cols, pane_rows) = frontend_resize_pane_size(
                    &current_workspace,
                    &attached_pane_id,
                    cols,
                    rows,
                    args.redraw && stdout_tty,
                );
                local::send_resize_intent_with_reason_and_sequence(
                    &mut stream,
                    &mut client_sequence,
                    &attached_pane_id,
                    pane_cols,
                    pane_rows,
                    protocol::ResizeReason::FrontendViewport,
                )?;
                apply_frontend_workspace_size(
                    &mut current_workspace,
                    &attached_pane_id,
                    pane_cols,
                    pane_rows,
                );
                let event = format_live_workspace_json(&current_workspace);
                recorder.record(&event)?;
                if args.output_json {
                    println!("{event}");
                } else if args.redraw {
                    print_live_surface_with_overlay(
                        &current_workspace,
                        &surface_state.current_surface_metadata,
                        &surface_state.current_surface_text,
                        args.redraw,
                        redraw_state.as_mut(),
                        Some(&surface_state.current_pane_surfaces),
                        Some(&surface_state.current_pane_surface_summaries),
                        active_overlay.as_ref(),
                    );
                } else {
                    println!("{}", current_workspace.display_line());
                }
                flush_stdout()?;
            }
            let input_text = if let Some(receiver) = stdin_bytes.as_ref() {
                match receiver.try_recv() {
                    Ok(StdinByteRead::Input(input)) => {
                        let (input, detach) =
                            split_stdin_bytes_for_detach(&input, args.detach_key.byte());
                        if let Some(input) = input {
                            for forward in stdin_byte_forwards(&input) {
                                match forward {
                                    StdinByteForward::Raw(input) => {
                                        let input_span = tracing::trace_span!(
                                            "live.stdin_bytes.forward_input",
                                            bytes = input.len(),
                                            pane_id = %attached_pane_id
                                        );
                                        let input_seq = input_span.in_scope(|| {
                                            local::send_raw_input_with_sequence(
                                                &mut stream,
                                                &mut client_sequence,
                                                &attached_pane_id,
                                                &input,
                                            )
                                        })?;
                                        if let Ok(text) = std::str::from_utf8(&input) {
                                            repaint_speculative_echo(
                                                stdin_bytes_speculative_echo_enabled(args),
                                                &client_state,
                                                &mut speculative_echo,
                                                &attached_pane_id,
                                                input_seq,
                                                text,
                                                &current_workspace,
                                                &surface_state.current_surface_metadata,
                                                &mut surface_state.current_surface_text,
                                                &mut redraw_state,
                                                use_styled,
                                            )?;
                                        }
                                    }
                                    StdinByteForward::Paste(text) => {
                                        let input_span = tracing::trace_span!(
                                            "live.stdin_bytes.forward_paste",
                                            bytes = text.len(),
                                            pane_id = %attached_pane_id
                                        );
                                        input_span.in_scope(|| {
                                            local::send_paste_input_with_sequence(
                                                &mut stream,
                                                &mut client_sequence,
                                                &attached_pane_id,
                                                &text,
                                            )
                                        })?;
                                    }
                                    StdinByteForward::Key(key) => {
                                        match handle_live_tui_key(
                                            key,
                                            &mut active_menu_index,
                                            &mut active_overlay,
                                            &mut stream,
                                            &mut client_sequence,
                                            &mut attached_pane_id,
                                            &mut current_workspace,
                                            &mut surface_state,
                                            &mut client_state,
                                            &mut host_mouse_modes,
                                            &mut speculative_echo,
                                            &mut client_inventory,
                                            &mut recorder,
                                            socket_scope,
                                            &options,
                                            setup_read_timeout,
                                            live_socket_read_timeout,
                                            &mut redraw_state,
                                            args,
                                            use_styled,
                                        )? {
                                            LiveKeyHandling::Handled => {
                                                flush_stdout()?;
                                            }
                                            LiveKeyHandling::Forward(bytes) => {
                                                let input_span = tracing::trace_span!(
                                                    "live.stdin_bytes.forward_input",
                                                    bytes = bytes.len(),
                                                    pane_id = %attached_pane_id
                                                );
                                                let input_seq = input_span.in_scope(|| {
                                                    local::send_raw_input_with_sequence(
                                                        &mut stream,
                                                        &mut client_sequence,
                                                        &attached_pane_id,
                                                        &bytes,
                                                    )
                                                })?;
                                                if let Ok(text) = std::str::from_utf8(&bytes) {
                                                    repaint_speculative_echo(
                                                        stdin_bytes_speculative_echo_enabled(args),
                                                        &client_state,
                                                        &mut speculative_echo,
                                                        &attached_pane_id,
                                                        input_seq,
                                                        text,
                                                        &current_workspace,
                                                        &surface_state.current_surface_metadata,
                                                        &mut surface_state.current_surface_text,
                                                        &mut redraw_state,
                                                        use_styled,
                                                    )?;
                                                }
                                            }
                                        }
                                    }
                                    StdinByteForward::Mouse(mouse) => {
                                        match live_mouse_dispatch_for_workspace(
                                            mouse,
                                            &current_workspace,
                                            &surface_state.current_surface_text,
                                            Some(&surface_state.current_pane_surfaces),
                                            Some(&surface_state.current_pane_modes),
                                            surface_state.current_modes,
                                            active_overlay.as_ref(),
                                        ) {
                                            Some(LiveMouseDispatch::FocusPane(pane_id)) => {
                                                active_overlay = None;
                                                active_menu_index = None;
                                                if focus_live_client_pane(
                                                    &pane_id,
                                                    &mut attached_pane_id,
                                                    &mut current_workspace,
                                                    &mut surface_state,
                                                    &client_state,
                                                    host_mouse_modes.as_mut(),
                                                    redraw_state.as_mut(),
                                                    args,
                                                    use_styled,
                                                )? {
                                                    flush_stdout()?;
                                                }
                                            }
                                            Some(LiveMouseDispatch::PaneMouse(pane_id, mouse)) => {
                                                active_overlay = None;
                                                active_menu_index = None;
                                                local::send_mouse_input_with_sequence(
                                                    &mut stream,
                                                    &mut client_sequence,
                                                    &pane_id,
                                                    mouse,
                                                )?;
                                            }
                                            Some(LiveMouseDispatch::PaneScroll {
                                                pane_id,
                                                direction,
                                                visible_rows,
                                            }) => {
                                                active_overlay = None;
                                                active_menu_index = None;
                                                if scroll_live_pane_view(
                                                    &mut stream,
                                                    &mut client_sequence,
                                                    &pane_id,
                                                    direction,
                                                    visible_rows,
                                                    &mut surface_state,
                                                    &mut client_state,
                                                    &mut speculative_echo,
                                                    &mut host_mouse_modes,
                                                    &mut client_inventory,
                                                    &mut recorder,
                                                    socket_scope,
                                                    &current_workspace,
                                                    args,
                                                    redraw_state.as_mut(),
                                                    use_styled,
                                                )? {
                                                    flush_stdout()?;
                                                }
                                            }
                                            Some(LiveMouseDispatch::Menu(action)) => {
                                                active_menu_index = menu_index(action);
                                                if action == tui::MenuAction::NewSession {
                                                    active_overlay = None;
                                                    active_menu_index = None;
                                                    let workspace =
                                                        run_live_new_session_menu_command(args)?;
                                                    current_workspace = workspace;
                                                    attached_pane_id =
                                                        current_workspace.pane_id.clone();
                                                    switch_live_surface_to_workspace_pane(
                                                        &current_workspace,
                                                        &mut surface_state,
                                                        &client_state,
                                                        host_mouse_modes.as_mut(),
                                                        use_styled,
                                                    )?;
                                                    if args.redraw && !args.output_json {
                                                        print_live_surface(
                                                            &current_workspace,
                                                            &surface_state.current_surface_metadata,
                                                            &surface_state.current_surface_text,
                                                            args.redraw,
                                                            redraw_state.as_mut(),
                                                            Some(
                                                                &surface_state
                                                                    .current_pane_surfaces,
                                                            ),
                                                            Some(
                                                                &surface_state
                                                                    .current_pane_surface_summaries,
                                                            ),
                                                        );
                                                        flush_stdout()?;
                                                    }
                                                } else {
                                                    let session_inventory =
                                                        if action == tui::MenuAction::Sessions {
                                                            Some(fetch_session_inventory(args)?)
                                                        } else {
                                                            None
                                                        };
                                                    active_overlay = Some(menu_overlay_for_action(
                                                        action,
                                                        &current_workspace,
                                                        &surface_state,
                                                    ));
                                                    if let Some(inventory) =
                                                        session_inventory.as_ref()
                                                    {
                                                        active_overlay = Some(
                                                            menu_overlay_for_action_with_session_inventory(
                                                                action,
                                                                &current_workspace,
                                                                &surface_state,
                                                                Some(inventory),
                                                            ),
                                                        );
                                                    }
                                                    if args.redraw && !args.output_json {
                                                        print_live_surface_with_overlay(
                                                            &current_workspace,
                                                            &surface_state.current_surface_metadata,
                                                            &surface_state.current_surface_text,
                                                            args.redraw,
                                                            redraw_state.as_mut(),
                                                            Some(
                                                                &surface_state
                                                                    .current_pane_surfaces,
                                                            ),
                                                            Some(
                                                                &surface_state
                                                                    .current_pane_surface_summaries,
                                                            ),
                                                            active_overlay.as_ref(),
                                                        );
                                                        flush_stdout()?;
                                                    }
                                                }
                                            }
                                            Some(LiveMouseDispatch::Overlay(action)) => {
                                                match action {
                                                    tui::OverlayAction::SwitchTab(tab_id) => {
                                                        active_overlay = None;
                                                        active_menu_index = None;
                                                        let workspace =
                                                            run_live_tab_switch_menu_command(
                                                                args, &tab_id,
                                                            )?;
                                                        current_workspace = workspace;
                                                        attached_pane_id =
                                                            current_workspace.pane_id.clone();
                                                        switch_live_surface_to_workspace_pane(
                                                            &current_workspace,
                                                            &mut surface_state,
                                                            &client_state,
                                                            host_mouse_modes.as_mut(),
                                                            use_styled,
                                                        )?;
                                                        if args.redraw && !args.output_json {
                                                            print_live_surface(
                                                                &current_workspace,
                                                                &surface_state
                                                                    .current_surface_metadata,
                                                                &surface_state.current_surface_text,
                                                                args.redraw,
                                                                redraw_state.as_mut(),
                                                                Some(
                                                                    &surface_state
                                                                        .current_pane_surfaces,
                                                                ),
                                                                Some(
                                                                    &surface_state
                                                                        .current_pane_surface_summaries,
                                                                ),
                                                            );
                                                            flush_stdout()?;
                                                        }
                                                    }
                                                    tui::OverlayAction::SwitchSession(
                                                        session_id,
                                                    ) => {
                                                        active_overlay = None;
                                                        active_menu_index = None;
                                                        if session_id
                                                            != current_workspace.session_id
                                                        {
                                                            let switched = switch_live_session(
                                                                args,
                                                                &options,
                                                                &mut client_state,
                                                                &mut speculative_echo,
                                                                &mut host_mouse_modes,
                                                                &mut client_inventory,
                                                                &mut recorder,
                                                                socket_scope,
                                                                &session_id,
                                                                setup_read_timeout,
                                                                live_socket_read_timeout,
                                                                &mut redraw_state,
                                                                use_styled,
                                                            )?;
                                                            stream = switched.stream;
                                                            client_sequence =
                                                                switched.client_sequence;
                                                            attached_pane_id =
                                                                switched.attached_pane_id;
                                                            current_workspace = switched.workspace;
                                                            surface_state = switched.surface_state;
                                                            flush_stdout()?;
                                                        }
                                                    }
                                                    tui::OverlayAction::FocusPane(pane_id) => {
                                                        active_overlay = None;
                                                        active_menu_index = None;
                                                        if focus_live_client_pane(
                                                            &pane_id,
                                                            &mut attached_pane_id,
                                                            &mut current_workspace,
                                                            &mut surface_state,
                                                            &client_state,
                                                            host_mouse_modes.as_mut(),
                                                            redraw_state.as_mut(),
                                                            args,
                                                            use_styled,
                                                        )? {
                                                            flush_stdout()?;
                                                        }
                                                    }
                                                }
                                            }
                                            Some(LiveMouseDispatch::ClearOverlay) => {
                                                if (active_overlay.take().is_some()
                                                    || active_menu_index.take().is_some())
                                                    && args.redraw
                                                    && !args.output_json
                                                {
                                                    print_live_surface(
                                                        &current_workspace,
                                                        &surface_state.current_surface_metadata,
                                                        &surface_state.current_surface_text,
                                                        args.redraw,
                                                        redraw_state.as_mut(),
                                                        Some(&surface_state.current_pane_surfaces),
                                                        Some(
                                                            &surface_state
                                                                .current_pane_surface_summaries,
                                                        ),
                                                    );
                                                    flush_stdout()?;
                                                }
                                            }
                                            None => {}
                                        }
                                    }
                                }
                            }
                            sent_stdin_bytes_this_cycle = true;
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
                    local::AttachMouseInput {
                        row: mouse_event.row,
                        col: mouse_event.col,
                        pixel_x: mouse_event.pixel_x,
                        pixel_y: mouse_event.pixel_y,
                        button: mouse_event.button,
                        action: mouse_event.action,
                        modifiers: mouse_event.modifiers,
                    },
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
                let input_seq = local::send_key_input_with_sequence(
                    &mut stream,
                    &mut client_sequence,
                    &attached_pane_id,
                    input_text,
                )?;
                repaint_speculative_echo(
                    args.speculative_echo,
                    &client_state,
                    &mut speculative_echo,
                    &attached_pane_id,
                    input_seq,
                    input_text,
                    &current_workspace,
                    &surface_state.current_surface_metadata,
                    &mut surface_state.current_surface_text,
                    &mut redraw_state,
                    use_styled,
                )?;
            }
        }

        loop {
            if let Some(reader) = stdin_bytes.as_ref()
                && (!sent_stdin_bytes_this_cycle || read_after_stdin_bytes_this_cycle)
            {
                let timeout = if sent_stdin_bytes_this_cycle {
                    post_input_stream_grace
                } else {
                    live_poll_timeout
                };
                let readiness = {
                    let poll_span = tracing::trace_span!(
                        "live.client.poll_stream_or_stdin",
                        timeout_ms = timeout.as_millis() as u64
                    );
                    poll_span.in_scope(|| {
                        poll_live_stream_or_stdin(
                            &stream,
                            reader.wake_reader(),
                            timeout,
                            sent_stdin_bytes_this_cycle,
                        )
                    })?
                };
                match readiness {
                    LiveLoopReadiness::Stdin | LiveLoopReadiness::Timeout => break,
                    LiveLoopReadiness::Stream => {}
                }
            }
            let read = {
                let read_span = tracing::trace_span!("live.client.read_surface_update");
                read_span.in_scope(|| local::read_live_surface_update_from_stream(&mut stream))?
            };
            if sent_stdin_bytes_this_cycle && !matches!(read, local::LiveSurfaceRead::NoFrame) {
                read_after_stdin_bytes_this_cycle = true;
            }
            match read {
                local::LiveSurfaceRead::Workspace(workspace) => {
                    current_workspace = workspace;
                    preserve_live_client_focus(&mut current_workspace, &attached_pane_id);
                    let event = format_live_workspace_json(&current_workspace);
                    recorder.record(&event)?;
                    if args.output_json {
                        println!("{event}");
                    } else if args.redraw {
                        print_live_surface_with_overlay(
                            &current_workspace,
                            &surface_state.current_surface_metadata,
                            &surface_state.current_surface_text,
                            args.redraw,
                            redraw_state.as_mut(),
                            Some(&surface_state.current_pane_surfaces),
                            Some(&surface_state.current_pane_surface_summaries),
                            active_overlay.as_ref(),
                        );
                    } else {
                        println!("{}", current_workspace.display_line());
                    }
                    flush_stdout()?;
                }
                local::LiveSurfaceRead::Presence(presence) => {
                    let event = format_live_presence_json(&presence);
                    recorder.record(&event)?;
                    if args.output_json {
                        println!("{event}");
                        flush_stdout()?;
                    }
                }
                local::LiveSurfaceRead::ClientInventorySnapshot(snapshot) => {
                    client_inventory.apply_snapshot(snapshot);
                    if let Some(state) = redraw_state.as_mut() {
                        state.record_client_count(client_inventory.count());
                        print_live_surface_with_overlay(
                            &current_workspace,
                            &surface_state.current_surface_metadata,
                            &surface_state.current_surface_text,
                            args.redraw,
                            Some(state),
                            Some(&surface_state.current_pane_surfaces),
                            Some(&surface_state.current_pane_surface_summaries),
                            active_overlay.as_ref(),
                        );
                        flush_stdout()?;
                    }
                }
                local::LiveSurfaceRead::ClientInventoryPatch(patch) => {
                    if client_inventory.apply_patch(patch).is_ok()
                        && let Some(state) = redraw_state.as_mut()
                    {
                        state.record_client_count(client_inventory.count());
                        print_live_surface_with_overlay(
                            &current_workspace,
                            &surface_state.current_surface_metadata,
                            &surface_state.current_surface_text,
                            args.redraw,
                            Some(state),
                            Some(&surface_state.current_pane_surfaces),
                            Some(&surface_state.current_pane_surface_summaries),
                            active_overlay.as_ref(),
                        );
                        flush_stdout()?;
                    }
                }
                local::LiveSurfaceRead::Pong(pong) => {
                    if let Some(tracker) = rtt_tracker.as_mut()
                        && let Some(rtt) = tracker.record_pong(&pong)
                    {
                        if let Some(state) = redraw_state.as_mut() {
                            state.record_rtt(rtt);
                            print_live_surface_with_overlay(
                                &current_workspace,
                                &surface_state.current_surface_metadata,
                                &surface_state.current_surface_text,
                                args.redraw,
                                Some(state),
                                Some(&surface_state.current_pane_surfaces),
                                Some(&surface_state.current_pane_surface_summaries),
                                active_overlay.as_ref(),
                            );
                            flush_stdout()?;
                        }
                    }
                }
                local::LiveSurfaceRead::Update(update) => {
                    process_surface_update(
                        &update,
                        &mut surface_state,
                        &mut speculative_echo,
                        &mut client_state,
                        &mut host_mouse_modes,
                        &mut redraw_state,
                        &mut recorder,
                        &current_workspace,
                        args,
                        use_styled,
                    )?;
                }
                local::LiveSurfaceRead::Error(error) => {
                    let event = format_live_error_json(&error);
                    recorder.record(&event)?;
                    if args.output_json {
                        println!("{event}");
                        flush_stdout()?;
                    }
                    return Err(format!("live server error: {error}").into());
                }
                local::LiveSurfaceRead::NoFrame => break,
                local::LiveSurfaceRead::Closed => {
                    eprintln!("nmux: live server closed connection");
                    return finish_live(
                        args,
                        &client_state,
                        &mut recorder,
                        LiveDetachReason::ServerClosed,
                    );
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

    finish_live(args, &client_state, &mut recorder, detach_reason)
}

/// Mutable state that tracks the current surface across live poll iterations.
///
/// Grouping these fields avoids threading a dozen `&mut` parameters through
/// helpers that process surface updates.
struct LiveSurfaceState {
    current_surface_metadata: local::TerminalMetadataSummary,
    current_modes: local::TerminalModeSummary,
    current_surface_text: String,
    current_pane_surfaces: BTreeMap<String, String>,
    current_pane_surface_summaries: BTreeMap<String, local::RenderedSurfaceSummary>,
    current_pane_modes: BTreeMap<String, local::TerminalModeSummary>,
    scrollback_views: BTreeMap<String, LiveScrollbackView>,
}

struct LiveSessionSwitch {
    stream: UnixStream,
    client_sequence: local::ClientFrameSequence,
    attached_pane_id: String,
    workspace: local::WorkspaceSummary,
    surface_state: LiveSurfaceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LiveScrollbackView {
    start_line: u64,
    line_count: u32,
    total_lines: u64,
}

/// Process a single surface update: reconcile speculative echo, render the
/// styled text, update pane surface state, and emit output (JSON or redraw).
///
/// This is the common path shared between the pending-surface-updates drain
/// after initial attach and the steady-state poll loop.
fn process_surface_update(
    update: &local::SurfaceUpdate,
    state: &mut LiveSurfaceState,
    speculative_echo: &mut local::SpeculativeEchoOverlay,
    client_state: &mut local::ClientAttachState,
    host_mouse_modes: &mut Option<HostMouseModeMirror>,
    redraw_state: &mut Option<RedrawState>,
    recorder: &mut LiveRecorder,
    current_workspace: &local::WorkspaceSummary,
    args: &Args,
    use_styled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let decode_start = Instant::now();
    speculative_echo.reconcile_update(update);
    let previous_metadata = state.current_surface_metadata.clone();
    let update_metadata = local::TerminalMetadataSummary {
        title: update.title.clone(),
        working_directory: update.working_directory.clone(),
    };
    let update_surface_text = client_state.render_surface_update_styled(update, use_styled)?;
    state
        .current_pane_surfaces
        .insert(update.pane_id.clone(), update_surface_text.clone());
    if let Some(summary) = client_state.cached_rendered_surface_summary(&update.pane_id) {
        state
            .current_pane_surface_summaries
            .insert(update.pane_id.clone(), summary);
    }
    state
        .current_pane_modes
        .insert(update.pane_id.clone(), update.modes);
    state.scrollback_views.remove(&update.pane_id);
    if update.pane_id == current_workspace.pane_id {
        state.current_surface_metadata = update_metadata.clone();
        state.current_modes = update.modes;
        if let Some(mouse_modes) = host_mouse_modes.as_mut() {
            mouse_modes.sync(state.current_modes)?;
        }
        state.current_surface_text = update_surface_text.clone();
    } else if let Some(active_text) = state.current_pane_surfaces.get(&current_workspace.pane_id) {
        state.current_surface_text = active_text.clone();
    }
    if let Some(rs) = redraw_state {
        rs.record_decode_time(decode_start.elapsed());
    }
    if args.output_json {
        let event = format_live_surface_update_json(
            current_workspace,
            &update_metadata,
            &update_surface_text,
            update,
        );
        recorder.record(&event)?;
        println!("{event}");
    } else {
        recorder.record(&format_live_surface_update_json(
            current_workspace,
            &update_metadata,
            &update_surface_text,
            update,
        ))?;
        print_live_update(
            current_workspace,
            &previous_metadata,
            &state.current_surface_metadata,
            &state.current_surface_text,
            update,
            args.redraw,
            redraw_state.as_mut(),
            Some(&state.current_pane_surfaces),
            Some(&state.current_pane_surface_summaries),
        );
    }
    flush_stdout()?;
    Ok(())
}

fn frontend_resize_pane_size(
    workspace: &local::WorkspaceSummary,
    pane_id: &str,
    terminal_cols: u32,
    terminal_rows: u32,
    ratatui_redraw: bool,
) -> (u32, u32) {
    if !ratatui_redraw {
        return (terminal_cols, terminal_rows);
    }

    let cols = terminal_cols.max(1).min(u16::MAX as u32) as u16;
    let rows = terminal_rows.saturating_sub(1).max(1).min(u16::MAX as u32) as u16;
    tui::pane_content_rect(workspace, cols, rows, pane_id)
        .map(|rect| (u32::from(rect.width.max(1)), u32::from(rect.height.max(1))))
        .unwrap_or((terminal_cols, terminal_rows))
}

fn apply_frontend_workspace_size(
    workspace: &mut local::WorkspaceSummary,
    pane_id: &str,
    cols: u32,
    rows: u32,
) {
    workspace.cols = cols;
    workspace.rows = rows;
    if let Some(tree) = workspace.pane_tree.as_mut() {
        apply_frontend_pane_size(tree, pane_id, cols, rows);
    }
}

fn apply_frontend_pane_size(
    pane: &mut local::WorkspacePaneSummary,
    pane_id: &str,
    cols: u32,
    rows: u32,
) -> bool {
    if pane.pane_id == pane_id {
        pane.cols = cols;
        pane.rows = rows;
        return true;
    }

    for child in &mut pane.children {
        if apply_frontend_pane_size(child, pane_id, cols, rows) {
            return true;
        }
    }
    false
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
        args.target_session_id.as_deref(),
        &command,
        args.start_working_dir.as_deref(),
        &args.start_env,
        terminal_size()?,
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
            socket_path: root.join("nmux.sock"),
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

struct PersistentDaemon;

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
        session_id: Option<&str>,
        command: &str,
        working_dir: Option<&str>,
        env: &[(String, String)],
        initial_size: Option<(u32, u32)>,
        startup_timeout: Duration,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            child: start_daemon_for_attach(
                socket_path,
                mode,
                session_id,
                command,
                working_dir,
                env,
                initial_size,
                startup_timeout,
            )?,
        })
    }
}

impl PersistentDaemon {
    fn start(
        socket_path: &Path,
        session_id: Option<&str>,
        command: &str,
        initial_size: Option<(u32, u32)>,
        startup_timeout: Duration,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let nmux = nmux_binary_path()?;
        let socket_arg = socket_path
            .to_str()
            .ok_or("daemon socket path is not UTF-8")?;
        let mut command_args = vec![
            "daemon".to_owned(),
            "--socket".to_owned(),
            socket_arg.to_owned(),
            "--live-forever".to_owned(),
            "--command".to_owned(),
            command.to_owned(),
        ];
        if let Some(session_id) = session_id {
            command_args.push("--session".to_owned());
            command_args.push(session_id.to_owned());
        }
        if let Some((cols, rows)) = initial_size {
            command_args.push("--cols".to_owned());
            command_args.push(cols.to_string());
            command_args.push("--rows".to_owned());
            command_args.push(rows.to_string());
        }
        let args = command_args
            .into_iter()
            .map(|arg| shell_quote_for_sh(&arg))
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "nohup {} {args} >/dev/null 2>&1 &",
            shell_quote_for_sh(&nmux.display().to_string())
        );
        let status = Command::new("sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|err| format!("failed to start daemon launcher: {err}"))?;
        if !status.success() {
            return Err(format!("daemon launcher failed: {status}").into());
        }
        wait_for_daemon_socket(socket_path, startup_timeout)?;
        Ok(Self)
    }
}

fn wait_for_daemon_socket(
    socket_path: &Path,
    startup_timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + startup_timeout;
    while Instant::now() < deadline {
        if socket_path.exists() {
            thread::sleep(Duration::from_millis(1000));
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err(format!(
        "daemon did not bind {} within {} ms",
        socket_path.display(),
        startup_timeout.as_millis()
    )
    .into())
}

fn wait_for_daemon_ready(
    child: &mut Child,
    startup_timeout: Duration,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{label} stdout was not captured"))?;
    let (ready_tx, ready_rx) = mpsc::channel();
    let reader_label = label.to_owned();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let result = match reader.read_line(&mut line) {
            Ok(0) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("{reader_label} exited before readiness"),
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
            return Err(format!("{label} exited before readiness").into());
        }
        Ok(Err(err)) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("failed to read {label} readiness: {err}").into());
        }
        Err(RecvTimeoutError::Timeout) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{label} did not become ready within {} ms",
                startup_timeout.as_millis()
            )
            .into());
        }
        Err(RecvTimeoutError::Disconnected) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{label} readiness reader stopped unexpectedly").into());
        }
    };
    if line.contains("\"event\":\"ready\"") {
        return Ok(());
    }
    let _ = child.wait();
    if let Some(message) = managed_ready_error_message(&line) {
        return Err(format!("{label} startup failed: {message}").into());
    }
    Err(format!("{label} startup failed: {}", line.trim()).into())
}

fn start_daemon_for_attach(
    socket_path: &Path,
    mode: ManagedDaemonMode,
    session_id: Option<&str>,
    command: &str,
    working_dir: Option<&str>,
    env: &[(String, String)],
    initial_size: Option<(u32, u32)>,
    startup_timeout: Duration,
) -> Result<Child, Box<dyn std::error::Error>> {
    let nmux = nmux_binary_path()?;
    let socket_path = socket_path
        .to_str()
        .ok_or("managed socket path is not UTF-8")?;
    let mut command_args = vec![
        "daemon".to_owned(),
        "--socket".to_owned(),
        socket_path.to_owned(),
        "--ready-json".to_owned(),
        mode.flag().to_owned(),
        "--command".to_owned(),
        command.to_owned(),
    ];
    if let Some(session_id) = session_id {
        command_args.push("--session".to_owned());
        command_args.push(session_id.to_owned());
    }
    if let Some(working_dir) = working_dir {
        command_args.push("--cwd".to_owned());
        command_args.push(working_dir.to_owned());
    }
    for (key, value) in env {
        command_args.push("--env".to_owned());
        command_args.push(format!("{key}={value}"));
    }
    if let Some((cols, rows)) = initial_size {
        command_args.push("--cols".to_owned());
        command_args.push(cols.to_string());
        command_args.push("--rows".to_owned());
        command_args.push(rows.to_string());
    }
    let mut child = Command::new(nmux)
        .args(command_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("failed to start daemon: {err}"))?;
    wait_for_daemon_ready(&mut child, startup_timeout, "managed daemon")?;
    Ok(child)
}

impl Drop for ManagedDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn nmux_binary_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = option_env!("CARGO_BIN_EXE_nmux") {
        return Ok(PathBuf::from(path));
    }
    Ok(std::env::current_exe()?)
}

fn managed_ready_error_message(line: &str) -> Option<String> {
    let message_key = "\"message\":";
    let message_start = line.find(message_key)? + message_key.len();
    decode_json_string_at(line[message_start..].trim_start()).ok()
}

fn decode_json_string_at(value: &str) -> Result<String, &'static str> {
    let mut chars = value.chars();
    if chars.next() != Some('"') {
        return Err("expected JSON string");
    }

    let mut decoded = String::new();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Ok(decoded),
            '\\' => match chars.next().ok_or("incomplete JSON escape")? {
                '"' => decoded.push('"'),
                '\\' => decoded.push('\\'),
                '/' => decoded.push('/'),
                'b' => decoded.push('\u{0008}'),
                'f' => decoded.push('\u{000c}'),
                'n' => decoded.push('\n'),
                'r' => decoded.push('\r'),
                't' => decoded.push('\t'),
                'u' => return Err("unicode JSON escapes are not supported here"),
                _ => return Err("invalid JSON escape"),
            },
            _ => decoded.push(ch),
        }
    }

    Err("unterminated JSON string")
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
    recorder: &mut LiveRecorder,
    reason: LiveDetachReason,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Err(err) = save_live_state(args, client_state) {
        report_live_setup_error(args, err.as_ref())?;
        return Err(err);
    }
    let event = format_live_detach_json(reason);
    recorder.record(&event)?;
    if args.output_json {
        println!("{event}");
        flush_stdout()?;
    }
    Ok(())
}

struct LiveRecorder {
    file: Option<fs::File>,
    start: Instant,
}

impl LiveRecorder {
    fn open(path: Option<&Path>) -> io::Result<Self> {
        let file = match path {
            Some(path) => {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    fs::create_dir_all(parent)?;
                }
                Some(fs::File::create(path)?)
            }
            None => None,
        };
        Ok(Self {
            file,
            start: Instant::now(),
        })
    }

    fn record(&mut self, event_json: &str) -> io::Result<()> {
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        let elapsed_ms = self.start.elapsed().as_millis();
        if let Some(rest) = event_json.strip_prefix('{') {
            writeln!(file, "{{\"elapsed_ms\":{elapsed_ms},{rest}")?;
        } else {
            writeln!(
                file,
                "{{\"elapsed_ms\":{elapsed_ms},\"event\":\"raw\",\"raw\":{}}}",
                local::json_string(event_json)
            )?;
        }
        file.flush()
    }
}

#[allow(clippy::too_many_arguments)]
fn repaint_speculative_echo(
    enabled: bool,
    client_state: &local::ClientAttachState,
    overlay: &mut local::SpeculativeEchoOverlay,
    pane_id: &str,
    input_seq: u64,
    text: &str,
    workspace: &local::WorkspaceSummary,
    metadata: &local::TerminalMetadataSummary,
    current_surface_text: &mut String,
    redraw_state: &mut Option<RedrawState>,
    use_styled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !enabled {
        return Ok(());
    }
    let Some(predicted) =
        client_state.render_speculative_echo(overlay, pane_id, input_seq, text, use_styled)
    else {
        return Ok(());
    };
    let prediction = overlay.prediction().cloned();
    *current_surface_text = predicted;
    match redraw_state {
        Some(state) => {
            if state.terminal.is_some() {
                print_live_surface(
                    workspace,
                    metadata,
                    current_surface_text,
                    true,
                    Some(state),
                    None,
                    None,
                );
            } else if let Some(prediction) = prediction.as_ref()
                && let Some(text) = state.render_speculative_append_text(
                    workspace,
                    current_surface_text,
                    prediction,
                )
            {
                print!("{text}");
            } else {
                print_live_surface(
                    workspace,
                    metadata,
                    current_surface_text,
                    true,
                    Some(state),
                    None,
                    None,
                );
            }
        }
        None => {
            print_live_surface(
                workspace,
                metadata,
                current_surface_text,
                true,
                None,
                None,
                None,
            );
        }
    }
    flush_stdout()?;
    Ok(())
}

fn stdin_bytes_speculative_echo_enabled(args: &Args) -> bool {
    args.redraw && args.stdin_bytes && args.local_echo == LocalEcho::Off && !args.output_json
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
) -> Result<
    (
        Option<local::ScrollbackChunkSummary>,
        Vec<local::SurfaceUpdate>,
        Vec<local::LiveSurfaceRead>,
    ),
    Box<dyn std::error::Error>,
> {
    if args.no_scrollback {
        return Ok((None, Vec::new(), Vec::new()));
    }
    let mut pending_updates = Vec::new();
    let mut pending_live = Vec::new();
    let scrollback = local::fetch_scrollback_chunk_with_selection_and_pending_live(
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
        Some(&mut pending_updates),
        Some(&mut pending_live),
    )?;
    Ok((Some(scrollback), pending_updates, pending_live))
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

struct StdinByteReader {
    rx: mpsc::Receiver<StdinByteRead>,
    wake_reader: UnixStream,
}

impl StdinByteReader {
    fn try_recv(&self) -> Result<StdinByteRead, TryRecvError> {
        let result = self.rx.try_recv();
        if !matches!(result, Err(TryRecvError::Empty)) {
            let _ = drain_stdin_wake_reader(&self.wake_reader);
        }
        result
    }

    fn wake_reader(&self) -> &UnixStream {
        &self.wake_reader
    }
}

fn spawn_stdin_byte_reader() -> io::Result<StdinByteReader> {
    let (tx, rx) = mpsc::channel();
    let (wake_reader, mut wake_writer) = UnixStream::pair()?;
    wake_reader.set_nonblocking(true)?;
    wake_writer.set_nonblocking(true)?;
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buffer = [0_u8; 1024];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) => {
                    let _ = tx.send(StdinByteRead::Closed);
                    notify_stdin_wake_reader(&mut wake_writer);
                    break;
                }
                Ok(count) => {
                    let input = buffer[..count].to_vec();
                    if tx.send(StdinByteRead::Input(input)).is_err() {
                        break;
                    }
                    notify_stdin_wake_reader(&mut wake_writer);
                }
                Err(err) => {
                    let _ = tx.send(StdinByteRead::Error(err.to_string()));
                    notify_stdin_wake_reader(&mut wake_writer);
                    break;
                }
            }
        }
    });
    Ok(StdinByteReader { rx, wake_reader })
}

fn notify_stdin_wake_reader(wake_writer: &mut UnixStream) {
    match wake_writer.write(&[1]) {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
        Err(_) => {}
    }
}

fn drain_stdin_wake_reader(wake_reader: &UnixStream) -> io::Result<()> {
    let mut reader = wake_reader.try_clone()?;
    let mut buffer = [0_u8; 64];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveLoopReadiness {
    Stream,
    Stdin,
    Timeout,
}

fn poll_live_stream_or_stdin(
    stream: &UnixStream,
    stdin_wake_reader: &UnixStream,
    timeout: Duration,
    wait_for_stream_on_stdin: bool,
) -> io::Result<LiveLoopReadiness> {
    let mut fds = [
        libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: stdin_wake_reader.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    loop {
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if ready > 0 {
            if fds[0].revents != 0 {
                return Ok(LiveLoopReadiness::Stream);
            }
            if fds[1].revents != 0 {
                if wait_for_stream_on_stdin
                    && poll_live_stream(stream, timeout)? == LiveLoopReadiness::Stream
                {
                    return Ok(LiveLoopReadiness::Stream);
                }
                return Ok(LiveLoopReadiness::Stdin);
            }
            return Ok(LiveLoopReadiness::Timeout);
        }
        if ready == 0 {
            return Ok(LiveLoopReadiness::Timeout);
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

fn poll_live_stream(stream: &UnixStream, timeout: Duration) -> io::Result<LiveLoopReadiness> {
    let mut fd = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    loop {
        let ready = unsafe { libc::poll(&mut fd, 1, timeout_ms) };
        if ready > 0 {
            return Ok(LiveLoopReadiness::Stream);
        }
        if ready == 0 {
            return Ok(LiveLoopReadiness::Timeout);
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum StdinByteForward {
    Raw(Vec<u8>),
    Paste(String),
    Mouse(SgrMouseInput),
    Key(StdinKeyInput),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StdinKey {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Tab,
    BackTab,
    OpenMenu(tui::MenuAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StdinKeyInput {
    key: StdinKey,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SgrMouseInput {
    row: u32,
    col: u32,
    button: protocol::MouseButton,
    action: protocol::MouseAction,
    modifiers: u32,
}

fn split_stdin_bytes_for_detach(input: &[u8], detach_byte: Option<u8>) -> (Option<Vec<u8>>, bool) {
    let Some(detach_byte) = detach_byte else {
        return (Some(input.to_vec()), false);
    };
    let Some(index) = input.iter().position(|byte| *byte == detach_byte) else {
        return (Some(input.to_vec()), false);
    };

    let before_detach = &input[..index];
    if before_detach.is_empty() {
        (None, true)
    } else {
        (Some(before_detach.to_vec()), true)
    }
}

fn stdin_byte_forwards(input: &[u8]) -> Vec<StdinByteForward> {
    let mut forwards = Vec::new();
    let mut offset = 0;
    while offset < input.len() {
        let paste = find_bytes(&input[offset..], BRACKETED_PASTE_START);
        let mouse = find_sgr_mouse_sequence(&input[offset..])
            .map(|(start, mouse, end)| (start, StdinByteForward::Mouse(mouse), end));
        let key = find_tui_key_sequence(&input[offset..])
            .map(|(start, key, end)| (start, StdinByteForward::Key(key), end));
        let Some((start_rel, forward, end_rel)) = next_structured_stdin_forward(
            paste.map(|start| (start, StdinByteForward::Raw(Vec::new()), 0)),
            mouse,
            key,
        ) else {
            forwards.push(StdinByteForward::Raw(input[offset..].to_vec()));
            break;
        };
        let start = offset + start_rel;
        if start > offset {
            forwards.push(StdinByteForward::Raw(input[offset..start].to_vec()));
        }
        if matches!(
            forward,
            StdinByteForward::Mouse(_) | StdinByteForward::Key(_)
        ) {
            forwards.push(forward);
            offset += end_rel;
            continue;
        }
        let paste_start = start + BRACKETED_PASTE_START.len();
        let Some(end_rel) = find_bytes(&input[paste_start..], BRACKETED_PASTE_END) else {
            forwards.push(StdinByteForward::Raw(input[start..].to_vec()));
            break;
        };
        let paste_end = paste_start + end_rel;
        match std::str::from_utf8(&input[paste_start..paste_end]) {
            Ok(text) => forwards.push(StdinByteForward::Paste(text.to_owned())),
            Err(_) => forwards.push(StdinByteForward::Raw(
                input[start..paste_end + BRACKETED_PASTE_END.len()].to_vec(),
            )),
        }
        offset = paste_end + BRACKETED_PASTE_END.len();
    }
    forwards.retain(|forward| match forward {
        StdinByteForward::Raw(bytes) => !bytes.is_empty(),
        StdinByteForward::Paste(_) => true,
        StdinByteForward::Mouse(_) => true,
        StdinByteForward::Key(_) => true,
    });
    forwards
}

fn next_structured_stdin_forward(
    paste: Option<(usize, StdinByteForward, usize)>,
    mouse: Option<(usize, StdinByteForward, usize)>,
    key: Option<(usize, StdinByteForward, usize)>,
) -> Option<(usize, StdinByteForward, usize)> {
    [paste, mouse, key]
        .into_iter()
        .flatten()
        .min_by_key(|candidate| candidate.0)
}

fn find_tui_key_sequence(input: &[u8]) -> Option<(usize, StdinKeyInput, usize)> {
    for start in 0..input.len() {
        if let Some((key, len)) = parse_tui_key_sequence(&input[start..]) {
            return Some((start, key, start + len));
        }
    }
    None
}

fn parse_tui_key_sequence(input: &[u8]) -> Option<(StdinKeyInput, usize)> {
    let candidates: &[(&[u8], StdinKey)] = &[
        (b"\x1b[A", StdinKey::Up),
        (b"\x1b[B", StdinKey::Down),
        (b"\x1b[C", StdinKey::Right),
        (b"\x1b[D", StdinKey::Left),
        (b"\x1b[Z", StdinKey::BackTab),
        (b"\r", StdinKey::Enter),
        (b"\n", StdinKey::Enter),
        (b"\t", StdinKey::Tab),
        (b"\x1bs", StdinKey::OpenMenu(tui::MenuAction::Sessions)),
        (b"\x1bn", StdinKey::OpenMenu(tui::MenuAction::NewSession)),
        (b"\x1bw", StdinKey::OpenMenu(tui::MenuAction::Windows)),
        (b"\x1bc", StdinKey::OpenMenu(tui::MenuAction::Clipboard)),
        (b"\x1b", StdinKey::Escape),
    ];
    let (bytes, key) = candidates
        .iter()
        .find(|(bytes, _)| input.starts_with(bytes))?;
    Some((
        StdinKeyInput {
            key: *key,
            bytes: bytes.to_vec(),
        },
        bytes.len(),
    ))
}

fn find_sgr_mouse_sequence(input: &[u8]) -> Option<(usize, SgrMouseInput, usize)> {
    let mut search_offset = 0;
    while search_offset < input.len() {
        let start_rel = find_bytes(&input[search_offset..], SGR_MOUSE_START)?;
        let start = search_offset + start_rel;
        match parse_sgr_mouse_sequence(&input[start..]) {
            Some((mouse, len)) => return Some((start, mouse, start + len)),
            None => search_offset = start.saturating_add(1),
        }
    }
    None
}

fn parse_sgr_mouse_sequence(input: &[u8]) -> Option<(SgrMouseInput, usize)> {
    if !input.starts_with(SGR_MOUSE_START) {
        return None;
    }
    let mut end = SGR_MOUSE_START.len();
    while end < input.len() && input[end] != b'M' && input[end] != b'm' {
        end += 1;
    }
    if end >= input.len() {
        return None;
    }
    let final_byte = input[end];
    let params = std::str::from_utf8(&input[SGR_MOUSE_START.len()..end]).ok()?;
    let mut parts = params.split(';');
    let code = parts.next()?.parse::<u32>().ok()?;
    let x = parts.next()?.parse::<u32>().ok()?;
    let y = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() || x == 0 || y == 0 {
        return None;
    }
    let action = if final_byte == b'm' {
        protocol::MouseAction::Release
    } else if code & 32 != 0 {
        protocol::MouseAction::Motion
    } else {
        protocol::MouseAction::Press
    };
    let button = if code & 64 != 0 {
        if code & 1 != 0 {
            protocol::MouseButton::WheelDown
        } else {
            protocol::MouseButton::WheelUp
        }
    } else {
        match code & 3 {
            0 => protocol::MouseButton::Left,
            1 => protocol::MouseButton::Middle,
            2 => protocol::MouseButton::Right,
            _ => protocol::MouseButton::None,
        }
    };
    let mut modifiers = 0;
    if code & 4 != 0 {
        modifiers |= 1;
    }
    if code & 16 != 0 {
        modifiers |= 2;
    }
    if code & 8 != 0 {
        modifiers |= 4;
    }
    Some((
        SgrMouseInput {
            row: y - 1,
            col: x - 1,
            button,
            action,
            modifiers,
        },
        end + 1,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LiveMouseDispatch {
    FocusPane(String),
    Menu(tui::MenuAction),
    Overlay(tui::OverlayAction),
    PaneMouse(String, local::AttachMouseInput),
    PaneScroll {
        pane_id: String,
        direction: LiveScrollDirection,
        visible_rows: u16,
    },
    ClearOverlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveScrollDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LiveKeyHandling {
    Handled,
    Forward(Vec<u8>),
}

#[allow(clippy::too_many_arguments)]
fn handle_live_tui_key(
    key: StdinKeyInput,
    active_menu_index: &mut Option<usize>,
    active_overlay: &mut Option<tui::TuiOverlay>,
    stream: &mut UnixStream,
    client_sequence: &mut local::ClientFrameSequence,
    attached_pane_id: &mut String,
    current_workspace: &mut local::WorkspaceSummary,
    surface_state: &mut LiveSurfaceState,
    client_state: &mut local::ClientAttachState,
    host_mouse_modes: &mut Option<HostMouseModeMirror>,
    speculative_echo: &mut local::SpeculativeEchoOverlay,
    client_inventory: &mut ClientInventoryCache,
    recorder: &mut LiveRecorder,
    socket_scope: Option<local::SocketIdentity>,
    options: &local::AttachOptions,
    setup_read_timeout: Duration,
    live_socket_read_timeout: Duration,
    redraw_state: &mut Option<RedrawState>,
    args: &Args,
    use_styled: bool,
) -> Result<LiveKeyHandling, Box<dyn std::error::Error>> {
    if args.output_json || !args.redraw {
        return Ok(LiveKeyHandling::Forward(key.bytes));
    }

    if key.key == StdinKey::Escape {
        if active_overlay.take().is_some() || active_menu_index.take().is_some() {
            print_live_surface(
                current_workspace,
                &surface_state.current_surface_metadata,
                &surface_state.current_surface_text,
                args.redraw,
                redraw_state.as_mut(),
                Some(&surface_state.current_pane_surfaces),
                Some(&surface_state.current_pane_surface_summaries),
            );
            return Ok(LiveKeyHandling::Handled);
        }
        return Ok(LiveKeyHandling::Forward(key.bytes));
    }

    if let Some(action) = match key.key {
        StdinKey::OpenMenu(action) => Some(action),
        _ => None,
    } {
        *active_menu_index = menu_index(action);
        open_live_menu_overlay(
            action,
            active_menu_index,
            active_overlay,
            stream,
            client_sequence,
            attached_pane_id,
            current_workspace,
            surface_state,
            client_state,
            host_mouse_modes,
            speculative_echo,
            client_inventory,
            recorder,
            socket_scope,
            options,
            setup_read_timeout,
            live_socket_read_timeout,
            redraw_state,
            args,
            use_styled,
        )?;
        return Ok(LiveKeyHandling::Handled);
    }

    if active_overlay.is_some() {
        match key.key {
            StdinKey::Up | StdinKey::BackTab => {
                if let Some(overlay) = active_overlay.as_mut() {
                    tui::move_overlay_selection(overlay, -1);
                }
                repaint_live_overlay(
                    current_workspace,
                    surface_state,
                    redraw_state,
                    args,
                    active_overlay.as_ref(),
                );
                return Ok(LiveKeyHandling::Handled);
            }
            StdinKey::Down | StdinKey::Tab => {
                if let Some(overlay) = active_overlay.as_mut() {
                    tui::move_overlay_selection(overlay, 1);
                }
                repaint_live_overlay(
                    current_workspace,
                    surface_state,
                    redraw_state,
                    args,
                    active_overlay.as_ref(),
                );
                return Ok(LiveKeyHandling::Handled);
            }
            StdinKey::Enter => {
                if let Some(action) = active_overlay
                    .as_ref()
                    .and_then(tui::selected_overlay_action)
                {
                    *active_overlay = None;
                    *active_menu_index = None;
                    handle_live_overlay_action(
                        action,
                        stream,
                        client_sequence,
                        attached_pane_id,
                        current_workspace,
                        surface_state,
                        client_state,
                        host_mouse_modes,
                        speculative_echo,
                        client_inventory,
                        recorder,
                        socket_scope,
                        options,
                        setup_read_timeout,
                        live_socket_read_timeout,
                        redraw_state,
                        args,
                        use_styled,
                    )?;
                }
                return Ok(LiveKeyHandling::Handled);
            }
            _ => return Ok(LiveKeyHandling::Forward(key.bytes)),
        }
    }

    if active_menu_index.is_some() {
        match key.key {
            StdinKey::Left | StdinKey::BackTab => {
                cycle_live_menu(active_menu_index, -1);
            }
            StdinKey::Right | StdinKey::Tab => {
                cycle_live_menu(active_menu_index, 1);
            }
            StdinKey::Down | StdinKey::Enter => {}
            _ => return Ok(LiveKeyHandling::Forward(key.bytes)),
        }
        let action = tui::MENU_ACTIONS[active_menu_index.unwrap_or(0)];
        open_live_menu_overlay(
            action,
            active_menu_index,
            active_overlay,
            stream,
            client_sequence,
            attached_pane_id,
            current_workspace,
            surface_state,
            client_state,
            host_mouse_modes,
            speculative_echo,
            client_inventory,
            recorder,
            socket_scope,
            options,
            setup_read_timeout,
            live_socket_read_timeout,
            redraw_state,
            args,
            use_styled,
        )?;
        return Ok(LiveKeyHandling::Handled);
    }

    Ok(LiveKeyHandling::Forward(key.bytes))
}

fn menu_index(action: tui::MenuAction) -> Option<usize> {
    tui::MENU_ACTIONS
        .iter()
        .position(|candidate| *candidate == action)
}

fn cycle_live_menu(active_menu_index: &mut Option<usize>, delta: i32) {
    let current = active_menu_index.unwrap_or(0);
    let next = (current as i32 + delta).rem_euclid(tui::MENU_ACTIONS.len() as i32) as usize;
    *active_menu_index = Some(next);
}

#[allow(clippy::too_many_arguments)]
fn open_live_menu_overlay(
    action: tui::MenuAction,
    active_menu_index: &mut Option<usize>,
    active_overlay: &mut Option<tui::TuiOverlay>,
    stream: &mut UnixStream,
    client_sequence: &mut local::ClientFrameSequence,
    attached_pane_id: &mut String,
    current_workspace: &mut local::WorkspaceSummary,
    surface_state: &mut LiveSurfaceState,
    client_state: &local::ClientAttachState,
    host_mouse_modes: &mut Option<HostMouseModeMirror>,
    speculative_echo: &mut local::SpeculativeEchoOverlay,
    client_inventory: &mut ClientInventoryCache,
    recorder: &mut LiveRecorder,
    socket_scope: Option<local::SocketIdentity>,
    options: &local::AttachOptions,
    setup_read_timeout: Duration,
    live_socket_read_timeout: Duration,
    redraw_state: &mut Option<RedrawState>,
    args: &Args,
    use_styled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if action == tui::MenuAction::NewSession {
        *active_overlay = None;
        *active_menu_index = None;
        let workspace = run_live_new_session_menu_command(args)?;
        *current_workspace = workspace;
        *attached_pane_id = current_workspace.pane_id.clone();
        switch_live_surface_to_workspace_pane(
            current_workspace,
            surface_state,
            client_state,
            host_mouse_modes.as_mut(),
            use_styled,
        )?;
        repaint_live_overlay(current_workspace, surface_state, redraw_state, args, None);
        return Ok(());
    }

    let session_inventory = if action == tui::MenuAction::Sessions {
        Some(fetch_session_inventory(args)?)
    } else {
        None
    };
    *active_overlay = Some(menu_overlay_for_action_with_session_inventory(
        action,
        current_workspace,
        surface_state,
        session_inventory.as_ref(),
    ));
    if let Some(overlay) = active_overlay.as_mut() {
        overlay.selected = tui::selectable_overlay_index(overlay);
    }
    repaint_live_overlay(
        current_workspace,
        surface_state,
        redraw_state,
        args,
        active_overlay.as_ref(),
    );
    let _ = (
        stream,
        client_sequence,
        speculative_echo,
        client_inventory,
        recorder,
        socket_scope,
        options,
        setup_read_timeout,
        live_socket_read_timeout,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_live_overlay_action(
    action: tui::OverlayAction,
    stream: &mut UnixStream,
    client_sequence: &mut local::ClientFrameSequence,
    attached_pane_id: &mut String,
    current_workspace: &mut local::WorkspaceSummary,
    surface_state: &mut LiveSurfaceState,
    client_state: &mut local::ClientAttachState,
    host_mouse_modes: &mut Option<HostMouseModeMirror>,
    speculative_echo: &mut local::SpeculativeEchoOverlay,
    client_inventory: &mut ClientInventoryCache,
    recorder: &mut LiveRecorder,
    socket_scope: Option<local::SocketIdentity>,
    options: &local::AttachOptions,
    setup_read_timeout: Duration,
    live_socket_read_timeout: Duration,
    redraw_state: &mut Option<RedrawState>,
    args: &Args,
    use_styled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        tui::OverlayAction::SwitchTab(tab_id) => {
            let workspace = run_live_tab_switch_menu_command(args, &tab_id)?;
            *current_workspace = workspace;
            *attached_pane_id = current_workspace.pane_id.clone();
            switch_live_surface_to_workspace_pane(
                current_workspace,
                surface_state,
                client_state,
                host_mouse_modes.as_mut(),
                use_styled,
            )?;
            repaint_live_overlay(current_workspace, surface_state, redraw_state, args, None);
        }
        tui::OverlayAction::SwitchSession(session_id) => {
            if session_id != current_workspace.session_id {
                let switched = switch_live_session(
                    args,
                    options,
                    client_state,
                    speculative_echo,
                    host_mouse_modes,
                    client_inventory,
                    recorder,
                    socket_scope,
                    &session_id,
                    setup_read_timeout,
                    live_socket_read_timeout,
                    redraw_state,
                    use_styled,
                )?;
                *stream = switched.stream;
                *client_sequence = switched.client_sequence;
                *attached_pane_id = switched.attached_pane_id;
                *current_workspace = switched.workspace;
                *surface_state = switched.surface_state;
            }
        }
        tui::OverlayAction::FocusPane(pane_id) => {
            let _ = focus_live_client_pane(
                &pane_id,
                attached_pane_id,
                current_workspace,
                surface_state,
                client_state,
                host_mouse_modes.as_mut(),
                redraw_state.as_mut(),
                args,
                use_styled,
            )?;
        }
    }
    Ok(())
}

fn repaint_live_overlay(
    current_workspace: &local::WorkspaceSummary,
    surface_state: &LiveSurfaceState,
    redraw_state: &mut Option<RedrawState>,
    args: &Args,
    active_overlay: Option<&tui::TuiOverlay>,
) {
    print_live_surface_with_overlay(
        current_workspace,
        &surface_state.current_surface_metadata,
        &surface_state.current_surface_text,
        args.redraw,
        redraw_state.as_mut(),
        Some(&surface_state.current_pane_surfaces),
        Some(&surface_state.current_pane_surface_summaries),
        active_overlay,
    );
}

fn live_mouse_dispatch_for_workspace(
    mouse: SgrMouseInput,
    workspace: &local::WorkspaceSummary,
    active_surface_text: &str,
    pane_surfaces: Option<&BTreeMap<String, String>>,
    pane_modes: Option<&BTreeMap<String, local::TerminalModeSummary>>,
    active_modes: local::TerminalModeSummary,
    overlay: Option<&tui::TuiOverlay>,
) -> Option<LiveMouseDispatch> {
    let (cols, rows) = terminal_size().ok().flatten().unwrap_or((80, 24));
    live_mouse_dispatch_for_workspace_size(
        mouse,
        workspace,
        active_surface_text,
        pane_surfaces,
        pane_modes,
        active_modes,
        overlay,
        cols.max(1).min(u16::MAX as u32) as u16,
        rows.max(1).min(u16::MAX as u32) as u16,
    )
}

#[allow(clippy::too_many_arguments)]
fn live_mouse_dispatch_for_workspace_size(
    mouse: SgrMouseInput,
    workspace: &local::WorkspaceSummary,
    active_surface_text: &str,
    pane_surfaces: Option<&BTreeMap<String, String>>,
    pane_modes: Option<&BTreeMap<String, local::TerminalModeSummary>>,
    active_modes: local::TerminalModeSummary,
    overlay: Option<&tui::TuiOverlay>,
    cols: u16,
    rows: u16,
) -> Option<LiveMouseDispatch> {
    let frame_rows = rows.saturating_sub(1).max(1);
    let frame = tui::render_workspace_frame(
        tui::WorkspaceFrameInput {
            workspace,
            active_surface_text,
            pane_surfaces,
            pane_surface_summaries: None,
            pane_chrome: None,
            overlay,
        },
        cols.max(1),
        frame_rows,
    );
    let x = u16::try_from(mouse.col).ok()?;
    let y = u16::try_from(mouse.row).ok()?;
    let hit = tui::hit_test_region(&frame.hits, x, y)?;

    match &hit.target {
        tui::HitTarget::PaneContent(pane_id) => {
            let target_modes = pane_modes
                .and_then(|modes| modes.get(pane_id))
                .copied()
                .unwrap_or_else(|| {
                    if pane_id == &workspace.pane_id {
                        active_modes
                    } else {
                        local::TerminalModeSummary::default()
                    }
                });
            if target_modes.mouse_tracking {
                return Some(LiveMouseDispatch::PaneMouse(
                    pane_id.clone(),
                    local::AttachMouseInput {
                        row: u32::from(y.saturating_sub(hit.rect.y)),
                        col: u32::from(x.saturating_sub(hit.rect.x)),
                        pixel_x: None,
                        pixel_y: None,
                        button: mouse.button,
                        action: mouse.action,
                        modifiers: mouse.modifiers,
                    },
                ));
            }
            if let Some(direction) = sgr_mouse_scroll_direction(mouse) {
                return Some(LiveMouseDispatch::PaneScroll {
                    pane_id: pane_id.clone(),
                    direction,
                    visible_rows: hit.rect.height.max(1),
                });
            }
            if pane_id != &workspace.pane_id && sgr_mouse_is_primary_press(mouse) {
                return Some(LiveMouseDispatch::FocusPane(pane_id.clone()));
            }
            None
        }
        tui::HitTarget::Pane(pane_id) | tui::HitTarget::WindowTreePane(pane_id)
            if sgr_mouse_is_primary_press(mouse) =>
        {
            Some(LiveMouseDispatch::FocusPane(pane_id.clone()))
        }
        tui::HitTarget::Menu(action) if sgr_mouse_is_primary_press(mouse) => {
            Some(LiveMouseDispatch::Menu(*action))
        }
        tui::HitTarget::Overlay(action) if sgr_mouse_is_primary_press(mouse) => {
            Some(LiveMouseDispatch::Overlay(action.clone()))
        }
        tui::HitTarget::Background if sgr_mouse_is_primary_press(mouse) => {
            Some(LiveMouseDispatch::ClearOverlay)
        }
        _ => None,
    }
}

fn sgr_mouse_is_primary_press(mouse: SgrMouseInput) -> bool {
    mouse.action == protocol::MouseAction::Press && mouse.button == protocol::MouseButton::Left
}

fn sgr_mouse_scroll_direction(mouse: SgrMouseInput) -> Option<LiveScrollDirection> {
    if mouse.action != protocol::MouseAction::Press {
        return None;
    }
    match mouse.button {
        protocol::MouseButton::WheelUp => Some(LiveScrollDirection::Up),
        protocol::MouseButton::WheelDown => Some(LiveScrollDirection::Down),
        _ => None,
    }
}

fn menu_overlay_for_action(
    action: tui::MenuAction,
    workspace: &local::WorkspaceSummary,
    surface_state: &LiveSurfaceState,
) -> tui::TuiOverlay {
    menu_overlay_for_action_with_session_inventory(action, workspace, surface_state, None)
}

fn menu_overlay_for_action_with_session_inventory(
    action: tui::MenuAction,
    workspace: &local::WorkspaceSummary,
    surface_state: &LiveSurfaceState,
    session_inventory: Option<&local::SessionInventorySummary>,
) -> tui::TuiOverlay {
    let (title, lines) = match action {
        tui::MenuAction::Sessions => (
            "sessions".to_owned(),
            session_overlay_lines(workspace, session_inventory),
        ),
        tui::MenuAction::NewSession => (
            "new session".to_owned(),
            vec![overlay_text("new named session")],
        ),
        tui::MenuAction::Windows => {
            let mut lines = vec![overlay_text(format!("tab {}", workspace.tab_id))];
            for tab in workspace_tabs(workspace) {
                let marker = if tab.tab_id == workspace.tab_id {
                    "*"
                } else {
                    " "
                };
                lines.push(tui::TuiOverlayLine {
                    text: format!("{marker} {} {}", tab.tab_id, tab.title),
                    action: (tab.tab_id != workspace.tab_id)
                        .then(|| tui::OverlayAction::SwitchTab(tab.tab_id)),
                });
            }
            for pane in workspace_panes(workspace) {
                let marker = if pane.pane_id == workspace.pane_id {
                    "*"
                } else {
                    " "
                };
                lines.push(tui::TuiOverlayLine {
                    text: format!("{marker} {} {}x{}", pane.pane_id, pane.cols, pane.rows),
                    action: (pane.pane_id != workspace.pane_id)
                        .then(|| tui::OverlayAction::FocusPane(pane.pane_id)),
                });
            }
            ("windows".to_owned(), lines)
        }
        tui::MenuAction::Clipboard => {
            let paste_mode = if surface_state.current_modes.bracketed_paste {
                "pane bracketed paste enabled"
            } else {
                "nmux paste forwarding enabled"
            };
            ("clipboard".to_owned(), vec![overlay_text(paste_mode)])
        }
    };
    let selected = lines.iter().position(|line| line.action.is_some());
    tui::TuiOverlay {
        title,
        lines,
        selected,
    }
}

fn overlay_text(text: impl Into<String>) -> tui::TuiOverlayLine {
    tui::TuiOverlayLine {
        text: text.into(),
        action: None,
    }
}

fn session_overlay_lines(
    workspace: &local::WorkspaceSummary,
    session_inventory: Option<&local::SessionInventorySummary>,
) -> Vec<tui::TuiOverlayLine> {
    let Some(inventory) = session_inventory else {
        return vec![overlay_text(format!("session {}", workspace.session_id))];
    };
    let mut lines = Vec::new();
    for session in &inventory.sessions {
        let marker = if session.session_id == workspace.session_id {
            "*"
        } else {
            " "
        };
        lines.push(tui::TuiOverlayLine {
            text: format!("{marker} {} {}", session.session_id, session.title),
            action: (session.session_id != workspace.session_id)
                .then(|| tui::OverlayAction::SwitchSession(session.session_id.clone())),
        });
    }
    lines
}

fn run_live_new_session_menu_command(
    args: &Args,
) -> Result<local::WorkspaceSummary, Box<dyn std::error::Error>> {
    let session_id = live_menu_new_session_id();
    let session_command = local::ControlCommandSummary {
        actor_id: args.actor_id.clone(),
        command_seq: 1,
        kind: protocol::ControlCommandKind::SessionNew,
        pane_id: None,
        tab_id: None,
        split_axis: protocol::SplitAxis::None,
        title: Some("new session".to_owned()),
        session_id: Some(session_id),
    };
    let stream = connect_to_daemon(args)?;
    match local::run_control_command_on_stream(stream, session_command) {
        Ok(workspace) => return Ok(workspace),
        Err(err) if live_session_new_should_fallback(err.as_ref()) => {}
        Err(err) => return Err(err),
    }

    run_live_new_tab_menu_command(args)
}

fn run_live_new_tab_menu_command(
    args: &Args,
) -> Result<local::WorkspaceSummary, Box<dyn std::error::Error>> {
    let command = local::ControlCommandSummary {
        actor_id: args.actor_id.clone(),
        command_seq: 1,
        kind: protocol::ControlCommandKind::TabNew,
        pane_id: None,
        tab_id: None,
        split_axis: protocol::SplitAxis::None,
        title: Some("new session".to_owned()),
        session_id: None,
    };
    let stream = connect_to_daemon(args)?;
    local::run_control_command_on_stream(stream, command)
}

fn live_session_new_should_fallback(error: &(dyn std::error::Error + 'static)) -> bool {
    const SINGLE_SESSION_REJECTION: &str = "session new requires daemon registry routing";
    if error
        .downcast_ref::<local::ServerError>()
        .is_some_and(|error| error.error.message.contains(SINGLE_SESSION_REJECTION))
    {
        return true;
    }
    error.to_string().contains(SINGLE_SESSION_REJECTION)
}

fn live_menu_new_session_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("session-{millis}")
}

fn run_live_tab_switch_menu_command(
    args: &Args,
    tab_id: &str,
) -> Result<local::WorkspaceSummary, Box<dyn std::error::Error>> {
    let command = local::ControlCommandSummary {
        actor_id: args.actor_id.clone(),
        command_seq: 1,
        kind: protocol::ControlCommandKind::TabSwitch,
        pane_id: None,
        tab_id: Some(tab_id.to_owned()),
        split_axis: protocol::SplitAxis::None,
        title: None,
        session_id: None,
    };
    let stream = connect_to_daemon(args)?;
    local::run_control_command_on_stream(stream, command)
}

#[allow(clippy::too_many_arguments)]
fn switch_live_session(
    args: &Args,
    options: &local::AttachOptions,
    client_state: &mut local::ClientAttachState,
    speculative_echo: &mut local::SpeculativeEchoOverlay,
    host_mouse_modes: &mut Option<HostMouseModeMirror>,
    client_inventory: &mut ClientInventoryCache,
    recorder: &mut LiveRecorder,
    socket_scope: Option<local::SocketIdentity>,
    session_id: &str,
    setup_read_timeout: Duration,
    live_socket_read_timeout: Duration,
    redraw_state: &mut Option<RedrawState>,
    use_styled: bool,
) -> Result<LiveSessionSwitch, Box<dyn std::error::Error>> {
    let mut stream = connect_to_daemon(args)?;
    stream.set_read_timeout(Some(setup_read_timeout))?;
    let mut switch_options = options.clone();
    switch_options.target_session_id = Some(session_id.to_owned());
    switch_options.request.known_surfaces = client_state.known_surfaces_for_scope(socket_scope);
    switch_options.known_scrollback_versions =
        client_state.known_scrollback_versions_for_scope(socket_scope);
    local::write_attach_request_for_session(
        &mut stream,
        &switch_options.request,
        Some(session_id),
    )?;
    let snapshot = local::attach_from_stream(&mut stream)?;
    let attached_pane_id = snapshot.status.pane_id.clone();
    client_state.apply_scope(socket_scope);
    let mut rendered = client_state.render_attach(snapshot)?;
    if use_styled && rendered.surface_text.is_some() {
        rendered.surface_text = client_state.cached_surface_text_styled(&attached_pane_id, true);
    }
    if rendered.surface_text.is_none() {
        rendered.surface_text =
            client_state.cached_surface_text_styled(&attached_pane_id, use_styled);
        if let Some(surface) = client_state.cached_surface_summary(&attached_pane_id) {
            rendered.surface_kind = surface.surface_kind;
            rendered.cursor = surface.cursor;
            rendered.modes = surface.modes;
        }
        rendered.surface_metadata = client_state
            .cached_surface_metadata(&attached_pane_id)
            .unwrap_or_default();
    }

    let workspace = rendered.workspace.clone();
    let initial_surface_text = rendered
        .surface_text
        .clone()
        .unwrap_or_else(|| workspace.display_line());
    let mut pane_surfaces = BTreeMap::new();
    pane_surfaces.insert(attached_pane_id.clone(), initial_surface_text.clone());
    seed_cached_pane_surfaces(&mut pane_surfaces, &workspace, client_state, use_styled);
    let mut pane_surface_summaries = BTreeMap::new();
    pane_surface_summaries.insert(attached_pane_id.clone(), rendered.surface.clone());
    seed_cached_pane_surface_summaries(&mut pane_surface_summaries, &workspace, client_state);
    let mut pane_modes = BTreeMap::new();
    pane_modes.insert(attached_pane_id.clone(), rendered.modes);
    seed_cached_pane_modes(&mut pane_modes, &workspace, client_state);
    let mut surface_state = LiveSurfaceState {
        current_surface_metadata: rendered.surface_metadata.clone(),
        current_modes: rendered.modes,
        current_surface_text: initial_surface_text,
        current_pane_surfaces: pane_surfaces,
        current_pane_surface_summaries: pane_surface_summaries,
        current_pane_modes: pane_modes,
        scrollback_views: BTreeMap::new(),
    };

    let mut client_sequence = local::ClientFrameSequence::default();
    let (scrollback, pending_surface_updates, pending_live_reads) = initial_live_scrollback(
        args,
        &mut stream,
        &mut client_sequence,
        &attached_pane_id,
        client_state,
        socket_scope,
    )?;
    for pending in pending_live_reads {
        match pending {
            local::LiveSurfaceRead::ClientInventorySnapshot(snapshot) => {
                client_inventory.apply_snapshot(snapshot);
            }
            local::LiveSurfaceRead::ClientInventoryPatch(patch) => {
                let _ = client_inventory.apply_patch(patch);
            }
            _ => {}
        }
    }
    if let Some(scrollback) = scrollback.as_ref() {
        client_state.cache_scrollback_chunk(scrollback);
    }
    if let Some(mouse_modes) = host_mouse_modes.as_mut() {
        mouse_modes.sync(surface_state.current_modes)?;
    }
    if args.output_json {
        let mut rendered = rendered;
        rendered.scrollback = scrollback;
        let event = format_live_attach_json(&rendered);
        recorder.record(&event)?;
        println!("{event}");
    } else {
        print_live_rendered(
            rendered,
            args.redraw,
            scrollback,
            redraw_state.as_mut(),
            Some(&surface_state.current_pane_surfaces),
            Some(&surface_state.current_pane_surface_summaries),
        );
    }
    recorder.record(&format_live_workspace_json(&workspace))?;
    for update in pending_surface_updates {
        process_surface_update(
            &update,
            &mut surface_state,
            speculative_echo,
            client_state,
            host_mouse_modes,
            redraw_state,
            recorder,
            &workspace,
            args,
            use_styled,
        )?;
    }
    stream.set_read_timeout(Some(live_socket_read_timeout))?;
    Ok(LiveSessionSwitch {
        stream,
        client_sequence,
        attached_pane_id,
        workspace,
        surface_state,
    })
}

#[allow(clippy::too_many_arguments)]
fn scroll_live_pane_view(
    stream: &mut UnixStream,
    sequence: &mut local::ClientFrameSequence,
    pane_id: &str,
    direction: LiveScrollDirection,
    visible_rows: u16,
    surface_state: &mut LiveSurfaceState,
    client_state: &mut local::ClientAttachState,
    speculative_echo: &mut local::SpeculativeEchoOverlay,
    host_mouse_modes: &mut Option<HostMouseModeMirror>,
    client_inventory: &mut ClientInventoryCache,
    recorder: &mut LiveRecorder,
    socket_scope: Option<local::SocketIdentity>,
    workspace: &local::WorkspaceSummary,
    args: &Args,
    mut redraw_state: Option<&mut RedrawState>,
    use_styled: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    if args.output_json || !args.redraw {
        return Ok(false);
    }
    let line_count = u32::from(visible_rows.max(1));
    let existing = surface_state.scrollback_views.get(pane_id).copied();
    let (start_line, tail_count) = match (direction, existing) {
        (LiveScrollDirection::Up, None) => (1, Some(line_count)),
        (LiveScrollDirection::Up, Some(view)) if view.start_line > 1 => (view.start_line - 1, None),
        (LiveScrollDirection::Up, Some(_)) => return Ok(false),
        (LiveScrollDirection::Down, None) => return Ok(false),
        (LiveScrollDirection::Down, Some(view)) => {
            let max_start = scrollback_max_start(view.total_lines, view.line_count);
            if view.start_line >= max_start {
                restore_live_pane_surface(
                    pane_id,
                    surface_state,
                    client_state,
                    workspace,
                    use_styled,
                );
                print_live_surface(
                    workspace,
                    &surface_state.current_surface_metadata,
                    &surface_state.current_surface_text,
                    args.redraw,
                    redraw_state.as_deref_mut(),
                    Some(&surface_state.current_pane_surfaces),
                    Some(&surface_state.current_pane_surface_summaries),
                );
                return Ok(true);
            }
            (view.start_line + 1, None)
        }
    };

    let mut pending_updates = Vec::new();
    let mut pending_live = Vec::new();
    let scrollback = local::fetch_scrollback_chunk_with_selection_and_pending_live(
        stream,
        sequence,
        pane_id,
        start_line,
        line_count,
        tail_count,
        |range_start, range_count| {
            client_state
                .cached_scrollback_version_for_scope(
                    socket_scope,
                    pane_id,
                    range_start,
                    range_count,
                )
                .unwrap_or(0)
        },
        Some(&mut pending_updates),
        Some(&mut pending_live),
    )?;
    for pending in pending_live {
        match pending {
            local::LiveSurfaceRead::ClientInventorySnapshot(snapshot) => {
                client_inventory.apply_snapshot(snapshot);
            }
            local::LiveSurfaceRead::ClientInventoryPatch(patch) => {
                let _ = client_inventory.apply_patch(patch);
            }
            _ => {}
        }
    }
    if let Some(state) = redraw_state.as_deref_mut() {
        state.record_client_count(client_inventory.count());
    }
    for update in pending_updates {
        speculative_echo.reconcile_update(&update);
        let update_metadata = local::TerminalMetadataSummary {
            title: update.title.clone(),
            working_directory: update.working_directory.clone(),
        };
        let update_surface_text = client_state.render_surface_update_styled(&update, use_styled)?;
        surface_state
            .current_pane_surfaces
            .insert(update.pane_id.clone(), update_surface_text.clone());
        if let Some(summary) = client_state.cached_rendered_surface_summary(&update.pane_id) {
            surface_state
                .current_pane_surface_summaries
                .insert(update.pane_id.clone(), summary);
        }
        surface_state
            .current_pane_modes
            .insert(update.pane_id.clone(), update.modes);
        surface_state.scrollback_views.remove(&update.pane_id);
        if update.pane_id == workspace.pane_id {
            surface_state.current_surface_metadata = update_metadata.clone();
            surface_state.current_modes = update.modes;
            if let Some(mouse_modes) = host_mouse_modes.as_mut() {
                mouse_modes.sync(surface_state.current_modes)?;
            }
            surface_state.current_surface_text = update_surface_text.clone();
        } else if let Some(active_text) =
            surface_state.current_pane_surfaces.get(&workspace.pane_id)
        {
            surface_state.current_surface_text = active_text.clone();
        }
        recorder.record(&format_live_surface_update_json(
            workspace,
            &update_metadata,
            &update_surface_text,
            &update,
        ))?;
    }
    if scrollback.lines.is_empty() {
        return Ok(false);
    }
    client_state.cache_scrollback_chunk(&scrollback);
    let rendered = render_scrollback_view_text(&scrollback);
    let rendered_summary = render_scrollback_view_summary(&scrollback);
    surface_state
        .current_pane_surfaces
        .insert(pane_id.to_owned(), rendered.clone());
    surface_state
        .current_pane_surface_summaries
        .insert(pane_id.to_owned(), rendered_summary);
    if pane_id == workspace.pane_id {
        surface_state.current_surface_text = rendered;
    }
    surface_state.scrollback_views.insert(
        pane_id.to_owned(),
        LiveScrollbackView {
            start_line: scrollback.start_line,
            line_count: u32::try_from(scrollback.lines.len()).unwrap_or(u32::MAX),
            total_lines: scrollback.total_lines,
        },
    );
    print_live_surface(
        workspace,
        &surface_state.current_surface_metadata,
        &surface_state.current_surface_text,
        args.redraw,
        redraw_state.as_deref_mut(),
        Some(&surface_state.current_pane_surfaces),
        Some(&surface_state.current_pane_surface_summaries),
    );
    Ok(true)
}

fn scrollback_max_start(total_lines: u64, line_count: u32) -> u64 {
    let line_count = u64::from(line_count.max(1));
    if total_lines > line_count {
        total_lines - line_count + 1
    } else {
        1
    }
}

fn restore_live_pane_surface(
    pane_id: &str,
    surface_state: &mut LiveSurfaceState,
    client_state: &local::ClientAttachState,
    workspace: &local::WorkspaceSummary,
    use_styled: bool,
) {
    surface_state.scrollback_views.remove(pane_id);
    let surface_text = client_state
        .cached_surface_text_styled(pane_id, use_styled)
        .unwrap_or_else(|| "(surface not cached)".to_owned());
    surface_state
        .current_pane_surfaces
        .insert(pane_id.to_owned(), surface_text.clone());
    if let Some(summary) = client_state.cached_rendered_surface_summary(pane_id) {
        surface_state
            .current_pane_surface_summaries
            .insert(pane_id.to_owned(), summary);
    }
    if pane_id == workspace.pane_id {
        surface_state.current_surface_text = surface_text;
    }
}

fn render_scrollback_view_text(scrollback: &local::ScrollbackChunkSummary) -> String {
    scrollback
        .lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_scrollback_view_summary(
    scrollback: &local::ScrollbackChunkSummary,
) -> local::RenderedSurfaceSummary {
    let row_updates = scrollback
        .lines
        .iter()
        .enumerate()
        .map(|(index, line)| local::SurfaceRowUpdate {
            row: u32::try_from(index).unwrap_or(u32::MAX),
            text: line.text.clone(),
            runs: line.runs.clone(),
            dirty_hash: line.dirty_hash,
            row_state_hash: line.row_state_hash,
            semantic_prompt: line.semantic_prompt,
            dirty: line.dirty,
            kitty_virtual_placeholder: line.kitty_virtual_placeholder,
        })
        .collect();
    let cols = scrollback
        .lines
        .iter()
        .map(|line| line.text.chars().count())
        .max()
        .unwrap_or(0);

    local::RenderedSurfaceSummary {
        pane_id: scrollback.pane_id.clone(),
        version: scrollback.scrollback_version,
        cols: u32::try_from(cols).unwrap_or(u32::MAX),
        rows: u32::try_from(scrollback.lines.len()).unwrap_or(u32::MAX),
        colors: scrollback.colors.clone(),
        styles: scrollback.styles.clone(),
        hyperlinks: scrollback.hyperlinks.clone(),
        row_updates,
    }
}

fn switch_live_surface_to_workspace_pane(
    workspace: &local::WorkspaceSummary,
    surface_state: &mut LiveSurfaceState,
    client_state: &local::ClientAttachState,
    host_mouse_modes: Option<&mut HostMouseModeMirror>,
    use_styled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let pane_id = &workspace.pane_id;
    let surface_text = client_state
        .cached_surface_text_styled(pane_id, use_styled)
        .unwrap_or_else(|| "(surface not cached)".to_owned());
    surface_state
        .current_pane_surfaces
        .insert(pane_id.clone(), surface_text.clone());
    if let Some(summary) = client_state.cached_rendered_surface_summary(pane_id) {
        surface_state
            .current_pane_surface_summaries
            .insert(pane_id.clone(), summary);
    }
    surface_state.scrollback_views.remove(pane_id);
    surface_state.current_surface_text = surface_text;
    surface_state.current_surface_metadata = client_state
        .cached_surface_metadata(pane_id)
        .unwrap_or_default();
    if let Some(surface) = client_state.cached_surface_summary(pane_id) {
        surface_state.current_modes = surface.modes;
        surface_state
            .current_pane_modes
            .insert(pane_id.clone(), surface.modes);
    } else {
        surface_state.current_modes = local::TerminalModeSummary::default();
        surface_state
            .current_pane_modes
            .insert(pane_id.clone(), local::TerminalModeSummary::default());
    }
    if let Some(mouse_modes) = host_mouse_modes {
        mouse_modes.sync(surface_state.current_modes)?;
    }
    Ok(())
}

fn focus_live_client_pane(
    pane_id: &str,
    attached_pane_id: &mut String,
    workspace: &mut local::WorkspaceSummary,
    surface_state: &mut LiveSurfaceState,
    client_state: &local::ClientAttachState,
    host_mouse_modes: Option<&mut HostMouseModeMirror>,
    redraw_state: Option<&mut RedrawState>,
    args: &Args,
    use_styled: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    if pane_id == attached_pane_id {
        return Ok(false);
    }
    if !workspace_panes(workspace)
        .iter()
        .any(|pane| pane.pane_id == pane_id)
    {
        return Ok(false);
    }

    *attached_pane_id = pane_id.to_owned();
    workspace.pane_id = pane_id.to_owned();
    switch_live_surface_to_workspace_pane(
        workspace,
        surface_state,
        client_state,
        host_mouse_modes,
        use_styled,
    )?;

    if args.output_json {
        return Ok(false);
    }
    if args.redraw {
        print_live_surface(
            workspace,
            &surface_state.current_surface_metadata,
            &surface_state.current_surface_text,
            args.redraw,
            redraw_state,
            Some(&surface_state.current_pane_surfaces),
            Some(&surface_state.current_pane_surface_summaries),
        );
    } else {
        println!("{}", workspace.display_line());
    }
    Ok(true)
}

fn preserve_live_client_focus(workspace: &mut local::WorkspaceSummary, pane_id: &str) {
    if workspace_panes(workspace)
        .iter()
        .any(|pane| pane.pane_id == pane_id)
    {
        workspace.pane_id = pane_id.to_owned();
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

struct RawTerminalGuard;

impl RawTerminalGuard {
    fn enable_if_needed(
        context: RawTerminalModeContext,
        local_echo: LocalEcho,
    ) -> io::Result<Option<Self>> {
        if !raw_terminal_mode_needed(context) {
            return Ok(None);
        }

        terminal::enable_raw_mode()?;
        if let Err(err) = apply_raw_terminal_fixups(local_echo) {
            let _ = terminal::disable_raw_mode();
            return Err(err);
        }

        Ok(Some(Self))
    }
}

impl Drop for RawTerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RawTerminalModeContext {
    stdin_bytes: bool,
    stdin_is_tty: bool,
}

fn raw_terminal_mode_needed(context: RawTerminalModeContext) -> bool {
    context.stdin_bytes && context.stdin_is_tty
}

struct RedrawTerminalGuard;

impl RedrawTerminalGuard {
    fn enable_if_needed(context: RedrawTerminalContext) -> io::Result<Option<Self>> {
        if !redraw_terminal_guard_needed(context) {
            return Ok(None);
        }

        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
        Ok(Some(Self))
    }
}

impl Drop for RedrawTerminalGuard {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        let _ = execute!(stdout, cursor::Show, LeaveAlternateScreen);
    }
}

struct HostMouseModeMirror {
    current: Option<local::TerminalModeSummary>,
}

impl HostMouseModeMirror {
    fn enable_if_needed(context: HostMouseModeContext) -> io::Result<Option<Self>> {
        if !host_mouse_mode_mirror_needed(context) {
            return Ok(None);
        }
        Ok(Some(Self { current: None }))
    }

    fn sync(&mut self, modes: local::TerminalModeSummary) -> io::Result<()> {
        if self.current == Some(modes) {
            return Ok(());
        }
        print!("{}", host_mouse_mode_disable_sequence());
        print!("{}", host_mouse_mode_enable_sequence(modes));
        flush_stdout()?;
        self.current = Some(modes);
        Ok(())
    }
}

impl Drop for HostMouseModeMirror {
    fn drop(&mut self) {
        print!("{}", host_mouse_mode_disable_sequence());
        let _ = flush_stdout();
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct HostMouseModeContext {
    stdin_bytes: bool,
    redraw: bool,
    stdout_is_tty: bool,
}

fn host_mouse_mode_mirror_needed(context: HostMouseModeContext) -> bool {
    context.stdin_bytes && context.redraw && context.stdout_is_tty
}

fn host_mouse_mode_disable_sequence() -> &'static str {
    "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1016l"
}

fn host_mouse_mode_enable_sequence(modes: local::TerminalModeSummary) -> &'static str {
    if !modes.mouse_tracking {
        return "\x1b[?1000h\x1b[?1006h";
    }
    match (modes.mouse_tracking_mode, modes.mouse_format) {
        (protocol::MouseTrackingMode::X10, protocol::MouseFormat::Sgr) => "\x1b[?1000h\x1b[?1006h",
        (protocol::MouseTrackingMode::Normal, protocol::MouseFormat::Sgr) => {
            "\x1b[?1000h\x1b[?1006h"
        }
        (protocol::MouseTrackingMode::Button, protocol::MouseFormat::Sgr) => {
            "\x1b[?1002h\x1b[?1006h"
        }
        (protocol::MouseTrackingMode::Any, protocol::MouseFormat::Sgr) => "\x1b[?1003h\x1b[?1006h",
        (protocol::MouseTrackingMode::X10, protocol::MouseFormat::SgrPixels) => {
            "\x1b[?1000h\x1b[?1006h\x1b[?1016h"
        }
        (protocol::MouseTrackingMode::Normal, protocol::MouseFormat::SgrPixels) => {
            "\x1b[?1000h\x1b[?1006h\x1b[?1016h"
        }
        (protocol::MouseTrackingMode::Button, protocol::MouseFormat::SgrPixels) => {
            "\x1b[?1002h\x1b[?1006h\x1b[?1016h"
        }
        (protocol::MouseTrackingMode::Any, protocol::MouseFormat::SgrPixels) => {
            "\x1b[?1003h\x1b[?1006h\x1b[?1016h"
        }
        (protocol::MouseTrackingMode::X10, _) => "\x1b[?1000h",
        (protocol::MouseTrackingMode::Normal, _) => "\x1b[?1000h",
        (protocol::MouseTrackingMode::Button, _) => "\x1b[?1002h",
        (protocol::MouseTrackingMode::Any, _) => "\x1b[?1003h",
        _ => "",
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RedrawTerminalContext {
    redraw: bool,
    stdout_is_tty: bool,
}

fn redraw_terminal_guard_needed(context: RedrawTerminalContext) -> bool {
    context.redraw && context.stdout_is_tty
}

struct SigwinchResize {
    _guard: Option<SigwinchGuard>,
    last_size: Option<(u32, u32)>,
}

impl SigwinchResize {
    fn enable_if_needed(context: SigwinchResizeContext) -> io::Result<Self> {
        if !sigwinch_resize_needed(context) {
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

        self.current_resize()
    }

    fn current_resize(&mut self) -> io::Result<Option<(u32, u32)>> {
        if self._guard.is_none() {
            return Ok(None);
        }
        let Some(size) = terminal_size()? else {
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
        let handler = handle_sigwinch as *const () as libc::sighandler_t;
        // SAFETY: installing a process signal handler is inherently global.
        // The handler only stores to an AtomicBool, which is signal-safe.
        let previous = unsafe { libc::signal(libc::SIGWINCH, handler) };
        if previous == libc::SIG_ERR {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { previous })
    }
}

impl Drop for SigwinchGuard {
    fn drop(&mut self) {
        // SAFETY: previous was returned by signal during install. Drop must not
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
    terminal_is_tty: bool,
}

fn sigwinch_resize_needed(context: SigwinchResizeContext) -> bool {
    context.stdin_bytes && !context.explicit_resize && context.terminal_is_tty
}

fn terminal_size() -> io::Result<Option<(u32, u32)>> {
    terminal_size_from_fds(
        &[
            io::stdout().as_raw_fd(),
            io::stdin().as_raw_fd(),
            io::stderr().as_raw_fd(),
        ],
        terminal::size,
    )
}

fn terminal_size_from_fds<F>(fds: &[i32], fallback: F) -> io::Result<Option<(u32, u32)>>
where
    F: FnOnce() -> io::Result<(u16, u16)>,
{
    for &fd in fds {
        if let Some(size) = terminal_size_from_fd(fd)? {
            return Ok(Some(size));
        }
    }

    let (cols, rows) = match fallback() {
        Ok(size) => size,
        Err(err) if terminal_size_unavailable(&err) => return Ok(None),
        Err(err) => return Err(err),
    };
    if cols == 0 || rows == 0 {
        return Ok(None);
    }
    Ok(Some((u32::from(cols), u32::from(rows))))
}

fn terminal_size_unavailable(err: &io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(
            libc::ENOTTY | libc::EBADF | libc::EINVAL | libc::EAGAIN | libc::ENODEV | libc::ENOENT
        )
    )
}

fn terminal_size_from_fd(fd: i32) -> io::Result<Option<(u32, u32)>> {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: size is a valid winsize struct and fd is an open file descriptor.
    let result = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut size) };
    if result < 0 {
        let err = io::Error::last_os_error();
        if terminal_size_unavailable(&err) {
            return Ok(None);
        }
        return Err(err);
    }
    if size.ws_col == 0 || size.ws_row == 0 {
        return Ok(None);
    }
    Ok(Some((u32::from(size.ws_col), u32::from(size.ws_row))))
}

fn apply_raw_terminal_fixups(local_echo: LocalEcho) -> io::Result<()> {
    let mut termios = read_stdin_termios()?;
    termios = raw_terminal_fixup_termios(termios, local_echo);

    // SAFETY: termios was fetched from STDIN_FILENO and only adjusted by this
    // process before being applied back to the same descriptor.
    if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &termios) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn read_stdin_termios() -> io::Result<libc::termios> {
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: STDIN_FILENO is a valid process file descriptor when raw mode is
    // enabled, and termios points to writable storage initialized by tcgetattr.
    if unsafe { libc::tcgetattr(libc::STDIN_FILENO, termios.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful tcgetattr initialized the termios storage.
    Ok(unsafe { termios.assume_init() })
}

fn raw_terminal_fixup_termios(mut termios: libc::termios, local_echo: LocalEcho) -> libc::termios {
    termios.c_iflag &= !libc::IXOFF;
    match local_echo {
        LocalEcho::Off => {}
        LocalEcho::Tty => {
            termios.c_lflag |= libc::ECHO;
        }
    }
    termios
}

fn stdin_is_tty() -> bool {
    io::stdin().is_tty()
}

fn stdout_is_tty() -> bool {
    io::stdout().is_tty()
}

fn attach_once(
    args: &Args,
    client_state: &mut local::ClientAttachState,
) -> Result<local::RenderedAttach, Box<dyn std::error::Error>> {
    let mut options = local::AttachOptions {
        target_session_id: args.target_session_id.clone(),
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
    apply_client_identity(args, &mut options.request);
    options.request.hostname = resolve_short_hostname();
    options.request.client_kind = "nmux".to_owned();
    options.request.subscribe_client_inventory = false;
    options.request.focused_pane_id = args
        .target_pane_id
        .clone()
        .or_else(|| args.target_tab_id.clone());
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

    let rendered = if args.tcp_endpoint.is_some() {
        let stream = connect_to_daemon(args)?;
        local::attach_render_once_from_stream(stream, options, client_state)
    } else {
        local::attach_render_once(&args.socket_path, options, client_state)
    }?;
    validate_target_session(args, &rendered.workspace.session_id)?;
    Ok(rendered)
}

fn validate_target_session(
    args: &Args,
    actual_session_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(expected) = args.target_session_id.as_deref() else {
        return Ok(());
    };
    if expected == actual_session_id {
        return Ok(());
    }
    Err(format!("target session {expected:?} is not served by this daemon; attached session is {actual_session_id:?}").into())
}

fn apply_client_identity(args: &Args, request: &mut local::AttachRequest) {
    request.actor_id.clone_from(&args.actor_id);
    request.user_id.clone_from(&args.user_id);
    request.display_name.clone_from(&args.display_name);
}

fn connect_to_daemon(args: &Args) -> Result<UnixStream, Box<dyn std::error::Error>> {
    if let Some(endpoint) = args.tcp_endpoint.as_deref() {
        let token = args
            .tcp_token
            .as_deref()
            .ok_or("remote TCP attach requires --token, --tcp-token, or NMUX_TOKEN")?;
        return match connect_timeout_duration(args) {
            Some(timeout) => local::connect_to_tcp_daemon_with_timeout(endpoint, token, timeout),
            None => local::connect_to_tcp_daemon(endpoint, token),
        };
    }
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
    redraw_state: Option<&mut RedrawState>,
    pane_surfaces: Option<&BTreeMap<String, String>>,
    pane_surface_summaries: Option<&BTreeMap<String, local::RenderedSurfaceSummary>>,
) {
    if redraw {
        let surface_text = rendered
            .surface_text
            .unwrap_or_else(|| rendered.workspace.display_line());
        if let Some(state) = redraw_state {
            if state.render_workspace(
                &rendered.workspace,
                &surface_text,
                pane_surfaces,
                pane_surface_summaries,
                None,
            ) {
                return;
            }
            let redraw_text = redraw_text_with_context(
                &rendered.workspace,
                &rendered.surface_metadata,
                &surface_text,
                initial_scrollback,
                false,
                pane_surfaces,
                None,
            );
            state.render_initial(&rendered.workspace, &redraw_text);
        } else {
            let redraw_text = redraw_text_with_context(
                &rendered.workspace,
                &rendered.surface_metadata,
                &surface_text,
                initial_scrollback,
                false,
                pane_surfaces,
                None,
            );
            redraw_terminal(&redraw_text);
        }
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
    redraw_state: Option<&mut RedrawState>,
    pane_surfaces: Option<&BTreeMap<String, String>>,
    pane_surface_summaries: Option<&BTreeMap<String, local::RenderedSurfaceSummary>>,
) {
    print_live_surface_with_overlay(
        workspace,
        metadata,
        surface_text,
        redraw,
        redraw_state,
        pane_surfaces,
        pane_surface_summaries,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn print_live_surface_with_overlay(
    workspace: &local::WorkspaceSummary,
    metadata: &local::TerminalMetadataSummary,
    surface_text: &str,
    redraw: bool,
    redraw_state: Option<&mut RedrawState>,
    pane_surfaces: Option<&BTreeMap<String, String>>,
    pane_surface_summaries: Option<&BTreeMap<String, local::RenderedSurfaceSummary>>,
    overlay: Option<&tui::TuiOverlay>,
) {
    if redraw {
        if let Some(state) = redraw_state {
            if state.render_workspace(
                workspace,
                surface_text,
                pane_surfaces,
                pane_surface_summaries,
                overlay,
            ) {
                return;
            }
            let text = redraw_text_with_context(
                workspace,
                metadata,
                surface_text,
                None,
                false,
                pane_surfaces,
                overlay,
            );
            state.render_diff(workspace, &text);
        } else {
            let text = redraw_text_with_context(
                workspace,
                metadata,
                surface_text,
                None,
                false,
                pane_surfaces,
                overlay,
            );
            redraw_terminal(&text);
        }
    } else {
        print_terminal_metadata(metadata);
        println!("{surface_text}");
    }
}

#[allow(clippy::too_many_arguments)]
fn print_live_update(
    workspace: &local::WorkspaceSummary,
    previous_metadata: &local::TerminalMetadataSummary,
    metadata: &local::TerminalMetadataSummary,
    surface_text: &str,
    update: &local::SurfaceUpdate,
    redraw: bool,
    redraw_state: Option<&mut RedrawState>,
    pane_surfaces: Option<&BTreeMap<String, String>>,
    pane_surface_summaries: Option<&BTreeMap<String, local::RenderedSurfaceSummary>>,
) {
    match live_update_print_kind(previous_metadata, metadata, update, redraw) {
        LiveUpdatePrintKind::Surface => print_live_surface(
            workspace,
            metadata,
            surface_text,
            redraw,
            redraw_state,
            pane_surfaces,
            pane_surface_summaries,
        ),
        LiveUpdatePrintKind::Metadata => {
            if redraw {
                if let Some(state) = redraw_state {
                    let _ = state.render_workspace(
                        workspace,
                        surface_text,
                        pane_surfaces,
                        pane_surface_summaries,
                        None,
                    );
                }
            } else {
                print_terminal_metadata(metadata);
            }
        }
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
    if update.kind == local::SurfaceUpdateKind::Snapshot
        || update.patch_kind == Some(protocol::PatchKind::ReplaceRows)
    {
        LiveUpdatePrintKind::Surface
    } else if redraw && metadata != previous_metadata {
        // In redraw mode without a status bar, metadata changes trigger a
        // differential repaint. Cursor-only and mode-only patches with
        // unchanged metadata are no-ops.
        LiveUpdatePrintKind::Metadata
    } else if !redraw && metadata != previous_metadata {
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

/// Frame statistics for the status bar.
#[derive(Debug, Clone, Default)]
struct FrameStats {
    /// Time spent decoding the protocol update and applying it to client state.
    decode_time: Duration,
    /// Time spent diffing rows and writing ANSI output.
    render_time: Duration,
    /// Number of rows rewritten in this frame.
    rows_changed: usize,
    /// Total rows in the surface.
    rows_total: usize,
    /// Most recently observed client/server ping round-trip time.
    rtt: Option<Duration>,
    /// Most recently observed subscribed live client count.
    client_count: Option<usize>,
    /// Rolling FPS for frames that changed visible content.
    rendered_fps: Option<u32>,
    /// Whether the latest frame had no visible content changes.
    idle: bool,
}

#[derive(Debug, Clone)]
struct RenderedFps {
    window: Duration,
    rendered_at: VecDeque<Instant>,
}

impl Default for RenderedFps {
    fn default() -> Self {
        Self {
            window: STATUS_FPS_WINDOW,
            rendered_at: VecDeque::new(),
        }
    }
}

impl RenderedFps {
    fn record(&mut self, now: Instant, rendered: bool) -> Option<u32> {
        if rendered {
            self.rendered_at.push_back(now);
        }
        while self
            .rendered_at
            .front()
            .is_some_and(|rendered_at| now.duration_since(*rendered_at) > self.window)
        {
            self.rendered_at.pop_front();
        }
        if self.rendered_at.is_empty() {
            return None;
        }
        let fps = (self.rendered_at.len() as u128 * 1000) / self.window.as_millis().max(1);
        Some(fps.max(1).min(u128::from(u32::MAX)) as u32)
    }
}

#[derive(Debug)]
struct RttTracker {
    actor_id: String,
    pending: Option<PendingPing>,
    next_ping_at: Instant,
}

#[derive(Debug)]
struct PendingPing {
    seq: u64,
    sent_at: Instant,
}

impl RttTracker {
    fn new(actor_id: String) -> Self {
        Self {
            actor_id,
            pending: None,
            next_ping_at: Instant::now(),
        }
    }

    fn maybe_send_ping(
        &mut self,
        stream: &mut UnixStream,
        sequence: &mut local::ClientFrameSequence,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let now = Instant::now();
        if let Some(pending) = self.pending.as_ref()
            && now.duration_since(pending.sent_at) < LIVE_RTT_PING_TIMEOUT
        {
            return Ok(());
        }
        if now < self.next_ping_at {
            return Ok(());
        }
        let seq = local::send_ping_with_sequence(stream, sequence, &self.actor_id)?;
        self.pending = Some(PendingPing { seq, sent_at: now });
        self.next_ping_at = now + LIVE_RTT_PING_INTERVAL;
        Ok(())
    }

    fn record_pong(&mut self, pong: &local::PingSummary) -> Option<Duration> {
        if pong.actor_id != self.actor_id {
            return None;
        }
        let pending = self.pending.as_ref()?;
        if pong.ping_seq != pending.seq {
            return None;
        }
        let rtt = pending.sent_at.elapsed();
        self.pending = None;
        Some(rtt)
    }
}

#[derive(Debug, Default)]
struct ClientInventoryCache {
    version: u64,
    clients: BTreeMap<String, local::ClientConnectionSummary>,
}

impl ClientInventoryCache {
    fn apply_snapshot(&mut self, snapshot: local::ClientInventorySnapshotSummary) {
        self.version = snapshot.version;
        self.clients = snapshot
            .clients
            .into_iter()
            .map(|client| (client.connection_id.clone(), client))
            .collect();
    }

    fn apply_patch(
        &mut self,
        patch: local::ClientInventoryPatchSummary,
    ) -> Result<(), &'static str> {
        if patch.base_version != self.version {
            return Err("client inventory patch base version mismatch");
        }
        for connection_id in patch.left_connection_ids {
            self.clients.remove(&connection_id);
        }
        for client in patch.joined.into_iter().chain(patch.updated) {
            self.clients.insert(client.connection_id.clone(), client);
        }
        self.version = patch.version;
        Ok(())
    }

    fn count(&self) -> usize {
        self.clients.len()
    }
}

/// Tracks displayed rows for differential rendering with a status bar.
struct RedrawState {
    /// Ratatui terminal compositor for interactive redraw on real TTYs.
    terminal: Option<Terminal<CrosstermBackend<io::Stdout>>>,
    /// Previously displayed rows (content rows, then the bottom status bar).
    previous_rows: Vec<String>,
    /// Terminal width for status bar formatting.
    terminal_cols: u32,
    /// Terminal height for detecting resize-induced screen reflow.
    terminal_rows: u32,
    /// When the last frame was rendered.
    last_frame_time: Instant,
    /// Most recent frame statistics.
    last_stats: FrameStats,
    /// Pending decode time set before render_diff is called.
    pending_decode_time: Duration,
    /// Most recently observed ping round-trip time.
    last_rtt: Option<Duration>,
    /// Most recently observed subscribed live client count.
    last_client_count: Option<usize>,
    /// Rolling rate for frames that changed visible content.
    rendered_fps: RenderedFps,
}

impl RedrawState {
    fn new() -> Self {
        Self {
            terminal: None,
            previous_rows: Vec::new(),
            terminal_cols: 80,
            terminal_rows: 24,
            last_frame_time: Instant::now(),
            last_stats: FrameStats::default(),
            pending_decode_time: Duration::ZERO,
            last_rtt: None,
            last_client_count: None,
            rendered_fps: RenderedFps::default(),
        }
    }

    fn new_with_terminal() -> io::Result<Self> {
        let mut state = Self::new();
        let area = redraw_terminal_area();
        state.terminal_cols = u32::from(area.width);
        state.terminal_rows = u32::from(area.height);
        clear_redraw_terminal()?;
        state.terminal = Some(Terminal::with_options(
            CrosstermBackend::new(io::stdout()),
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )?);
        Ok(state)
    }

    fn update_terminal_size(&mut self) -> bool {
        if let Ok(Some((cols, rows))) = terminal_size() {
            let changed = self.terminal_cols != cols || self.terminal_rows != rows;
            self.terminal_cols = cols;
            self.terminal_rows = rows;
            return changed;
        }
        false
    }

    /// Record decode time so the next render_diff can include it in stats.
    fn record_decode_time(&mut self, decode_time: Duration) {
        self.pending_decode_time = decode_time;
    }

    fn record_rtt(&mut self, rtt: Duration) {
        self.last_rtt = Some(rtt);
    }

    fn record_client_count(&mut self, count: usize) {
        self.last_client_count = Some(count);
        self.last_stats.client_count = Some(count);
    }

    fn render_workspace(
        &mut self,
        workspace: &local::WorkspaceSummary,
        surface_text: &str,
        pane_surfaces: Option<&BTreeMap<String, String>>,
        pane_surface_summaries: Option<&BTreeMap<String, local::RenderedSurfaceSummary>>,
        overlay: Option<&tui::TuiOverlay>,
    ) -> bool {
        let render_start = Instant::now();
        let area = redraw_terminal_area();
        let resized = self.terminal_cols != u32::from(area.width)
            || self.terminal_rows != u32::from(area.height);
        let workspace_height = area.height.saturating_sub(1).max(1);
        let workspace_text = tui::render_workspace_frame(
            tui::WorkspaceFrameInput {
                workspace,
                active_surface_text: surface_text,
                pane_surfaces,
                pane_surface_summaries,
                pane_chrome: None,
                overlay,
            },
            area.width.max(1),
            workspace_height,
        )
        .text;
        let content_rows: Vec<String> = workspace_text.lines().map(String::from).collect();
        let previous_content_rows = self.previous_rows.len().saturating_sub(1);
        let mut rows_changed = 0;
        for index in 0..content_rows.len().max(previous_content_rows) {
            let new_row = content_rows.get(index).map(String::as_str).unwrap_or("");
            let old_row = self
                .previous_rows
                .get(index)
                .map(String::as_str)
                .unwrap_or("");
            if new_row != old_row {
                rows_changed += 1;
            }
        }
        let rendered_fps = self.rendered_fps.record(render_start, rows_changed > 0);
        let render_time = render_start.elapsed();
        self.last_stats = FrameStats {
            decode_time: self.pending_decode_time,
            render_time,
            rows_changed,
            rows_total: content_rows.len(),
            rtt: self.last_rtt,
            client_count: self.last_client_count,
            rendered_fps,
            idle: rows_changed == 0,
        };
        let status_text = self.format_status_text(workspace);
        let Some(terminal) = self.terminal.as_mut() else {
            return false;
        };
        if resized {
            let _ = clear_redraw_terminal();
            let _ = terminal.resize(area);
            let _ = terminal.clear();
        }

        let draw_result = terminal.draw(|frame| {
            let area = frame.area();
            if area.width == 0 || area.height == 0 {
                return;
            }
            let workspace_area = Rect::new(area.x, area.y, area.width, workspace_height);
            tui::render_workspace_to_buffer(
                frame.buffer_mut(),
                workspace_area,
                tui::WorkspaceFrameInput {
                    workspace,
                    active_surface_text: surface_text,
                    pane_surfaces,
                    pane_surface_summaries,
                    pane_chrome: None,
                    overlay,
                },
            );
            if area.height > 1 {
                let status_area = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
                render_ratatui_status_bar(frame.buffer_mut(), status_area, &status_text);
            }
        });

        let Ok(completed) = draw_result else {
            return false;
        };

        self.terminal_cols = u32::from(completed.area.width);
        self.terminal_rows = u32::from(completed.area.height);
        self.pending_decode_time = Duration::ZERO;
        self.last_frame_time = Instant::now();
        let mut all_rows = content_rows;
        all_rows.push(status_text);
        self.previous_rows = all_rows;
        true
    }

    fn format_status_text(&self, workspace: &local::WorkspaceSummary) -> String {
        // Leave the final terminal column untouched. Printing a full-width
        // line on the bottom row can set the terminal's autowrap state and
        // scroll the alternate screen on the next write, which pushes the
        // ratatui menu off the top of the viewport.
        let width = self.terminal_cols.saturating_sub(1).max(1) as usize;

        let identity = fixed_status_field(&workspace.pane_id, 6);
        let size = fixed_status_field(&format!("{}x{}", workspace.cols, workspace.rows), 5);
        let left = format!(" nmux {identity} {size}");

        let right = format_stats_right(&self.last_stats);

        let content_len = left.len() + right.len();
        if width > content_len {
            format!(
                "{left}{:padding$}{right}",
                "",
                padding = width - content_len
            )
        } else {
            truncate_chars(&format!("{left} {right}"), width)
        }
    }

    /// Build the full-width inverse-video status bar line for the legacy
    /// string redraw fallback.
    fn format_status_bar(&self, workspace: &local::WorkspaceSummary) -> String {
        let content = self.format_status_text(workspace);

        format!(
            "{}{content}{}",
            SetAttribute(Attribute::Reverse),
            SetAttribute(Attribute::NoReverse)
        )
    }

    /// Render the full surface text differentially: only write rows that changed.
    /// The status bar occupies the final terminal row; content starts at row 1.
    fn render_diff(&mut self, workspace: &local::WorkspaceSummary, surface_text: &str) {
        print!("{}", self.render_diff_text(workspace, surface_text));
    }

    fn render_diff_text(
        &mut self,
        workspace: &local::WorkspaceSummary,
        surface_text: &str,
    ) -> String {
        let render_start = Instant::now();
        if self.update_terminal_size() && !self.previous_rows.is_empty() {
            return self.render_initial_text(workspace, surface_text);
        }

        let content_rows: Vec<String> = surface_text.lines().map(String::from).collect();
        let mut output = String::new();
        let mut rows_changed: usize = 0;

        // Hide cursor during update to avoid flicker.
        output.push_str(&format!("{}", cursor::Hide));

        // Diff content rows (starting at terminal row 1).
        let max_content = content_rows
            .len()
            .max(self.previous_rows.len().saturating_sub(1));
        for i in 0..max_content {
            let new_row = content_rows.get(i).map(String::as_str).unwrap_or("");
            let old_row = self.previous_rows.get(i).map(String::as_str).unwrap_or("");
            if new_row != old_row {
                output.push_str(&format!(
                    "{}{}{}",
                    cursor::MoveTo(0, terminal_row(i + 1)),
                    Clear(ClearType::CurrentLine),
                    new_row
                ));
                rows_changed += 1;
            }
        }

        let render_time = render_start.elapsed();
        let rendered_fps = self.rendered_fps.record(render_start, rows_changed > 0);

        self.last_stats = FrameStats {
            decode_time: self.pending_decode_time,
            render_time,
            rows_changed,
            rows_total: content_rows.len(),
            rtt: self.last_rtt,
            client_count: self.last_client_count,
            rendered_fps,
            idle: rows_changed == 0,
        };
        self.pending_decode_time = Duration::ZERO;
        self.last_frame_time = Instant::now();

        // Always redraw the status bar since stats change every frame.
        let status_bar = self.format_status_bar(workspace);
        let status_row = self.status_row();
        output.push_str(&format!(
            "{}{}{}",
            cursor::MoveTo(0, status_row),
            Clear(ClearType::CurrentLine),
            status_bar
        ));

        output.push_str(&format!("{}", cursor::MoveTo(0, status_row)));

        // Store content rows + status bar for next diff.
        let mut all_rows = Vec::with_capacity(content_rows.len() + 1);
        all_rows.extend(content_rows);
        all_rows.push(status_bar);
        self.previous_rows = all_rows;

        output
    }

    fn render_speculative_append_text(
        &mut self,
        workspace: &local::WorkspaceSummary,
        surface_text: &str,
        prediction: &local::SpeculativeEchoPrediction,
    ) -> Option<String> {
        if workspace.pane_id != prediction.pane_id {
            return None;
        }
        if workspace
            .pane_tree
            .as_ref()
            .is_some_and(|root| !root.children.is_empty())
        {
            return None;
        }
        let row_index = usize::try_from(prediction.row).ok()?;
        if self.previous_rows.len() <= row_index {
            return None;
        }
        let row_text = surface_text.lines().nth(row_index)?;
        self.previous_rows[row_index] = row_text.to_owned();
        self.last_frame_time = Instant::now();
        self.last_stats.rows_changed = 1;
        self.last_stats.rows_total = surface_text.lines().count();

        let terminal_row_index = terminal_row(row_index + 1);
        let terminal_col = prediction.col.min(u16::MAX as u32) as u16;
        let status_row = self.status_row();
        let mut output = String::new();
        output.push_str(&format!(
            "{}{}{}{}",
            cursor::MoveTo(terminal_col, terminal_row_index),
            SetAttribute(Attribute::Underlined),
            prediction.text,
            SetAttribute(Attribute::NoUnderline)
        ));
        output.push_str(&format!("{}", cursor::MoveTo(0, status_row)));
        Some(output)
    }

    /// Full repaint for initial frame (no previous state to diff against).
    fn render_initial(&mut self, workspace: &local::WorkspaceSummary, surface_text: &str) {
        print!("{}", self.render_initial_text(workspace, surface_text));
    }

    fn render_initial_text(
        &mut self,
        workspace: &local::WorkspaceSummary,
        surface_text: &str,
    ) -> String {
        self.last_frame_time = Instant::now();
        self.last_stats = FrameStats::default();
        self.update_terminal_size();

        let content_rows: Vec<String> = surface_text.lines().map(String::from).collect();
        self.last_stats.rows_changed = content_rows.len();
        self.last_stats.rows_total = content_rows.len();
        self.last_stats.rtt = self.last_rtt;
        self.last_stats.client_count = self.last_client_count;
        self.last_stats.rendered_fps = self.rendered_fps.record(self.last_frame_time, true);
        self.last_stats.idle = false;

        let status_bar = self.format_status_bar(workspace);

        // Clear screen, draw content from row 1, then keep the status bar on
        // the final row so row 1 remains available for ratatui menus.
        let mut output = String::new();
        let status_row = self.status_row();
        output.push_str(&format!(
            "{}{}{}{}{}{}{}",
            Clear(ClearType::All),
            cursor::MoveTo(0, 0),
            surface_text,
            cursor::MoveTo(0, status_row),
            Clear(ClearType::CurrentLine),
            status_bar,
            cursor::MoveTo(0, status_row)
        ));

        let mut all_rows = Vec::with_capacity(content_rows.len() + 1);
        all_rows.extend(content_rows);
        all_rows.push(status_bar);
        self.previous_rows = all_rows;

        output
    }

    fn status_row(&self) -> u16 {
        self.terminal_rows.saturating_sub(1).min(u16::MAX as u32) as u16
    }
}

fn terminal_row(row_1_based: usize) -> u16 {
    row_1_based.saturating_sub(1).min(u16::MAX as usize) as u16
}

fn truncate_chars(value: &str, width: usize) -> String {
    value.chars().take(width).collect()
}

fn fixed_status_field(value: &str, width: usize) -> String {
    let truncated = truncate_chars(value, width);
    format!("{truncated:<width$}")
}

fn redraw_terminal_area() -> Rect {
    let (cols, rows) = terminal_size().ok().flatten().unwrap_or((80, 24));
    Rect::new(
        0,
        0,
        cols.max(1).min(u16::MAX as u32) as u16,
        rows.max(1).min(u16::MAX as u32) as u16,
    )
}

fn clear_redraw_terminal() -> io::Result<()> {
    execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0))
}

fn render_ratatui_status_bar(buffer: &mut Buffer, area: Rect, text: &str) {
    let style = Style::default()
        .fg(Color::Black)
        .bg(Color::White)
        .add_modifier(Modifier::REVERSED);
    let width = area.width.saturating_sub(1).max(1);
    for x in area.x..area.x.saturating_add(width) {
        if let Some(cell) = buffer.cell_mut((x, area.y)) {
            cell.set_symbol(" ");
            cell.set_style(style);
        }
    }
    for (offset, ch) in text.chars().take(usize::from(width)).enumerate() {
        if let Some(cell) = buffer.cell_mut((area.x + offset as u16, area.y)) {
            cell.set_symbol(&ch.to_string());
            cell.set_style(style);
        }
    }
}

fn resolve_short_hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: buf is a valid fixed-size buffer and gethostname writes a NUL-terminated string into it.
    let c_hostname = unsafe {
        if libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) == 0 {
            let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..len]).into_owned()
        } else {
            "unknown".to_owned()
        }
    };
    // Use short hostname (before first dot).
    c_hostname
        .split('.')
        .next()
        .unwrap_or(&c_hostname)
        .to_owned()
}

fn format_stats_right(stats: &FrameStats) -> String {
    let decode = fixed_status_field(&format_duration_short(stats.decode_time.as_micros()), 6);
    let render = fixed_status_field(&format_duration_short(stats.render_time.as_micros()), 6);
    let rows = fixed_status_field(&format!("{}/{}", stats.rows_changed, stats.rows_total), 5);
    let fps = if stats.idle {
        "idle".to_owned()
    } else {
        stats
            .rendered_fps
            .map(|fps| format!("{fps:>3}fps"))
            .unwrap_or_else(|| "  0fps".to_owned())
    };
    let fps = fixed_status_field(&fps, 6);
    let rtt = stats
        .rtt
        .map(|rtt| format_duration_short(rtt.as_micros()))
        .unwrap_or_else(|| "-".to_owned());
    let rtt = fixed_status_field(&rtt, 4);
    let clients = stats
        .client_count
        .map(|count| count.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let clients = fixed_status_field(&clients, 1);
    format!("rows:{rows} clients:{clients} rtt:{rtt} fps:{fps} decode:{decode} render:{render} ")
}

fn format_duration_short(micros: u128) -> String {
    if micros < 1000 {
        format!("{micros}us")
    } else {
        format!("{}ms", micros / 1000)
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
    has_status_bar: bool,
    pane_surfaces: Option<&BTreeMap<String, String>>,
    overlay: Option<&tui::TuiOverlay>,
) -> String {
    if has_status_bar {
        let (cols, rows) = terminal_size().ok().flatten().unwrap_or((80, 24));
        let rows = rows.saturating_sub(1).max(1).min(u16::MAX as u32) as u16;
        let cols = cols.max(1).min(u16::MAX as u32) as u16;
        return tui::render_workspace_frame(
            tui::WorkspaceFrameInput {
                workspace,
                active_surface_text: surface_text,
                pane_surfaces,
                pane_surface_summaries: None,
                pane_chrome: None,
                overlay,
            },
            cols,
            rows,
        )
        .text;
    }

    let mut text = String::new();
    // Without a status bar, include workspace info as a header line.
    text.push_str(&workspace.display_line());
    text.push('\n');
    append_terminal_metadata(&mut text, metadata);
    if let Some(scrollback) = scrollback {
        text.push_str(&format_scrollback(&scrollback));
    }
    text.push_str(&redraw_workspace_surface_text(
        workspace,
        surface_text,
        pane_surfaces,
    ));
    text
}

fn redraw_workspace_surface_text(
    workspace: &local::WorkspaceSummary,
    active_surface_text: &str,
    pane_surfaces: Option<&BTreeMap<String, String>>,
) -> String {
    let Some(root) = workspace.pane_tree.as_ref() else {
        return active_surface_text.to_owned();
    };
    if root.children.is_empty() {
        return active_surface_text.to_owned();
    }
    render_redraw_pane_node(root, workspace, active_surface_text, pane_surfaces).join("\n")
}

fn render_redraw_pane_node(
    pane: &local::WorkspacePaneSummary,
    workspace: &local::WorkspaceSummary,
    active_surface_text: &str,
    pane_surfaces: Option<&BTreeMap<String, String>>,
) -> Vec<String> {
    if pane.children.is_empty() {
        return render_redraw_leaf_pane(pane, workspace, active_surface_text, pane_surfaces);
    }

    let child_blocks: Vec<Vec<String>> = pane
        .children
        .iter()
        .map(|child| render_redraw_pane_node(child, workspace, active_surface_text, pane_surfaces))
        .collect();

    match pane.split_axis {
        protocol::SplitAxis::Vertical => join_redraw_blocks_vertical(&child_blocks),
        protocol::SplitAxis::Horizontal => join_redraw_blocks_horizontal(&child_blocks),
        _ => child_blocks.into_iter().flatten().collect(),
    }
}

fn render_redraw_leaf_pane(
    pane: &local::WorkspacePaneSummary,
    workspace: &local::WorkspaceSummary,
    active_surface_text: &str,
    pane_surfaces: Option<&BTreeMap<String, String>>,
) -> Vec<String> {
    let active = pane.pane_id == workspace.pane_id;
    let header = if active {
        format!("[{} active]", pane.pane_id)
    } else {
        format!("[{}]", pane.pane_id)
    };
    let body = if active {
        active_surface_text
    } else {
        pane_surfaces
            .and_then(|surfaces| surfaces.get(&pane.pane_id).map(String::as_str))
            .unwrap_or("(surface not cached)")
    };

    let mut lines = vec![header];
    lines.extend(body.lines().map(String::from));
    lines
}

fn join_redraw_blocks_horizontal(blocks: &[Vec<String>]) -> Vec<String> {
    let mut lines = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            let width = block
                .iter()
                .map(|line| visible_width(line))
                .max()
                .unwrap_or(1)
                .max(1);
            lines.push("-".repeat(width));
        }
        lines.extend(block.iter().cloned());
    }
    lines
}

fn join_redraw_blocks_vertical(blocks: &[Vec<String>]) -> Vec<String> {
    let widths: Vec<usize> = blocks
        .iter()
        .map(|block| {
            block
                .iter()
                .map(|line| visible_width(line))
                .max()
                .unwrap_or(1)
        })
        .collect();
    let height = blocks.iter().map(Vec::len).max().unwrap_or(0);
    let mut lines = Vec::with_capacity(height);

    for row in 0..height {
        let mut line = String::new();
        for (index, block) in blocks.iter().enumerate() {
            if index > 0 {
                line.push_str(" | ");
            }
            let cell = block.get(row).map(String::as_str).unwrap_or("");
            line.push_str(cell);
            let padding = widths[index].saturating_sub(visible_width(cell));
            line.push_str(&" ".repeat(padding));
        }
        lines.push(line);
    }

    lines
}

fn visible_width(line: &str) -> usize {
    line.chars().count()
}

fn seed_cached_pane_surfaces(
    surfaces: &mut BTreeMap<String, String>,
    workspace: &local::WorkspaceSummary,
    client_state: &local::ClientAttachState,
    styled: bool,
) {
    if let Some(root) = workspace.pane_tree.as_ref() {
        seed_cached_pane_surfaces_from_node(surfaces, root, client_state, styled);
    } else if !surfaces.contains_key(&workspace.pane_id)
        && let Some(text) = client_state.cached_surface_text_styled(&workspace.pane_id, styled)
    {
        surfaces.insert(workspace.pane_id.clone(), text);
    }
}

fn seed_cached_pane_surfaces_from_node(
    surfaces: &mut BTreeMap<String, String>,
    pane: &local::WorkspacePaneSummary,
    client_state: &local::ClientAttachState,
    styled: bool,
) {
    if pane.children.is_empty() {
        if !surfaces.contains_key(&pane.pane_id)
            && let Some(text) = client_state.cached_surface_text_styled(&pane.pane_id, styled)
        {
            surfaces.insert(pane.pane_id.clone(), text);
        }
        return;
    }

    for child in &pane.children {
        seed_cached_pane_surfaces_from_node(surfaces, child, client_state, styled);
    }
}

fn seed_cached_pane_surface_summaries(
    surfaces: &mut BTreeMap<String, local::RenderedSurfaceSummary>,
    workspace: &local::WorkspaceSummary,
    client_state: &local::ClientAttachState,
) {
    if let Some(root) = workspace.pane_tree.as_ref() {
        seed_cached_pane_surface_summaries_from_node(surfaces, root, client_state);
    } else if !surfaces.contains_key(&workspace.pane_id)
        && let Some(summary) = client_state.cached_rendered_surface_summary(&workspace.pane_id)
    {
        surfaces.insert(workspace.pane_id.clone(), summary);
    }
}

fn seed_cached_pane_surface_summaries_from_node(
    surfaces: &mut BTreeMap<String, local::RenderedSurfaceSummary>,
    pane: &local::WorkspacePaneSummary,
    client_state: &local::ClientAttachState,
) {
    if pane.children.is_empty() {
        if !surfaces.contains_key(&pane.pane_id)
            && let Some(summary) = client_state.cached_rendered_surface_summary(&pane.pane_id)
        {
            surfaces.insert(pane.pane_id.clone(), summary);
        }
        return;
    }

    for child in &pane.children {
        seed_cached_pane_surface_summaries_from_node(surfaces, child, client_state);
    }
}

fn seed_cached_pane_modes(
    modes: &mut BTreeMap<String, local::TerminalModeSummary>,
    workspace: &local::WorkspaceSummary,
    client_state: &local::ClientAttachState,
) {
    if let Some(root) = workspace.pane_tree.as_ref() {
        seed_cached_pane_modes_from_node(modes, root, client_state);
    } else if !modes.contains_key(&workspace.pane_id)
        && let Some(mode_summary) = client_state.cached_surface_modes(&workspace.pane_id)
    {
        modes.insert(workspace.pane_id.clone(), mode_summary);
    }
}

fn seed_cached_pane_modes_from_node(
    modes: &mut BTreeMap<String, local::TerminalModeSummary>,
    pane: &local::WorkspacePaneSummary,
    client_state: &local::ClientAttachState,
) {
    if pane.children.is_empty() {
        if !modes.contains_key(&pane.pane_id)
            && let Some(mode_summary) = client_state.cached_surface_modes(&pane.pane_id)
        {
            modes.insert(pane.pane_id.clone(), mode_summary);
        }
        return;
    }

    for child in &pane.children {
        seed_cached_pane_modes_from_node(modes, child, client_state);
    }
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
    target_session_id: Option<String>,
    tcp_endpoint: Option<String>,
    tcp_token: Option<String>,
    target_pane_id: Option<String>,
    target_tab_id: Option<String>,
    actor_id: String,
    user_id: String,
    display_name: String,
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
    record_path: Option<PathBuf>,
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
    detach_key: DetachKey,
    redraw: bool,
    speculative_echo: bool,
    live_resize: Option<(u32, u32)>,
    interval_ms: u64,
    connect_timeout_ms: Option<u64>,
    iterations: Option<usize>,
    script_command: Option<ScriptCommand>,
    script_split_axis: protocol::SplitAxis,
    script_title: Option<String>,
    replay_path: Option<PathBuf>,
    auto_default: bool,
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
    #[arg(
        short = 's',
        long = "session",
        value_name = "NAME",
        allow_hyphen_values = true
    )]
    target_session_id: Option<String>,
    #[arg(long = "tcp", value_name = "HOST:PORT", allow_hyphen_values = true)]
    tcp_endpoint: Option<String>,
    #[arg(
        long = "tcp-token",
        alias = "token",
        value_name = "TOKEN",
        allow_hyphen_values = true
    )]
    tcp_token: Option<String>,
    #[arg(long = "pane", value_name = "PANE_ID", allow_hyphen_values = true)]
    target_pane_id: Option<String>,
    #[arg(long = "tab", value_name = "TAB_ID", allow_hyphen_values = true)]
    target_tab_id: Option<String>,
    #[arg(long = "actor-id", value_name = "ID", allow_hyphen_values = true)]
    actor_id: Option<String>,
    #[arg(long = "user-id", value_name = "ID", allow_hyphen_values = true)]
    user_id: Option<String>,
    #[arg(long = "display-name", value_name = "NAME", allow_hyphen_values = true)]
    display_name: Option<String>,
    #[arg(long = "key", value_name = "TEXT", allow_hyphen_values = true)]
    key_text: Option<String>,
    #[arg(
        long = "key-name",
        value_name = "NAME",
        value_parser = parse_key_name,
        action = ArgAction::Append,
        allow_hyphen_values = true
    )]
    key_names: Vec<String>,
    #[arg(
        long = "key-modifiers",
        value_name = "MODS",
        value_parser = parse_key_modifiers_for_clap,
        allow_hyphen_values = true
    )]
    key_modifiers: Option<u32>,
    #[arg(long = "paste", value_name = "TEXT", allow_hyphen_values = true)]
    paste_text: Option<String>,
    #[arg(long = "focus", value_name = "gained|lost", allow_hyphen_values = true)]
    focus_event: Option<FocusEvent>,
    #[arg(
        long = "mouse",
        value_name = "action:button:row:col",
        value_parser = parse_mouse_event,
        allow_hyphen_values = true
    )]
    mouse_event: Option<MouseEvent>,
    #[arg(
        long = "mouse-modifiers",
        value_name = "MODS",
        value_parser = parse_mouse_modifiers_for_clap,
        allow_hyphen_values = true
    )]
    mouse_modifiers: Option<u32>,
    #[arg(
        long = "mouse-pixels",
        value_name = "x:y",
        value_parser = parse_mouse_pixels,
        allow_hyphen_values = true
    )]
    mouse_pixels: Option<(u32, u32)>,
    #[arg(long = "no-input", action = ArgAction::SetTrue)]
    no_input: bool,
    #[arg(
        long = "scrollback-start",
        value_name = "LINE",
        value_parser = parse_scrollback_start_arg
    )]
    scrollback_start: Option<u64>,
    #[arg(
        long = "scrollback-count",
        value_name = "COUNT",
        value_parser = parse_scrollback_count_arg
    )]
    scrollback_count: Option<u32>,
    #[arg(
        long = "scrollback-tail",
        value_name = "COUNT",
        value_parser = parse_scrollback_tail_arg
    )]
    scrollback_tail: Option<u32>,
    #[arg(long = "no-scrollback", action = ArgAction::SetTrue)]
    no_scrollback: bool,
    #[arg(long = "state", value_name = "PATH")]
    state_path: Option<PathBuf>,
    #[arg(long = "record", value_name = "PATH")]
    record_path: Option<PathBuf>,
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
    #[arg(
        long = "startup-timeout-ms",
        value_name = "MS",
        value_parser = parse_startup_timeout_ms_arg
    )]
    startup_timeout_ms: Option<u64>,
    #[arg(long = "stdin", action = ArgAction::SetTrue)]
    stdin_input: bool,
    #[arg(long = "stdin-bytes", action = ArgAction::SetTrue)]
    stdin_bytes: bool,
    #[arg(
        long = "local-echo",
        value_name = "off|tty",
        allow_hyphen_values = true
    )]
    local_echo: Option<LocalEcho>,
    #[arg(
        long = "detach-key",
        value_name = "ctrl-]|none",
        allow_hyphen_values = true
    )]
    detach_key: Option<DetachKey>,
    #[arg(long = "redraw", action = ArgAction::SetTrue)]
    redraw: bool,
    #[arg(long = "speculative-echo", action = ArgAction::SetTrue)]
    speculative_echo: bool,
    #[arg(long = "cols", value_name = "COUNT", value_parser = parse_cols_arg)]
    live_cols: Option<u32>,
    #[arg(long = "rows", value_name = "COUNT", value_parser = parse_rows_arg)]
    live_rows: Option<u32>,
    #[arg(long = "interval-ms", value_name = "MS", value_parser = parse_interval_ms_arg)]
    interval_ms: Option<u64>,
    #[arg(
        long = "connect-timeout-ms",
        value_name = "MS",
        value_parser = parse_connect_timeout_ms_arg
    )]
    connect_timeout_ms: Option<u64>,
    #[arg(long = "iterations", value_name = "COUNT", value_parser = parse_iterations_arg)]
    iterations: Option<usize>,
    #[command(subcommand)]
    command: Option<RawCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptCommand {
    SessionList,
    PaneList,
    PaneSend,
    PaneSnapshot,
    PaneSplit,
    TabList,
    TabNew,
    TabSwitch,
    TabClose,
    SessionNew,
    SessionKill,
    Replay,
}

#[derive(Debug, Subcommand)]
enum RawCommand {
    Pane {
        #[command(subcommand)]
        command: RawPaneCommand,
    },
    Tab {
        #[command(subcommand)]
        command: RawTabCommand,
    },
    Session {
        #[command(subcommand)]
        command: RawSessionCommand,
    },
    Replay {
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    Attach {
        #[arg(value_name = "SESSION", allow_hyphen_values = true)]
        session: Option<String>,
    },
    New {
        #[arg(value_name = "SESSION", allow_hyphen_values = true)]
        session: Option<String>,
    },
    Ls,
    Kill {
        #[arg(value_name = "SESSION", allow_hyphen_values = true)]
        session: Option<String>,
    },
    SendKeys {
        #[arg(short = 't', value_name = "PANE_ID", allow_hyphen_values = true)]
        target: Option<String>,
        #[arg(value_name = "KEYS", allow_hyphen_values = true)]
        keys: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
enum RawPaneCommand {
    Ls {
        #[arg(long = "json", action = ArgAction::SetTrue)]
        json: bool,
    },
    Send {
        #[arg(value_name = "PANE_ID", allow_hyphen_values = true)]
        pane_id: String,
        #[arg(value_name = "TEXT", allow_hyphen_values = true)]
        text: String,
    },
    #[command(alias = "read")]
    Snapshot {
        #[arg(value_name = "PANE_ID", allow_hyphen_values = true)]
        pane_id: String,
        #[arg(long = "json", action = ArgAction::SetTrue)]
        json: bool,
    },
    Split {
        #[arg(value_name = "horizontal|vertical")]
        axis: SplitAxisArg,
        #[arg(value_name = "PANE_ID", allow_hyphen_values = true)]
        pane_id: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum RawTabCommand {
    Ls {
        #[arg(long = "json", action = ArgAction::SetTrue)]
        json: bool,
    },
    New {
        #[arg(value_name = "TAB_ID", allow_hyphen_values = true)]
        tab_id: Option<String>,
        #[arg(long = "title", value_name = "TITLE", allow_hyphen_values = true)]
        title: Option<String>,
    },
    Close {
        #[arg(value_name = "TAB_ID", allow_hyphen_values = true)]
        tab_id: Option<String>,
    },
    Switch {
        #[arg(value_name = "TAB_ID", allow_hyphen_values = true)]
        tab_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum RawSessionCommand {
    New {
        #[arg(value_name = "SESSION", allow_hyphen_values = true)]
        session: String,
        #[arg(long = "title", value_name = "TITLE", allow_hyphen_values = true)]
        title: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum SplitAxisArg {
    Horizontal,
    Vertical,
}

impl From<SplitAxisArg> for protocol::SplitAxis {
    fn from(value: SplitAxisArg) -> Self {
        match value {
            SplitAxisArg::Horizontal => protocol::SplitAxis::Horizontal,
            SplitAxisArg::Vertical => protocol::SplitAxis::Vertical,
        }
    }
}

fn args() -> Result<Args, Box<dyn std::error::Error>> {
    args_from_iter(std::env::args().skip(1))
}

fn args_from_iter<I, S>(args: I) -> Result<Args, Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    let input_args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let auto_default = input_args.is_empty();
    let args = preprocess_args(input_args)?;
    let mut raw =
        RawArgs::try_parse_from(std::iter::once(std::ffi::OsString::from("nmux")).chain(args))
            .map_err(clap_error_message)?;
    let script = normalize_script_command(&mut raw)?;
    let script_command = script.command;
    let replay_path = script.replay_path;
    let socket_path_set = raw.socket_path.is_some();
    let (socket_path, socket_source) = match raw.socket_path {
        Some(path) => (path, local::SocketPathSource::Explicit),
        None => local::default_socket_path_and_source(),
    };
    if raw.target_session_id.as_deref().is_some_and(str::is_empty) {
        return Err("--session requires a non-empty name".into());
    }
    let key_set = raw.key_text.is_some();
    let key_name_set = !raw.key_names.is_empty();
    let key_modifiers_set = raw.key_modifiers.is_some();
    let paste_set = raw.paste_text.is_some();
    let focus_set = raw.focus_event.is_some();
    let mouse_set = raw.mouse_event.is_some();
    let mouse_modifiers_set = raw.mouse_modifiers.is_some();
    let mouse_pixels_set = raw.mouse_pixels.is_some();
    let local_echo_set = raw.local_echo.is_some();
    let detach_key_set = raw.detach_key.is_some();
    let start = raw.start || raw.shell;
    let live = raw.live || raw.shell;
    let stdin_bytes = raw.stdin_bytes || raw.shell;
    let redraw = raw.redraw || raw.shell;
    let scrollback_start_set = raw.scrollback_start.is_some();
    let scrollback_count_set = raw.scrollback_count.is_some();
    let scrollback_tail_set = raw.scrollback_tail.is_some();
    let startup_timeout_set = raw.startup_timeout_ms.is_some();
    let scrollback_start_line = raw.scrollback_start.unwrap_or(1);
    let scrollback_line_count = raw.scrollback_count.unwrap_or(2);
    let scrollback_tail_count = raw.scrollback_tail;
    let live_cols = raw.live_cols;
    let live_rows = raw.live_rows;
    let interval_ms = raw.interval_ms.unwrap_or(if stdin_bytes {
        // Interactive byte input needs fast polling for responsive rendering
        // (~60 fps).
        16
    } else {
        1000
    });
    let connect_timeout_ms = raw.connect_timeout_ms;
    let startup_timeout_ms = raw
        .startup_timeout_ms
        .unwrap_or(DEFAULT_MANAGED_STARTUP_TIMEOUT_MS);
    let iterations = raw.iterations;
    let key_name = raw.key_names.first().cloned();
    let key_names = raw.key_names.clone();
    let key_modifiers = raw.key_modifiers.unwrap_or(0);
    let mut mouse_event = raw.mouse_event;
    let mouse_modifiers = raw.mouse_modifiers.unwrap_or(0);
    let mouse_pixels = raw.mouse_pixels;
    let focus_event = raw.focus_event;
    let local_echo = raw.local_echo.unwrap_or(LocalEcho::Off);
    let detach_key = raw.detach_key.unwrap_or(DetachKey::CtrlRightBracket);
    let start_working_dir = match raw.start_working_dir {
        Some(value) if value.is_empty() => {
            return Err("--cwd requires a non-empty directory path".into());
        }
        value => value,
    };
    if raw.target_pane_id.as_deref().is_some_and(str::is_empty) {
        return Err("--pane requires a non-empty pane ID".into());
    }
    if raw.target_tab_id.as_deref().is_some_and(str::is_empty) {
        return Err("--tab requires a non-empty tab ID".into());
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
        || raw.state_info_json
        || matches!(script_command, Some(ScriptCommand::Replay));
    if raw.tcp_endpoint.as_deref().is_some_and(str::is_empty) {
        return Err("--tcp requires a non-empty HOST:PORT".into());
    }
    let tcp_token = raw.tcp_token.or_else(|| {
        std::env::var("NMUX_TOKEN")
            .ok()
            .filter(|value| !value.is_empty())
    });
    if tcp_token.as_deref().is_some_and(str::is_empty) {
        return Err("--token requires a non-empty token".into());
    }
    if raw.tcp_endpoint.is_some() && tcp_token.is_none() && !exits_before_attach {
        return Err("remote TCP attach requires --token, --tcp-token, or NMUX_TOKEN".into());
    }
    if raw.tcp_endpoint.is_none() && tcp_token.is_some() && !exits_before_attach {
        return Err("--token requires a remote host or --tcp".into());
    }
    if raw.tcp_endpoint.is_some() && socket_path_set && !exits_before_attach {
        return Err("--tcp cannot be combined with --socket".into());
    }
    if raw.tcp_endpoint.is_some() && start && !exits_before_attach {
        return Err("--tcp cannot be combined with --start or --shell".into());
    }
    let actor_id = raw.actor_id.unwrap_or_else(|| "local-actor".to_owned());
    let user_id = raw.user_id.unwrap_or_else(|| "local-user".to_owned());
    let display_name = raw.display_name.unwrap_or_else(|| "local".to_owned());
    if actor_id.is_empty() {
        return Err("--actor-id requires a non-empty ID".into());
    }
    if user_id.is_empty() {
        return Err("--user-id requires a non-empty ID".into());
    }
    if display_name.is_empty() {
        return Err("--display-name requires a non-empty name".into());
    }
    if raw.target_pane_id.is_some() && raw.target_tab_id.is_some() {
        return Err("--pane cannot be combined with --tab".into());
    }
    let mut input_text = raw.key_text;
    if key_name_set || paste_set || focus_set || mouse_set || raw.no_input || raw.follow {
        input_text = None;
    }
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
        validate_positive_numeric_args(PositiveNumericArgs {
            scrollback_start_line,
            scrollback_line_count,
            scrollback_tail_count,
            live_resize,
            interval_ms,
            connect_timeout_ms,
            startup_timeout_ms,
        })?;
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
        validate_no_input_resize_args(NoInputResizeArgs {
            no_input_set: raw.no_input,
            live_resize,
        })?;
        validate_mode_args(ClientModeArgs {
            live,
            follow: raw.follow,
            stdin_input: raw.stdin_input,
            stdin_bytes,
            local_echo_set,
            detach_key_set,
            redraw,
            speculative_echo: raw.speculative_echo,
            record_set: raw.record_path.is_some(),
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
        if start {
            validate_working_dir_arg(start_working_dir.as_deref())?;
        }
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
        target_session_id: raw.target_session_id,
        tcp_endpoint: raw.tcp_endpoint,
        tcp_token,
        target_pane_id: raw.target_pane_id,
        target_tab_id: raw.target_tab_id,
        actor_id,
        user_id,
        display_name,
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
        record_path: raw.record_path,
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
        detach_key,
        redraw,
        speculative_echo: raw.speculative_echo,
        live_resize,
        interval_ms,
        connect_timeout_ms,
        iterations,
        script_command,
        script_split_axis: script.split_axis,
        script_title: script.title,
        replay_path,
        auto_default,
    })
}

fn preprocess_args<I, S>(args: I) -> Result<Vec<std::ffi::OsString>, Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    let raw = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let Some(first) = raw.first() else {
        return Ok(raw);
    };
    let Some(first_text) = first.to_str() else {
        return Ok(raw);
    };
    if first_text.starts_with('-') || known_command(first_text) {
        return Ok(raw);
    }

    let mut normalized = Vec::with_capacity(raw.len() + 1);
    normalized.push(std::ffi::OsString::from("--tcp"));
    normalized.push(std::ffi::OsString::from(normalize_remote_endpoint(
        first_text,
    )?));
    normalized.extend(raw.into_iter().skip(1));
    Ok(normalized)
}

fn known_command(value: &str) -> bool {
    matches!(
        value,
        "pane" | "tab" | "session" | "replay" | "attach" | "new" | "ls" | "kill" | "send-keys"
    )
}

fn normalize_remote_endpoint(value: &str) -> Result<String, Box<dyn std::error::Error>> {
    let value = value.trim();
    if value.is_empty() {
        return Err("remote host requires a non-empty value".into());
    }
    let host = value
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(value);
    if host.is_empty() {
        return Err("remote host requires a host after user@".into());
    }
    if host
        .rsplit_once(':')
        .is_some_and(|(_, port)| !port.is_empty())
    {
        Ok(host.to_owned())
    } else {
        Ok(format!("{host}:{DEFAULT_REMOTE_PORT}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedScriptCommand {
    command: Option<ScriptCommand>,
    split_axis: protocol::SplitAxis,
    title: Option<String>,
    replay_path: Option<PathBuf>,
}

fn normalize_script_command(
    raw: &mut RawArgs,
) -> Result<NormalizedScriptCommand, Box<dyn std::error::Error>> {
    let Some(command) = raw.command.take() else {
        return Ok(NormalizedScriptCommand {
            command: None,
            split_axis: protocol::SplitAxis::None,
            title: None,
            replay_path: None,
        });
    };
    match command {
        RawCommand::Replay { path } => {
            if path.as_os_str().is_empty() {
                return Err("replay requires a non-empty path".into());
            }
            Ok(NormalizedScriptCommand {
                command: Some(ScriptCommand::Replay),
                split_axis: protocol::SplitAxis::None,
                title: None,
                replay_path: Some(path),
            })
        }
        RawCommand::Ls => {
            raw.no_input = true;
            raw.no_scrollback = true;
            Ok(NormalizedScriptCommand {
                command: Some(ScriptCommand::SessionList),
                split_axis: protocol::SplitAxis::None,
                title: None,
                replay_path: None,
            })
        }
        RawCommand::Attach { session } => {
            if session.as_deref().is_some_and(str::is_empty) {
                return Err("attach requires a non-empty session name".into());
            }
            if raw.target_session_id.is_some() && session.is_some() {
                return Err("--session cannot be combined with attach SESSION".into());
            }
            if let Some(session) = session {
                raw.target_session_id = Some(session);
            }
            Ok(NormalizedScriptCommand {
                command: None,
                split_axis: protocol::SplitAxis::None,
                title: None,
                replay_path: None,
            })
        }
        RawCommand::New { session } => {
            if session.as_deref().is_some_and(str::is_empty) {
                return Err("new requires a non-empty session name".into());
            }
            if raw.target_session_id.is_some() && session.is_some() {
                return Err("--session cannot be combined with new SESSION".into());
            }
            if let Some(session) = session {
                raw.target_session_id = Some(session);
            }
            raw.start = true;
            raw.live = true;
            raw.stdin_bytes = true;
            raw.redraw = true;
            Ok(NormalizedScriptCommand {
                command: None,
                split_axis: protocol::SplitAxis::None,
                title: None,
                replay_path: None,
            })
        }
        RawCommand::Kill { session } => {
            if session.as_deref().is_some_and(str::is_empty) {
                return Err("kill requires a non-empty session name".into());
            }
            if raw.target_session_id.is_some() && session.is_some() {
                return Err("--session cannot be combined with kill SESSION".into());
            }
            if let Some(session) = session {
                raw.target_session_id = Some(session);
            }
            raw.no_input = true;
            raw.no_scrollback = true;
            Ok(NormalizedScriptCommand {
                command: Some(ScriptCommand::SessionKill),
                split_axis: protocol::SplitAxis::None,
                title: None,
                replay_path: None,
            })
        }
        RawCommand::SendKeys { target, keys } => {
            if raw.target_pane_id.is_some() || target.is_some() {
                let pane_id = target.or_else(|| raw.target_pane_id.clone());
                if pane_id.as_deref().is_some_and(str::is_empty) {
                    return Err("send-keys target requires a non-empty pane ID".into());
                }
                raw.target_pane_id = pane_id;
            }
            if keys.is_empty() {
                return Err("send-keys requires at least one key".into());
            }
            raw.key_text = Some(keys.join(""));
            raw.no_scrollback = true;
            Ok(NormalizedScriptCommand {
                command: Some(ScriptCommand::PaneSend),
                split_axis: protocol::SplitAxis::None,
                title: None,
                replay_path: None,
            })
        }
        RawCommand::Pane { command } => match command {
            RawPaneCommand::Ls { json } => {
                raw.no_input = true;
                raw.no_scrollback = true;
                raw.output_json = json || raw.output_json;
                Ok(NormalizedScriptCommand {
                    command: Some(ScriptCommand::PaneList),
                    split_axis: protocol::SplitAxis::None,
                    title: None,
                    replay_path: None,
                })
            }
            RawPaneCommand::Send { pane_id, text } => {
                if raw.target_pane_id.is_some() {
                    return Err("--pane cannot be combined with the pane subcommand".into());
                }
                if pane_id.is_empty() {
                    return Err("pane send requires a non-empty pane ID".into());
                }
                raw.target_pane_id = Some(pane_id);
                raw.key_text = Some(text);
                raw.no_scrollback = true;
                Ok(NormalizedScriptCommand {
                    command: Some(ScriptCommand::PaneSend),
                    split_axis: protocol::SplitAxis::None,
                    title: None,
                    replay_path: None,
                })
            }
            RawPaneCommand::Snapshot { pane_id, json } => {
                if raw.target_pane_id.is_some() {
                    return Err("--pane cannot be combined with the pane subcommand".into());
                }
                if pane_id.is_empty() {
                    return Err("pane snapshot requires a non-empty pane ID".into());
                }
                raw.target_pane_id = Some(pane_id);
                raw.no_input = true;
                raw.no_scrollback = true;
                raw.output_json = json || raw.output_json;
                Ok(NormalizedScriptCommand {
                    command: Some(ScriptCommand::PaneSnapshot),
                    split_axis: protocol::SplitAxis::None,
                    title: None,
                    replay_path: None,
                })
            }
            RawPaneCommand::Split { axis, pane_id } => {
                if raw.target_pane_id.is_some() {
                    return Err("--pane cannot be combined with the pane subcommand".into());
                }
                if pane_id.as_deref().is_some_and(str::is_empty) {
                    return Err("pane split requires a non-empty pane ID".into());
                }
                raw.target_pane_id = pane_id;
                raw.no_input = true;
                raw.no_scrollback = true;
                Ok(NormalizedScriptCommand {
                    command: Some(ScriptCommand::PaneSplit),
                    split_axis: axis.into(),
                    title: None,
                    replay_path: None,
                })
            }
        },
        RawCommand::Tab { command } => {
            if raw.target_tab_id.is_some() {
                return Err("--tab cannot be combined with the tab subcommand".into());
            }
            match command {
                RawTabCommand::Ls { json } => {
                    raw.no_input = true;
                    raw.no_scrollback = true;
                    raw.output_json = json || raw.output_json;
                    Ok(NormalizedScriptCommand {
                        command: Some(ScriptCommand::TabList),
                        split_axis: protocol::SplitAxis::None,
                        title: None,
                        replay_path: None,
                    })
                }
                RawTabCommand::New { tab_id, title } => {
                    if tab_id.as_deref().is_some_and(str::is_empty) {
                        return Err("tab new requires a non-empty tab ID".into());
                    }
                    if title.as_deref().is_some_and(str::is_empty) {
                        return Err("tab new --title requires a non-empty title".into());
                    }
                    raw.target_tab_id = tab_id;
                    raw.no_input = true;
                    raw.no_scrollback = true;
                    Ok(NormalizedScriptCommand {
                        command: Some(ScriptCommand::TabNew),
                        split_axis: protocol::SplitAxis::None,
                        title,
                        replay_path: None,
                    })
                }
                RawTabCommand::Close { tab_id } => {
                    if tab_id.as_deref().is_some_and(str::is_empty) {
                        return Err("tab close requires a non-empty tab ID".into());
                    }
                    raw.target_tab_id = tab_id;
                    raw.no_input = true;
                    raw.no_scrollback = true;
                    Ok(NormalizedScriptCommand {
                        command: Some(ScriptCommand::TabClose),
                        split_axis: protocol::SplitAxis::None,
                        title: None,
                        replay_path: None,
                    })
                }
                RawTabCommand::Switch { tab_id } => {
                    if tab_id.is_empty() {
                        return Err("tab switch requires a non-empty tab ID".into());
                    }
                    raw.target_tab_id = Some(tab_id);
                    raw.no_input = true;
                    raw.no_scrollback = true;
                    Ok(NormalizedScriptCommand {
                        command: Some(ScriptCommand::TabSwitch),
                        split_axis: protocol::SplitAxis::None,
                        title: None,
                        replay_path: None,
                    })
                }
            }
        }
        RawCommand::Session { command } => match command {
            RawSessionCommand::New { session, title } => {
                if raw.target_session_id.is_some() {
                    return Err("--session cannot be combined with the session subcommand".into());
                }
                if session.is_empty() {
                    return Err("session new requires a non-empty session name".into());
                }
                if title.as_deref().is_some_and(str::is_empty) {
                    return Err("session new --title requires a non-empty title".into());
                }
                raw.target_session_id = Some(session);
                raw.no_input = true;
                raw.no_scrollback = true;
                Ok(NormalizedScriptCommand {
                    command: Some(ScriptCommand::SessionNew),
                    split_axis: protocol::SplitAxis::None,
                    title,
                    replay_path: None,
                })
            }
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputFormat {
    Text,
    Json,
}

fn output_format(json: bool) -> OutputFormat {
    if json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    }
}

fn print_context(format: OutputFormat) -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("NMUX").ok().as_deref() != Some("1") {
        return Err("not running inside an nmux pane (NMUX=1 is not set)".into());
    }

    let session_id = required_context_env("NMUX_SESSION_ID")?;
    let pane_id = required_context_env("NMUX_PANE_ID")?;
    let socket = required_context_env("NMUX_SOCKET")?;
    let origin = required_context_env("NMUX_ORIGIN")?;

    if format == OutputFormat::Json {
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

fn print_key_names(format: OutputFormat) {
    match format {
        OutputFormat::Json => println!("{}", format_key_names_json()),
        OutputFormat::Text => {
            for key_name in SUPPORTED_KEY_NAMES {
                println!("{key_name}");
            }
            for (alias, canonical) in KEY_NAME_ALIASES {
                println!("{alias} -> {canonical}");
            }
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
        "{{\"key_names\":{},\"key_modifiers\":{},\"focus_events\":{},\"mouse_actions\":{},\"mouse_buttons\":{},\"local_echo\":{},\"detach_keys\":{}}}",
        format_key_names_json(),
        format_json_string_array(KEY_MODIFIER_NAMES),
        format_json_string_array(FOCUS_EVENT_NAMES),
        format_json_string_array(MOUSE_ACTION_NAMES),
        format_json_string_array(MOUSE_BUTTON_NAMES),
        format_json_string_array(LOCAL_ECHO_NAMES),
        format_json_string_array(DETACH_KEY_NAMES)
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

fn format_live_presence_json(presence: &local::PresenceSummary) -> String {
    let focused_pane_id = presence
        .focused_pane_id
        .as_deref()
        .map(local::json_string)
        .unwrap_or_else(|| "null".to_owned());
    format!(
        "{{\"event\":\"presence\",\"presence\":{{\"actor_id\":{},\"user_id\":{},\"display_name\":{},\"mode\":{},\"focused_pane_id\":{focused_pane_id}}}}}",
        local::json_string(&presence.actor_id),
        local::json_string(&presence.user_id),
        local::json_string(&presence.display_name),
        local::json_string(attach_mode_name(presence.mode))
    )
}

fn attach_mode_name(mode: AttachMode) -> &'static str {
    match mode {
        AttachMode::ReadOnly => "read-only",
        AttachMode::ReadWrite => "read-write",
    }
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

fn format_session_inventory_json(inventory: &local::SessionInventorySummary) -> String {
    let sessions = inventory
        .sessions
        .iter()
        .map(|session| {
            format!(
                "{{\"session_id\":{},\"title\":{},\"active\":{}}}",
                local::json_string(&session.session_id),
                local::json_string(&session.title),
                session.session_id == inventory.active_session_id
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"active_session_id\":{},\"sessions\":[{sessions}]}}",
        local::json_string(&inventory.active_session_id)
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

fn parse_numeric_arg<T>(flag: &str, value: &str) -> Result<T, String>
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
    let first_line = first_line
        .lines()
        .next()
        .unwrap_or("invalid command line")
        .trim_start_matches("error: ")
        .to_owned();
    if let Some((_, reason)) = first_line.rsplit_once(": ")
        && reason.starts_with("--")
    {
        return reason.to_owned();
    }
    first_line
}

fn parse_scrollback_start_arg(value: &str) -> Result<u64, String> {
    parse_numeric_arg("--scrollback-start", value)
}

fn parse_scrollback_count_arg(value: &str) -> Result<u32, String> {
    parse_numeric_arg("--scrollback-count", value)
}

fn parse_scrollback_tail_arg(value: &str) -> Result<u32, String> {
    parse_numeric_arg("--scrollback-tail", value)
}

fn parse_cols_arg(value: &str) -> Result<u32, String> {
    parse_numeric_arg("--cols", value)
}

fn parse_rows_arg(value: &str) -> Result<u32, String> {
    parse_numeric_arg("--rows", value)
}

fn parse_interval_ms_arg(value: &str) -> Result<u64, String> {
    parse_numeric_arg("--interval-ms", value)
}

fn parse_connect_timeout_ms_arg(value: &str) -> Result<u64, String> {
    parse_numeric_arg("--connect-timeout-ms", value)
}

fn parse_startup_timeout_ms_arg(value: &str) -> Result<u64, String> {
    parse_numeric_arg("--startup-timeout-ms", value)
}

fn parse_iterations_arg(value: &str) -> Result<usize, String> {
    parse_numeric_arg("--iterations", value)
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

fn validate_working_dir_arg(path: Option<&str>) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };
    let metadata =
        fs::metadata(path).map_err(|err| format!("--cwd must be an existing directory: {err}"))?;
    if !metadata.is_dir() {
        return Err("--cwd must be an existing directory".to_owned());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct NoInputResizeArgs {
    no_input_set: bool,
    live_resize: Option<(u32, u32)>,
}

fn validate_no_input_resize_args(args: NoInputResizeArgs) -> Result<(), &'static str> {
    if args.no_input_set && args.live_resize.is_some() {
        return Err("--no-input cannot be combined with --cols/--rows");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PositiveNumericArgs {
    scrollback_start_line: u64,
    scrollback_line_count: u32,
    scrollback_tail_count: Option<u32>,
    live_resize: Option<(u32, u32)>,
    interval_ms: u64,
    connect_timeout_ms: Option<u64>,
    startup_timeout_ms: u64,
}

fn validate_positive_numeric_args(args: PositiveNumericArgs) -> Result<(), &'static str> {
    if args.scrollback_start_line == 0 {
        return Err("--scrollback-start must be greater than 0");
    }
    if args.scrollback_line_count == 0 {
        return Err("--scrollback-count must be greater than 0");
    }
    if args.scrollback_tail_count == Some(0) {
        return Err("--scrollback-tail must be greater than 0");
    }
    if args.interval_ms == 0 {
        return Err("--interval-ms must be greater than 0");
    }
    if args.connect_timeout_ms == Some(0) {
        return Err("--connect-timeout-ms must be greater than 0");
    }
    if args.startup_timeout_ms == 0 {
        return Err("--startup-timeout-ms must be greater than 0");
    }
    if args.live_resize.is_some_and(|(cols, rows)| {
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
    detach_key_set: bool,
    redraw: bool,
    speculative_echo: bool,
    record_set: bool,
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
    if args.detach_key_set && !args.stdin_bytes {
        return Err("--detach-key requires --stdin-bytes");
    }
    if args.redraw && !args.live {
        return Err("--redraw requires --live");
    }
    if args.speculative_echo && !args.live {
        return Err("--speculative-echo requires --live");
    }
    if args.record_set && !args.live {
        return Err("--record requires --live");
    }
    if args.speculative_echo && !args.redraw {
        return Err("--speculative-echo requires --redraw");
    }
    if args.speculative_echo && !args.key_set {
        return Err("--speculative-echo requires --key");
    }
    if args.live_resize.is_some() && !args.live {
        return Err("--cols and --rows require --live");
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
  nmux [OPTIONS] [user@]HOST[:PORT]
  nmux daemon [DAEMON_OPTIONS]
  nmux attach [SESSION]
  nmux new [SESSION]
  nmux ls
  nmux kill [SESSION]
  nmux [OPTIONS] pane ls [--json]
  nmux [OPTIONS] pane send PANE_ID TEXT
  nmux [OPTIONS] pane read PANE_ID [--json]
  nmux [OPTIONS] pane snapshot PANE_ID --json
  nmux [OPTIONS] pane split horizontal|vertical [PANE_ID]
  nmux [OPTIONS] tab ls [--json]
  nmux [OPTIONS] tab new [TAB_ID] [--title TITLE]
  nmux [OPTIONS] tab switch TAB_ID
  nmux [OPTIONS] tab close [TAB_ID]
  nmux send-keys [-t PANE_ID] KEYS...
  nmux replay PATH
  nmux version [--json]

Options:
  --socket PATH              Unix socket path
  -s, --session NAME         Target a named session on the selected daemon
  --tcp HOST:PORT            Connect over TCP instead of a Unix socket
  --tcp-token, --token TOKEN Shared token for TCP transport authentication
  --pane PANE_ID             Attach to and send input to PANE_ID
  --tab TAB_ID               Attach to and make TAB_ID active
  --actor-id ID              Client actor ID for presence
  --user-id ID               Client user ID for presence
  --display-name NAME        Client display name for presence
  --print-context            Print inherited nmux pane context and exit
  --print-context-json       Print inherited nmux pane context as JSON and exit
  --print-socket             Print the resolved socket path and exit
  --print-socket-json        Print the resolved socket path as JSON and exit
  --connect-timeout-ms MS    Wait up to this long for the daemon transport
  --startup-timeout-ms MS    Wait up to this long for managed daemon readiness
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
  --record PATH              Write timestamped live JSON events to PATH
  --follow                   Reconnect in a polling loop
  --live                     Keep one attach connection open
  --start                    Start a private local daemon before attaching
  --shell                    Start a private live shell with stdin-bytes redraw
  --command SHELL            Managed daemon pane command for --start
  --cwd DIR                  Existing pane working directory for --start
  --env KEY=VALUE            Managed daemon pane environment for --start
  --stdin                    Stream newline-delimited stdin in live mode
  --stdin-bytes              Stream raw stdin chunks in live mode
  --local-echo off|tty       Local TTY echo policy for --stdin-bytes
  --detach-key ctrl-]|none   Local detach key for --stdin-bytes
  --redraw                   Repaint the current live surface in place
  --speculative-echo         Experimental redraw-only local echo prediction
  --cols COUNT               Live ResizeIntent columns; both dimensions required
  --rows COUNT               Live ResizeIntent rows; both dimensions required
  --interval-ms MS           Poll/read timeout in milliseconds
  --iterations COUNT         Bounded follow/live cycle count
  --version-json             Show version as JSON
  -V, --version              Show version
  -h, --help                 Show this help

Subcommands:
  daemon [OPTIONS]                Run the daemon in the foreground
  attach [SESSION]                Attach to the default or named local session
  new [SESSION]                   Start a private live shell session
  ls                              List the current local session
  kill [SESSION]                  Stop the default or named local session
  pane ls [--json]                List panes in the active tab
  pane send PANE_ID TEXT          Send text input to a pane
  pane read PANE_ID [--json]      Print a pane surface, optionally as JSON
  pane snapshot PANE_ID --json    Print a pane snapshot as JSON
  pane split AXIS [PANE_ID]       Split a pane horizontally or vertically
  tab ls [--json]                 List tabs visible to the current protocol
  tab new [TAB_ID]                Create and switch to a new tab
  tab switch TAB_ID               Switch to an existing tab
  tab close [TAB_ID]              Close a tab, defaulting to the active tab
  send-keys [-t PANE_ID] KEYS...  Send text keys to a pane
  replay PATH                     Print surface frames from a recorded live session
  version [--json]                Print client version information

Notes:
  Default socket: --socket, else valid absolute $NMUX_SOCKET, else valid absolute $XDG_RUNTIME_DIR/nmux/nmux.sock, else /tmp/nmux-$UID/nmux.sock.
  [user@]HOST[:PORT] is direct TCP today; host without a port uses 7007.
  Remote TCP requires --token, --tcp-token, or NMUX_TOKEN.
  --tcp cannot be combined with --socket, --start, or --shell.
  Bare interactive nmux attaches to the shared local session, starting it if missing.
  --print-context prints inherited NMUX_* pane identity without connecting.
  --print-context-json prints the same inherited context as a JSON object.
  --print-socket-json prints the resolved socket path and source as JSON.
  --state-info and --state-info-json require --state PATH and do not connect.
  --json emits one object per one-shot/follow attach, or newline-delimited live events.
  --record writes newline-delimited live events with elapsed_ms timestamps for replay/export tooling.
  --start waits for nmux daemon --ready-json and cleans up the private daemon on exit.
  --startup-timeout-ms controls that managed readiness wait and defaults to 5000.
  --shell is shorthand for --start --live --stdin-bytes --redraw using $SHELL or sh.
  --speculative-echo is experimental and predicts only simple printable --key input in redraw mode.
  Ctrl-] detaches byte-streamed live sessions by default; --detach-key none passes it through.
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
  nmux --live --redraw --key x --speculative-echo
  nmux daemon --live-forever
  nmux devbox:7007 --token TOKEN
  nmux --shell
  nmux --start --cwd /tmp --env NMUX_DEMO=1 --command 'pwd; env | grep ^NMUX_DEMO=; cat >/dev/null'
  nmux --start --live --stdin-bytes --redraw --command '$SHELL'
"
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum LocalEcho {
    Off,
    Tty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum DetachKey {
    #[value(name = "ctrl-]")]
    CtrlRightBracket,
    None,
}

impl DetachKey {
    fn byte(self) -> Option<u8> {
        match self {
            Self::CtrlRightBracket => Some(STDIN_BYTES_DETACH),
            Self::None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
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

#[cfg(test)]
fn parse_local_echo(value: &str) -> Result<LocalEcho, &'static str> {
    match value {
        "off" => Ok(LocalEcho::Off),
        "tty" => Ok(LocalEcho::Tty),
        _ => Err("requires off or tty"),
    }
}

#[cfg(test)]
fn parse_detach_key(value: &str) -> Result<DetachKey, &'static str> {
    match value {
        "ctrl-]" => Ok(DetachKey::CtrlRightBracket),
        "none" => Ok(DetachKey::None),
        _ => Err("requires ctrl-] or none"),
    }
}

#[cfg(test)]
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

fn parse_key_modifiers_for_clap(value: &str) -> Result<u32, String> {
    parse_key_modifiers(value).map_err(|err| format!("--key-modifiers {err}"))
}

fn parse_mouse_modifiers_for_clap(value: &str) -> Result<u32, String> {
    parse_key_modifiers(value).map_err(|err| format!("--mouse-modifiers {err}"))
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
        AttachMode, BRACKETED_PASTE_END, BRACKETED_PASTE_START, ClientModeArgs,
        DEFAULT_REMOTE_PORT, DetachKey, ExplicitInputModeArgs, FocusEvent, FrameStats,
        HostMouseModeContext, InterimSurfaceFidelityWarningContext, KEY_NAME_ALIASES,
        LiveDetachReason, LiveMouseDispatch, LiveScrollDirection, LiveSurfaceState,
        LiveUpdatePrintKind, LocalEcho, MouseEvent, NoInputResizeArgs, PositiveNumericArgs,
        RawTerminalModeContext, RedrawState, RedrawTerminalContext, STDIN_BYTES_DETACH,
        SUPPORTED_KEY_NAMES, ScriptCommand, ScrollbackSelectionArgFlags, SgrMouseInput,
        SigwinchResizeContext, StateInfoSocketSummary, StdinByteForward, StdinKey, StdinKeyInput,
        args_from_iter, configure_default_live_args, default_attach_error_needs_restart,
        format_cli_error_json, format_context_json, format_input_choices_json,
        format_key_names_json, format_live_attach_json, format_live_cli_error_json,
        format_live_detach_json, format_live_error_json, format_live_presence_json,
        format_live_surface_update_json, format_live_workspace_json, format_rendered_attach_json,
        format_scrollback, format_state_info_json, format_state_info_text, format_stats_right,
        frontend_resize_pane_size, host_mouse_mode_disable_sequence,
        host_mouse_mode_enable_sequence, host_mouse_mode_mirror_needed,
        interim_surface_fidelity_warning_needed, live_mouse_dispatch_for_workspace_size,
        live_session_new_should_fallback, live_update_print_kind, managed_ready_error_message,
        menu_overlay_for_action, menu_overlay_for_action_with_session_inventory, parse_detach_key,
        parse_env_assignment, parse_focus_event, parse_key_modifiers, parse_key_name,
        parse_local_echo, parse_mouse_event, parse_mouse_pixels, parse_numeric_arg,
        preprocess_args, raw_terminal_fixup_termios, raw_terminal_mode_needed,
        redraw_terminal_guard_needed, redraw_text_with_context, redraw_workspace_surface_text,
        render_scrollback_view_summary, sigwinch_resize_needed, split_stdin_bytes_for_detach,
        stdin_byte_forwards, terminal_size_from_fds, terminal_size_unavailable, tui, usage,
        validate_explicit_input_modes as super_validate_explicit_input_modes,
        validate_mode_args as super_validate_mode_args, validate_no_input_resize_args,
        validate_positive_numeric_args, validate_scrollback_selection_args,
    };
    use nmux_cli::local;
    use nmux_proto::protocol;
    use std::collections::BTreeMap;
    use std::io;
    use std::path::Path;

    fn zero_termios() -> libc::termios {
        // SAFETY: tests assign the termios fields read by raw_terminal_fixup_termios
        // before asserting against the returned value.
        unsafe { std::mem::zeroed() }
    }

    fn strip_csi_for_test(value: &str) -> String {
        let mut stripped = String::new();
        let mut chars = value.chars();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                if chars.next() == Some('[') {
                    for next in chars.by_ref() {
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
            } else {
                stripped.push(ch);
            }
        }
        stripped
    }

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

    fn positive_numeric_defaults() -> PositiveNumericArgs {
        PositiveNumericArgs {
            scrollback_start_line: 1,
            scrollback_line_count: 2,
            scrollback_tail_count: None,
            live_resize: None,
            interval_ms: 1000,
            connect_timeout_ms: None,
            startup_timeout_ms: 1,
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
        assert!(args.auto_default);
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
    fn default_live_args_keep_fast_poll_but_use_startup_setup_timeout() {
        let mut args = args_from_iter(std::iter::empty::<&str>()).expect("args");
        args.startup_timeout_ms = 1234;

        configure_default_live_args(&mut args);

        assert!(args.live);
        assert!(args.stdin_bytes);
        assert!(args.redraw);
        assert!(args.no_scrollback);
        assert_eq!(args.interval_ms, 16);
        assert_eq!(args.connect_timeout_ms, Some(1234));
    }

    #[test]
    fn default_attach_error_restarts_on_exited_pane_process() {
        let error = super::local::ServerError {
            error: super::local::ErrorSummary {
                code: super::protocol::ErrorCode::Unknown,
                message: "output polling failed: pane process is not running: pane-1".to_owned(),
                retryable: false,
                pane_id: Some("pane-1".to_owned()),
                input_seq: 0,
            },
        };

        assert!(default_attach_error_needs_restart(&error));
    }

    #[test]
    fn tab_arg_selects_target_tab_and_conflicts_with_pane_arg() {
        let args = args_from_iter(["--tab", "tab-2"]).expect("args");
        assert_eq!(args.target_tab_id.as_deref(), Some("tab-2"));
        assert_eq!(args.target_pane_id, None);

        let err = match args_from_iter(["--tab", ""]) {
            Ok(_) => panic!("empty tab should fail"),
            Err(err) => err.to_string(),
        };
        assert_eq!(err, "--tab requires a non-empty tab ID");

        let err = match args_from_iter(["--pane", "pane-1", "--tab", "tab-2"]) {
            Ok(_) => panic!("pane and tab should conflict"),
            Err(err) => err.to_string(),
        };
        assert_eq!(err, "--pane cannot be combined with --tab");
    }

    #[test]
    fn presence_identity_args_override_defaults() {
        let args = args_from_iter([
            "--actor-id",
            "actor-a",
            "--user-id",
            "user-a",
            "--display-name",
            "Alice",
        ])
        .expect("args");
        assert_eq!(args.actor_id, "actor-a");
        assert_eq!(args.user_id, "user-a");
        assert_eq!(args.display_name, "Alice");

        let err = match args_from_iter(["--actor-id", ""]) {
            Ok(_) => panic!("empty actor ID should fail"),
            Err(err) => err.to_string(),
        };
        assert_eq!(err, "--actor-id requires a non-empty ID");
    }

    #[test]
    fn pane_subcommands_normalize_to_attach_options() {
        let send = args_from_iter(["pane", "send", "pane-2", "hello\n"]).expect("send args");
        assert_eq!(send.script_command, Some(ScriptCommand::PaneSend));
        assert_eq!(send.target_pane_id.as_deref(), Some("pane-2"));
        assert_eq!(send.input_text.as_deref(), Some("hello\n"));
        assert!(send.no_scrollback);

        let snapshot =
            args_from_iter(["pane", "snapshot", "pane-2", "--json"]).expect("snapshot args");
        assert_eq!(snapshot.script_command, Some(ScriptCommand::PaneSnapshot));
        assert_eq!(snapshot.target_pane_id.as_deref(), Some("pane-2"));
        assert!(snapshot.no_input);
        assert!(snapshot.no_scrollback);
        assert!(snapshot.output_json);

        let read = args_from_iter(["pane", "read", "pane-2", "--json"]).expect("read args");
        assert_eq!(read.script_command, Some(ScriptCommand::PaneSnapshot));
        assert_eq!(read.target_pane_id.as_deref(), Some("pane-2"));
        assert!(read.output_json);

        let split = args_from_iter(["pane", "split", "vertical", "pane-2"]).expect("split args");
        assert_eq!(split.script_command, Some(ScriptCommand::PaneSplit));
        assert_eq!(split.target_pane_id.as_deref(), Some("pane-2"));
        assert_eq!(split.script_split_axis, protocol::SplitAxis::Vertical);
        assert!(split.no_input);
        assert!(split.no_scrollback);

        let tab_new =
            args_from_iter(["tab", "new", "tab-work", "--title", "Work"]).expect("tab new args");
        assert_eq!(tab_new.script_command, Some(ScriptCommand::TabNew));
        assert_eq!(tab_new.target_tab_id.as_deref(), Some("tab-work"));
        assert_eq!(tab_new.script_title.as_deref(), Some("Work"));

        let tab_close = args_from_iter(["tab", "close", "tab-work"]).expect("tab close args");
        assert_eq!(tab_close.script_command, Some(ScriptCommand::TabClose));
        assert_eq!(tab_close.target_tab_id.as_deref(), Some("tab-work"));

        let tab_switch = args_from_iter(["tab", "switch", "tab-work"]).expect("tab switch args");
        assert_eq!(tab_switch.script_command, Some(ScriptCommand::TabSwitch));
        assert_eq!(tab_switch.target_tab_id.as_deref(), Some("tab-work"));
        assert!(tab_switch.no_input);
        assert!(tab_switch.no_scrollback);

        let pane_ls = args_from_iter(["pane", "ls", "--json"]).expect("pane ls args");
        assert_eq!(pane_ls.script_command, Some(ScriptCommand::PaneList));
        assert!(pane_ls.no_input);
        assert!(pane_ls.output_json);

        let tab_ls = args_from_iter(["tab", "ls", "--json"]).expect("tab ls args");
        assert_eq!(tab_ls.script_command, Some(ScriptCommand::TabList));
        assert!(tab_ls.no_input);
        assert!(tab_ls.output_json);

        let session_ls = args_from_iter(["ls"]).expect("session ls args");
        assert_eq!(session_ls.script_command, Some(ScriptCommand::SessionList));
        assert!(session_ls.no_input);

        let session_new = args_from_iter(["session", "new", "work", "--title", "Work"])
            .expect("session new args");
        assert_eq!(session_new.script_command, Some(ScriptCommand::SessionNew));
        assert_eq!(session_new.target_session_id.as_deref(), Some("work"));
        assert_eq!(session_new.script_title.as_deref(), Some("Work"));
        assert!(session_new.no_input);
        assert!(session_new.no_scrollback);

        let attach_named = args_from_iter(["attach", "work"]).expect("attach args");
        assert_eq!(attach_named.target_session_id.as_deref(), Some("work"));

        let new_named = args_from_iter(["new", "work"]).expect("new args");
        assert_eq!(new_named.target_session_id.as_deref(), Some("work"));
        assert!(new_named.start);
        assert!(new_named.live);

        let kill_default = args_from_iter(["kill"]).expect("kill args");
        assert_eq!(
            kill_default.script_command,
            Some(ScriptCommand::SessionKill)
        );
        assert_eq!(kill_default.target_session_id, None);
        assert!(kill_default.no_input);
        assert!(kill_default.no_scrollback);

        let kill_named = args_from_iter(["kill", "work"]).expect("kill named args");
        assert_eq!(kill_named.script_command, Some(ScriptCommand::SessionKill));
        assert_eq!(kill_named.target_session_id.as_deref(), Some("work"));
        assert!(kill_named.no_input);
        assert!(kill_named.no_scrollback);

        let kill_global = args_from_iter(["--session", "work", "kill"]).expect("kill -s args");
        assert_eq!(kill_global.script_command, Some(ScriptCommand::SessionKill));
        assert_eq!(kill_global.target_session_id.as_deref(), Some("work"));

        let err = match args_from_iter(["--session", "work", "kill", "other"]) {
            Ok(_) => panic!("duplicated session target should fail"),
            Err(err) => err.to_string(),
        };
        assert_eq!(err, "--session cannot be combined with kill SESSION");

        let err = match args_from_iter(["--session", "work", "session", "new", "other"]) {
            Ok(_) => panic!("duplicated session target should fail"),
            Err(err) => err.to_string(),
        };
        assert_eq!(
            err,
            "--session cannot be combined with the session subcommand"
        );
    }

    #[test]
    fn remote_positional_normalizes_to_tcp_endpoint() {
        let direct = args_from_iter(["devbox:9000", "--token", "secret"]).expect("direct remote");
        assert_eq!(direct.tcp_endpoint.as_deref(), Some("devbox:9000"));
        assert_eq!(direct.tcp_token.as_deref(), Some("secret"));

        let default_port =
            args_from_iter(["user@devbox", "--token", "secret"]).expect("default remote port");
        let expected_endpoint = format!("devbox:{DEFAULT_REMOTE_PORT}");
        assert_eq!(
            default_port.tcp_endpoint.as_deref(),
            Some(expected_endpoint.as_str())
        );

        let err = match args_from_iter(["devbox:9000"]) {
            Ok(_) => panic!("remote without token should fail"),
            Err(err) => err.to_string(),
        };
        assert_eq!(
            err,
            "remote TCP attach requires --token, --tcp-token, or NMUX_TOKEN"
        );

        let processed = preprocess_args(["pane", "split", "vertical"]).expect("preprocess");
        assert_eq!(
            processed,
            vec![
                std::ffi::OsString::from("pane"),
                std::ffi::OsString::from("split"),
                std::ffi::OsString::from("vertical")
            ]
        );
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
        assert_eq!(args.interval_ms, 16);
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
    fn managed_ready_error_message_extracts_daemon_error_json() {
        assert_eq!(
            managed_ready_error_message(
                "{\"event\":\"error\",\"error\":{\"message\":\"socket path already exists: /tmp/nmux.sock\"}}\n"
            )
            .as_deref(),
            Some("socket path already exists: /tmp/nmux.sock")
        );
        assert_eq!(
            managed_ready_error_message(
                "{\"event\":\"error\",\"error\":{\"message\":\"escaped \\\"quote\\\" and \\\\slash\"}}\n"
            )
            .as_deref(),
            Some("escaped \"quote\" and \\slash")
        );
        assert_eq!(
            managed_ready_error_message(
                "{\"event\":\"ready\",\"NMUX_SOCKET\":\"/tmp/nmux.sock\"}\n"
            ),
            None
        );
    }

    #[test]
    fn terminal_size_treats_ci_tty_unavailable_errors_as_absent_size() {
        for code in [libc::EAGAIN, libc::ENODEV, libc::ENOENT] {
            let size = terminal_size_from_fds(&[], || Err(io::Error::from_raw_os_error(code)))
                .expect("transient or absent tty should not fail managed startup");
            assert_eq!(size, None);

            let err = io::Error::from_raw_os_error(code);
            assert!(terminal_size_unavailable(&err));
        }
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
        assert!(json.contains("\"detach_keys\":[\"ctrl-]\",\"none\"]"));
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
                pane_tree: None,
                tabs: Vec::new(),
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
            pane_tree: None,
            tabs: Vec::new(),
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
        assert_eq!(
            format_live_presence_json(&local::PresenceSummary {
                actor_id: "writer".to_owned(),
                user_id: "user-1".to_owned(),
                display_name: "Writer".to_owned(),
                mode: AttachMode::ReadWrite,
                kind: protocol::PresenceKind::Joined,
                focused_pane_id: Some("pane-1".to_owned()),
            }),
            "{\"event\":\"presence\",\"presence\":{\"actor_id\":\"writer\",\"user_id\":\"user-1\",\"display_name\":\"Writer\",\"mode\":\"read-write\",\"focused_pane_id\":\"pane-1\"}}"
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
        // With differential rendering, cursor-only patches with unchanged
        // metadata in redraw mode are no-ops (no rows changed).
        assert_eq!(
            live_update_print_kind(&previous, &previous, &cursor_only, true),
            LiveUpdatePrintKind::None
        );
        // But cursor-only + metadata change in redraw mode does trigger Metadata.
        assert_eq!(
            live_update_print_kind(&previous, &changed, &cursor_only, true),
            LiveUpdatePrintKind::Metadata
        );
        // ReplaceRows in redraw mode still triggers Surface.
        assert_eq!(
            live_update_print_kind(&previous, &previous, &replace_rows, true),
            LiveUpdatePrintKind::Surface
        );
    }

    #[test]
    fn raw_terminal_mode_is_only_needed_for_stdin_bytes_on_tty() {
        let interactive_byte_mode = RawTerminalModeContext {
            stdin_bytes: true,
            stdin_is_tty: true,
        };

        assert!(raw_terminal_mode_needed(interactive_byte_mode));
        assert!(!raw_terminal_mode_needed(RawTerminalModeContext {
            stdin_is_tty: false,
            ..interactive_byte_mode
        }));
        assert!(!raw_terminal_mode_needed(RawTerminalModeContext {
            stdin_bytes: false,
            ..interactive_byte_mode
        }));
    }

    #[test]
    fn redraw_terminal_guard_is_only_needed_for_redraw_on_tty() {
        let redraw_to_tty = RedrawTerminalContext {
            redraw: true,
            stdout_is_tty: true,
        };

        assert!(redraw_terminal_guard_needed(redraw_to_tty));
        assert!(!redraw_terminal_guard_needed(RedrawTerminalContext {
            stdout_is_tty: false,
            ..redraw_to_tty
        }));
        assert!(!redraw_terminal_guard_needed(RedrawTerminalContext {
            redraw: false,
            ..redraw_to_tty
        }));
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
            terminal_is_tty: true,
        };

        assert!(sigwinch_resize_needed(interactive_byte_mode));
        assert!(!sigwinch_resize_needed(SigwinchResizeContext {
            explicit_resize: true,
            ..interactive_byte_mode
        }));
        assert!(!sigwinch_resize_needed(SigwinchResizeContext {
            terminal_is_tty: false,
            ..interactive_byte_mode
        }));
        assert!(!sigwinch_resize_needed(SigwinchResizeContext {
            stdin_bytes: false,
            ..interactive_byte_mode
        }));
    }

    #[test]
    fn redraw_state_renders_status_bar_and_diffs_content() {
        let mut state = RedrawState::new();
        state.terminal_cols = 120;

        let ws = local::WorkspaceSummary {
            session_id: "local".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            cols: 80,
            rows: 24,
            resize_policy: protocol::ResizePolicy::Fixed,
            pane_tree: None,
            tabs: Vec::new(),
        };

        let initial = state.render_initial_text(&ws, "pane output\nsecond line");
        // Status bar is in inverse video on the final terminal row.
        assert!(
            initial.contains("\x1b[7m") && initial.contains("nmux"),
            "initial frame should contain status bar: {initial:?}"
        );
        assert!(
            initial.contains("pane-1") && initial.contains("80x24"),
            "status bar should show pane id and size: {initial:?}"
        );
        // Content starts on the first terminal row so ratatui chrome can own row 1.
        assert!(
            initial.contains("pane output"),
            "initial frame should contain content: {initial:?}"
        );
        assert!(
            initial.contains("\x1b[1;1Hpane output"),
            "initial frame should position content at the top of the terminal: {initial:?}"
        );

        let update = state.render_diff_text(&ws, "pane output\nsecond line changed");
        // The status bar is always redrawn on the final row.
        assert!(
            update.contains("nmux"),
            "diff should redraw status bar: {update:?}"
        );
        // Content row 2 changed on terminal row 2.
        assert!(
            update.contains("\x1b[2;1H\x1b[2Ksecond line changed"),
            "diff should update changed content row at terminal row 2: {update:?}"
        );
        // Status bar should show stats.
        assert!(
            update.contains("rows") && update.contains("fps"),
            "status bar should contain frame stats: {update:?}"
        );

        state.record_rtt(std::time::Duration::from_millis(3));
        let rtt_update = state.render_diff_text(&ws, "pane output\nsecond line changed");
        assert!(
            rtt_update.contains("rtt:3ms"),
            "status bar should contain RTT after a pong: {rtt_update:?}"
        );

        state.record_client_count(5);
        let clients_update = state.render_diff_text(&ws, "pane output\nsecond line changed");
        assert!(
            clients_update.contains("clients:5"),
            "status bar should contain live client count: {clients_update:?}"
        );
    }

    #[test]
    fn redraw_status_bar_avoids_bottom_row_autowrap() {
        let mut state = RedrawState::new();
        state.terminal_cols = 32;
        state.last_stats = FrameStats {
            decode_time: std::time::Duration::from_micros(123),
            render_time: std::time::Duration::from_micros(456),
            rows_changed: 12,
            rows_total: 34,
            rtt: Some(std::time::Duration::from_millis(9)),
            client_count: Some(2),
            rendered_fps: Some(8),
            idle: false,
        };

        let ws = local::WorkspaceSummary {
            session_id: "local".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-with-a-long-name".to_owned(),
            cols: 155,
            rows: 50,
            resize_policy: protocol::ResizePolicy::Fixed,
            pane_tree: None,
            tabs: Vec::new(),
        };

        let status = state.format_status_bar(&ws);
        let visible = strip_csi_for_test(&status);
        assert!(
            visible.chars().count() <= 31,
            "bottom-row status must not reach the final terminal column: {status:?}"
        );
    }

    #[test]
    fn status_stats_use_fixed_fields_and_idle_fps() {
        let active = FrameStats {
            decode_time: std::time::Duration::from_micros(123),
            render_time: std::time::Duration::from_micros(456),
            rows_changed: 1,
            rows_total: 2,
            rtt: Some(std::time::Duration::from_millis(3)),
            client_count: Some(5),
            rendered_fps: Some(12),
            idle: false,
        };
        let idle = FrameStats {
            decode_time: std::time::Duration::from_micros(9),
            render_time: std::time::Duration::from_micros(10),
            rows_changed: 0,
            rows_total: 2,
            rtt: None,
            client_count: None,
            rendered_fps: Some(12),
            idle: true,
        };

        let active_text = format_stats_right(&active);
        let idle_text = format_stats_right(&idle);

        assert_eq!(active_text.chars().count(), idle_text.chars().count());
        assert!(active_text.contains("rtt:3ms"));
        assert!(active_text.contains("clients:5"));
        assert!(active_text.contains("fps: 12fps"));
        assert!(idle_text.contains("fps:idle"));
    }

    #[test]
    fn redraw_state_fast_paints_speculative_append_for_single_pane() {
        let mut state = RedrawState::new();
        state.terminal_cols = 120;

        let ws = local::WorkspaceSummary {
            session_id: "local".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            cols: 80,
            rows: 24,
            resize_policy: protocol::ResizePolicy::Fixed,
            pane_tree: None,
            tabs: Vec::new(),
        };

        let _ = state.render_initial_text(&ws, "ready");
        let prediction = local::SpeculativeEchoPrediction {
            pane_id: "pane-1".to_owned(),
            base_version: 1,
            input_seq: 1,
            row: 0,
            col: 5,
            text: "x".to_owned(),
        };

        let update = state
            .render_speculative_append_text(&ws, "ready\x1b[4mx\x1b[24m", &prediction)
            .expect("fast speculative render");

        assert!(
            update.contains("\x1b[1;6H\x1b[4mx\x1b[24m"),
            "fast path should only paint predicted cell at cursor: {update:?}"
        );
        assert_eq!(state.previous_rows[0], "ready\x1b[4mx\x1b[24m");
    }

    #[test]
    fn redraw_context_omits_metadata_rows_when_status_bar_is_present() {
        let ws = local::WorkspaceSummary {
            session_id: "local".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            cols: 80,
            rows: 24,
            resize_policy: protocol::ResizePolicy::Fixed,
            pane_tree: None,
            tabs: Vec::new(),
        };
        let metadata = local::TerminalMetadataSummary {
            title: "shell title".to_owned(),
            working_directory: "file://localhost/tmp/nmux".to_owned(),
        };

        let status_bar_text =
            redraw_text_with_context(&ws, &metadata, "pane output", None, true, None, None);
        assert!(
            !status_bar_text.contains("title=") && !status_bar_text.contains("working-directory="),
            "status-bar redraw should not inject metadata rows: {status_bar_text:?}"
        );
        assert!(
            status_bar_text
                .lines()
                .next()
                .is_some_and(|line| line.contains("Sessions")),
            "ratatui menu should remain visible on row 1: {status_bar_text:?}"
        );
        assert!(status_bar_text.contains("pane output"));

        let fallback_text =
            redraw_text_with_context(&ws, &metadata, "pane output", None, false, None, None);
        assert!(
            fallback_text.contains("title=shell title")
                && fallback_text.contains("working-directory=file://localhost/tmp/nmux"),
            "non-status redraw should keep metadata rows: {fallback_text:?}"
        );
    }

    #[test]
    fn frontend_resize_uses_ratatui_pane_content_size() {
        let workspace = local::WorkspaceSummary {
            session_id: "local".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-2".to_owned(),
            cols: 40,
            rows: 24,
            resize_policy: protocol::ResizePolicy::Fixed,
            pane_tree: Some(local::WorkspacePaneSummary {
                pane_id: "pane-root".to_owned(),
                cols: 80,
                rows: 24,
                resize_policy: protocol::ResizePolicy::Fixed,
                split_axis: protocol::SplitAxis::Vertical,
                children: vec![
                    local::WorkspacePaneSummary {
                        pane_id: "pane-1".to_owned(),
                        cols: 40,
                        rows: 24,
                        resize_policy: protocol::ResizePolicy::Fixed,
                        split_axis: protocol::SplitAxis::None,
                        children: Vec::new(),
                    },
                    local::WorkspacePaneSummary {
                        pane_id: "pane-2".to_owned(),
                        cols: 40,
                        rows: 24,
                        resize_policy: protocol::ResizePolicy::Fixed,
                        split_axis: protocol::SplitAxis::None,
                        children: Vec::new(),
                    },
                ],
            }),
            tabs: Vec::new(),
        };

        assert_eq!(
            frontend_resize_pane_size(&workspace, "pane-2", 100, 20, true),
            (35, 16)
        );
        assert_eq!(
            frontend_resize_pane_size(&workspace, "pane-2", 100, 20, false),
            (100, 20)
        );
    }

    #[test]
    fn redraw_workspace_surface_text_renders_split_panes() {
        let workspace = local::WorkspaceSummary {
            session_id: "local".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-2".to_owned(),
            cols: 40,
            rows: 24,
            resize_policy: protocol::ResizePolicy::Fixed,
            pane_tree: Some(local::WorkspacePaneSummary {
                pane_id: "pane-root".to_owned(),
                cols: 80,
                rows: 24,
                resize_policy: protocol::ResizePolicy::Fixed,
                split_axis: protocol::SplitAxis::Vertical,
                children: vec![
                    local::WorkspacePaneSummary {
                        pane_id: "pane-1".to_owned(),
                        cols: 40,
                        rows: 24,
                        resize_policy: protocol::ResizePolicy::Fixed,
                        split_axis: protocol::SplitAxis::None,
                        children: Vec::new(),
                    },
                    local::WorkspacePaneSummary {
                        pane_id: "pane-2".to_owned(),
                        cols: 40,
                        rows: 24,
                        resize_policy: protocol::ResizePolicy::Fixed,
                        split_axis: protocol::SplitAxis::None,
                        children: Vec::new(),
                    },
                ],
            }),
            tabs: Vec::new(),
        };
        let mut surfaces = BTreeMap::new();
        surfaces.insert("pane-1".to_owned(), "left cached".to_owned());

        let text = redraw_workspace_surface_text(&workspace, "right active", Some(&surfaces));

        assert!(text.contains("[pane-1]"), "{text:?}");
        assert!(text.contains("[pane-2 active]"), "{text:?}");
        assert!(text.contains("left cached"), "{text:?}");
        assert!(text.contains("right active"), "{text:?}");
        assert!(text.contains(" | "), "{text:?}");
    }

    #[test]
    fn raw_terminal_mode_fixup_clears_flow_control_and_handles_local_echo() {
        let mut original = zero_termios();
        original.c_lflag = libc::ICANON | libc::ISIG | libc::IEXTEN;
        original.c_iflag = libc::IXON | libc::IXOFF | libc::ICRNL;
        original.c_oflag = libc::OPOST;

        let raw = raw_terminal_fixup_termios(original, LocalEcho::Off);
        assert_eq!(raw.c_lflag & libc::ICANON, libc::ICANON);
        assert_eq!(raw.c_lflag & libc::ECHO, 0);
        assert_eq!(raw.c_lflag & libc::ISIG, libc::ISIG);
        assert_eq!(raw.c_lflag & libc::IEXTEN, libc::IEXTEN);
        assert_eq!(raw.c_iflag & libc::IXON, libc::IXON);
        assert_eq!(raw.c_iflag & libc::IXOFF, 0);
        assert_eq!(raw.c_iflag & libc::ICRNL, libc::ICRNL);
        assert_eq!(raw.c_oflag & libc::OPOST, libc::OPOST);

        let raw = raw_terminal_fixup_termios(original, LocalEcho::Tty);
        assert_eq!(raw.c_lflag & libc::ECHO, libc::ECHO);
        assert_eq!(raw.c_iflag & libc::IXOFF, 0);
    }

    #[test]
    fn host_mouse_mode_mirror_is_limited_to_interactive_redraw_byte_mode() {
        assert!(host_mouse_mode_mirror_needed(HostMouseModeContext {
            stdin_bytes: true,
            redraw: true,
            stdout_is_tty: true,
        }));
        assert!(!host_mouse_mode_mirror_needed(HostMouseModeContext {
            stdin_bytes: false,
            redraw: true,
            stdout_is_tty: true,
        }));
        assert!(!host_mouse_mode_mirror_needed(HostMouseModeContext {
            stdin_bytes: true,
            redraw: false,
            stdout_is_tty: true,
        }));
        assert!(!host_mouse_mode_mirror_needed(HostMouseModeContext {
            stdin_bytes: true,
            redraw: true,
            stdout_is_tty: false,
        }));
    }

    #[test]
    fn host_mouse_mode_sequences_mirror_daemon_modes() {
        assert_eq!(
            host_mouse_mode_disable_sequence(),
            "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1016l"
        );
        assert_eq!(
            host_mouse_mode_enable_sequence(local::TerminalModeSummary {
                mouse_tracking: false,
                mouse_tracking_mode: protocol::MouseTrackingMode::None,
                mouse_format: protocol::MouseFormat::X10,
                ..local::TerminalModeSummary::default()
            }),
            "\x1b[?1000h\x1b[?1006h"
        );
        assert_eq!(
            host_mouse_mode_enable_sequence(local::TerminalModeSummary {
                mouse_tracking: true,
                mouse_tracking_mode: protocol::MouseTrackingMode::Normal,
                mouse_format: protocol::MouseFormat::Sgr,
                ..local::TerminalModeSummary::default()
            }),
            "\x1b[?1000h\x1b[?1006h"
        );
        assert_eq!(
            host_mouse_mode_enable_sequence(local::TerminalModeSummary {
                mouse_tracking: true,
                mouse_tracking_mode: protocol::MouseTrackingMode::Any,
                mouse_format: protocol::MouseFormat::SgrPixels,
                ..local::TerminalModeSummary::default()
            }),
            "\x1b[?1003h\x1b[?1006h\x1b[?1016h"
        );
    }

    #[test]
    fn local_echo_arg_accepts_explicit_choices() {
        assert_eq!(parse_local_echo("off"), Ok(LocalEcho::Off));
        assert_eq!(parse_local_echo("tty"), Ok(LocalEcho::Tty));
        assert!(parse_local_echo("auto").is_err());
    }

    #[test]
    fn detach_key_arg_accepts_explicit_choices() {
        assert_eq!(parse_detach_key("ctrl-]"), Ok(DetachKey::CtrlRightBracket));
        assert_eq!(parse_detach_key("none"), Ok(DetachKey::None));
        assert!(parse_detach_key("ctrl-c").is_err());
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
                live: true,
                detach_key_set: true,
                ..ClientModeArgs::default()
            }),
            Err("--detach-key requires --stdin-bytes")
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
                speculative_echo: true,
                ..ClientModeArgs::default()
            }),
            Err("--speculative-echo requires --live")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                live: true,
                speculative_echo: true,
                ..ClientModeArgs::default()
            }),
            Err("--speculative-echo requires --redraw")
        );
        assert_eq!(
            super_validate_mode_args(ClientModeArgs {
                live: true,
                redraw: true,
                speculative_echo: true,
                ..ClientModeArgs::default()
            }),
            Err("--speculative-echo requires --key")
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
                speculative_echo: true,
                key_set: true,
                live_resize: Some((80, 24)),
                iterations: Some(1),
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
                output_json: true,
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
            validate_no_input_resize_args(NoInputResizeArgs {
                no_input_set: true,
                live_resize: Some((80, 24)),
            }),
            Err("--no-input cannot be combined with --cols/--rows")
        );
        assert!(
            validate_no_input_resize_args(NoInputResizeArgs {
                no_input_set: true,
                live_resize: None,
            })
            .is_ok()
        );
        assert!(
            validate_no_input_resize_args(NoInputResizeArgs {
                no_input_set: false,
                live_resize: Some((80, 24)),
            })
            .is_ok()
        );
    }

    #[test]
    fn numeric_validation_rejects_zero_live_loop_values() {
        assert_eq!(
            validate_positive_numeric_args(PositiveNumericArgs {
                interval_ms: 0,
                ..positive_numeric_defaults()
            }),
            Err("--interval-ms must be greater than 0")
        );
        assert_eq!(
            validate_positive_numeric_args(PositiveNumericArgs {
                live_resize: Some((0, 24)),
                ..positive_numeric_defaults()
            }),
            Err("--cols and --rows must be between 1 and 65535")
        );
        assert_eq!(
            validate_positive_numeric_args(PositiveNumericArgs {
                live_resize: Some((80, 0)),
                ..positive_numeric_defaults()
            }),
            Err("--cols and --rows must be between 1 and 65535")
        );
        assert_eq!(
            validate_positive_numeric_args(PositiveNumericArgs {
                live_resize: Some((65536, 24)),
                ..positive_numeric_defaults()
            }),
            Err("--cols and --rows must be between 1 and 65535")
        );
        assert_eq!(
            validate_positive_numeric_args(PositiveNumericArgs {
                live_resize: Some((80, 65536)),
                ..positive_numeric_defaults()
            }),
            Err("--cols and --rows must be between 1 and 65535")
        );
        assert_eq!(
            validate_positive_numeric_args(PositiveNumericArgs {
                connect_timeout_ms: Some(0),
                ..positive_numeric_defaults()
            }),
            Err("--connect-timeout-ms must be greater than 0")
        );
        assert_eq!(
            validate_positive_numeric_args(PositiveNumericArgs {
                startup_timeout_ms: 0,
                ..positive_numeric_defaults()
            }),
            Err("--startup-timeout-ms must be greater than 0")
        );
        assert!(
            validate_positive_numeric_args(PositiveNumericArgs {
                connect_timeout_ms: Some(1),
                ..positive_numeric_defaults()
            })
            .is_ok()
        );
        assert!(
            validate_positive_numeric_args(PositiveNumericArgs {
                scrollback_tail_count: Some(1),
                ..positive_numeric_defaults()
            })
            .is_ok()
        );
        assert!(validate_positive_numeric_args(positive_numeric_defaults()).is_ok());
        assert!(
            validate_positive_numeric_args(PositiveNumericArgs {
                live_resize: Some((65535, 65535)),
                ..positive_numeric_defaults()
            })
            .is_ok()
        );
    }

    #[test]
    fn numeric_validation_rejects_zero_scrollback_values() {
        assert_eq!(
            validate_positive_numeric_args(PositiveNumericArgs {
                scrollback_start_line: 0,
                ..positive_numeric_defaults()
            }),
            Err("--scrollback-start must be greater than 0")
        );
        assert_eq!(
            validate_positive_numeric_args(PositiveNumericArgs {
                scrollback_line_count: 0,
                ..positive_numeric_defaults()
            }),
            Err("--scrollback-count must be greater than 0")
        );
        assert_eq!(
            validate_positive_numeric_args(PositiveNumericArgs {
                scrollback_tail_count: Some(0),
                ..positive_numeric_defaults()
            }),
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
    fn speculative_echo_arg_is_live_redraw_only() {
        let args = args_from_iter(["--live", "--redraw", "--key", "x", "--speculative-echo"])
            .expect("parse speculative echo args");
        assert!(args.speculative_echo);
        assert!(args.live);
        assert_eq!(args.input_text.as_deref(), Some("x"));
        assert!(args.redraw);

        let err = match args_from_iter(["--live", "--speculative-echo"]) {
            Ok(_) => panic!("speculative echo without redraw should fail"),
            Err(err) => err.to_string(),
        };
        assert_eq!(err, "--speculative-echo requires --redraw");

        let err =
            match args_from_iter(["--live", "--redraw", "--stdin-bytes", "--speculative-echo"]) {
                Ok(_) => panic!("speculative echo with raw byte mode should fail"),
                Err(err) => err.to_string(),
            };
        assert_eq!(err, "--speculative-echo requires --key");
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
        let err = parse_numeric_arg::<u64>("--interval-ms", "slow")
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
        assert!(usage.contains("--tcp HOST:PORT"));
        assert!(usage.contains("--tcp-token, --token TOKEN"));
        assert!(usage.contains("-s, --session NAME"));
        assert!(usage.contains("--state-info"));
        assert!(usage.contains("--state-info-json"));
        assert!(usage.contains("--tab TAB_ID"));
        assert!(usage.contains("--actor-id ID"));
        assert!(usage.contains("--user-id ID"));
        assert!(usage.contains("--display-name NAME"));
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
        assert!(usage.contains("pane send PANE_ID TEXT"));
        assert!(usage.contains("pane ls [--json]"));
        assert!(usage.contains("pane read PANE_ID [--json]"));
        assert!(usage.contains("pane snapshot PANE_ID --json"));
        assert!(usage.contains("pane split AXIS [PANE_ID]"));
        assert!(usage.contains("tab new [TAB_ID]"));
        assert!(usage.contains("tab ls [--json]"));
        assert!(usage.contains("tab switch TAB_ID"));
        assert!(usage.contains("tab close [TAB_ID]"));
        assert!(usage.contains("send-keys [-t PANE_ID] KEYS..."));
        assert!(usage.contains("version [--json]"));
        assert!(usage.contains("--start waits for nmux daemon --ready-json"));
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
    fn scrollback_view_summary_preserves_runs_and_styles() {
        let mut scrollback = scrollback_summary(4, 9, &[(4, "four"), (5, "five")]);
        scrollback.styles.push(local::StyleSummary {
            fg_rgba: 0xff0000ff,
            bg_rgba: 0,
            underline_rgba: 0,
            flags: 1,
        });
        scrollback.lines[0].runs = vec![local::CellRunSummary {
            text: "four".to_owned(),
            cell_widths: vec![1, 1, 1, 1],
            style_id: 0,
            flags: 0,
            hyperlink_id: 0,
            semantic_content: protocol::CellSemanticContent::Output,
        }];
        scrollback.lines[0].dirty_hash = 12;
        scrollback.lines[0].row_state_hash = 13;
        scrollback.lines[0].semantic_prompt = protocol::RowSemanticPrompt::Prompt;
        scrollback.lines[0].dirty = true;

        let summary = render_scrollback_view_summary(&scrollback);

        assert_eq!(summary.pane_id, "pane-1");
        assert_eq!(summary.version, 1);
        assert_eq!(summary.rows, 2);
        assert_eq!(summary.styles, scrollback.styles);
        assert_eq!(summary.row_updates[0].row, 0);
        assert_eq!(summary.row_updates[0].text, "four");
        assert_eq!(summary.row_updates[0].runs, scrollback.lines[0].runs);
        assert_eq!(summary.row_updates[0].dirty_hash, 12);
        assert_eq!(summary.row_updates[0].row_state_hash, 13);
        assert_eq!(
            summary.row_updates[0].semantic_prompt,
            protocol::RowSemanticPrompt::Prompt
        );
        assert!(summary.row_updates[0].dirty);
    }

    #[test]
    fn stdin_bytes_detach_splits_before_ctrl_right_bracket() {
        assert_eq!(
            split_stdin_bytes_for_detach(b"ping\n", Some(STDIN_BYTES_DETACH)),
            (Some(b"ping\n".to_vec()), false)
        );
        assert_eq!(
            split_stdin_bytes_for_detach(b"ping\n\x1dignored", Some(STDIN_BYTES_DETACH)),
            (Some(b"ping\n".to_vec()), true)
        );
        assert_eq!(
            split_stdin_bytes_for_detach(b"\x1d", Some(STDIN_BYTES_DETACH)),
            (None, true)
        );
        assert_eq!(
            split_stdin_bytes_for_detach(b"ping\n\x1d", None),
            (Some(b"ping\n\x1d".to_vec()), false)
        );
    }

    #[test]
    fn stdin_bytes_decode_bracketed_paste_for_forwarding() {
        let input = [
            b"before".as_slice(),
            BRACKETED_PASTE_START,
            b"pasted\ntext".as_slice(),
            BRACKETED_PASTE_END,
            b"after".as_slice(),
        ]
        .concat();

        assert_eq!(
            stdin_byte_forwards(&input),
            vec![
                StdinByteForward::Raw(b"before".to_vec()),
                StdinByteForward::Paste("pasted\ntext".to_owned()),
                StdinByteForward::Raw(b"after".to_vec()),
            ]
        );
    }

    #[test]
    fn stdin_bytes_keep_incomplete_or_non_utf8_bracketed_paste_raw() {
        assert_eq!(
            stdin_byte_forwards(b"\x1b[200~unterminated"),
            vec![StdinByteForward::Raw(b"\x1b[200~unterminated".to_vec())]
        );

        let input = [BRACKETED_PASTE_START, &[0xff, b'a'], BRACKETED_PASTE_END].concat();
        assert_eq!(
            stdin_byte_forwards(&input),
            vec![StdinByteForward::Raw(b"\x1b[200~\xffa\x1b[201~".to_vec())]
        );
    }

    #[test]
    fn stdin_bytes_decode_sgr_mouse_for_forwarding() {
        assert_eq!(
            stdin_byte_forwards(b"before\x1b[<64;12;5Mafter"),
            vec![
                StdinByteForward::Raw(b"before".to_vec()),
                StdinByteForward::Mouse(SgrMouseInput {
                    row: 4,
                    col: 11,
                    button: protocol::MouseButton::WheelUp,
                    action: protocol::MouseAction::Press,
                    modifiers: 0,
                }),
                StdinByteForward::Raw(b"after".to_vec()),
            ]
        );

        assert_eq!(
            stdin_byte_forwards(b"\x1b[<21;3;2m"),
            vec![StdinByteForward::Mouse(SgrMouseInput {
                row: 1,
                col: 2,
                button: protocol::MouseButton::Middle,
                action: protocol::MouseAction::Release,
                modifiers: 3,
            })]
        );
    }

    #[test]
    fn stdin_bytes_decode_tui_keyboard_navigation_for_forwarding() {
        assert_eq!(
            stdin_byte_forwards(b"before\x1b[A\x1bw\rafter"),
            vec![
                StdinByteForward::Raw(b"before".to_vec()),
                StdinByteForward::Key(StdinKeyInput {
                    key: StdinKey::Up,
                    bytes: b"\x1b[A".to_vec(),
                }),
                StdinByteForward::Key(StdinKeyInput {
                    key: StdinKey::OpenMenu(tui::MenuAction::Windows),
                    bytes: b"\x1bw".to_vec(),
                }),
                StdinByteForward::Key(StdinKeyInput {
                    key: StdinKey::Enter,
                    bytes: b"\r".to_vec(),
                }),
                StdinByteForward::Raw(b"after".to_vec()),
            ]
        );
    }

    #[test]
    fn live_mouse_routing_targets_active_pane_content() {
        let workspace = local::WorkspaceSummary {
            session_id: "local".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            cols: 20,
            rows: 8,
            resize_policy: protocol::ResizePolicy::Fixed,
            pane_tree: None,
            tabs: Vec::new(),
        };
        let modes = local::TerminalModeSummary {
            mouse_tracking: true,
            mouse_tracking_mode: protocol::MouseTrackingMode::Normal,
            mouse_format: protocol::MouseFormat::Sgr,
            ..local::TerminalModeSummary::default()
        };
        let Some(LiveMouseDispatch::PaneMouse(pane_id, mouse)) =
            live_mouse_dispatch_for_workspace_size(
                SgrMouseInput {
                    row: 2,
                    col: 2,
                    button: protocol::MouseButton::WheelDown,
                    action: protocol::MouseAction::Press,
                    modifiers: 0,
                },
                &workspace,
                "ready",
                None,
                None,
                modes,
                None,
                80,
                24,
            )
        else {
            panic!("pane content should receive wheel input");
        };

        assert_eq!(pane_id, "pane-1");
        assert_eq!(mouse.row, 0);
        assert_eq!(mouse.col, 1);
        assert_eq!(mouse.button, protocol::MouseButton::WheelDown);

        assert_eq!(
            live_mouse_dispatch_for_workspace_size(
                SgrMouseInput {
                    row: 2,
                    col: 2,
                    button: protocol::MouseButton::WheelUp,
                    action: protocol::MouseAction::Press,
                    modifiers: 0,
                },
                &workspace,
                "ready",
                None,
                None,
                local::TerminalModeSummary::default(),
                None,
                80,
                24,
            ),
            Some(LiveMouseDispatch::PaneScroll {
                pane_id: "pane-1".to_owned(),
                direction: LiveScrollDirection::Up,
                visible_rows: 20,
            }),
            "pane content wheel scrolls nmux-owned scrollback when the pane app has not enabled mouse tracking"
        );

        assert_eq!(
            live_mouse_dispatch_for_workspace_size(
                SgrMouseInput {
                    row: 1,
                    col: 0,
                    button: protocol::MouseButton::Left,
                    action: protocol::MouseAction::Press,
                    modifiers: 0,
                },
                &workspace,
                "ready",
                None,
                None,
                modes,
                None,
                80,
                24,
            ),
            Some(LiveMouseDispatch::FocusPane("pane-1".to_owned())),
            "pane chrome clicks select the pane"
        );
    }

    #[test]
    fn live_mouse_routing_focuses_inactive_tree_pane() {
        let workspace = local::WorkspaceSummary {
            session_id: "local".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-2".to_owned(),
            cols: 40,
            rows: 10,
            resize_policy: protocol::ResizePolicy::Fixed,
            pane_tree: Some(local::WorkspacePaneSummary {
                pane_id: "pane-root".to_owned(),
                cols: 80,
                rows: 10,
                resize_policy: protocol::ResizePolicy::Fixed,
                split_axis: protocol::SplitAxis::Vertical,
                children: vec![
                    local::WorkspacePaneSummary {
                        pane_id: "pane-1".to_owned(),
                        cols: 40,
                        rows: 10,
                        resize_policy: protocol::ResizePolicy::Fixed,
                        split_axis: protocol::SplitAxis::None,
                        children: Vec::new(),
                    },
                    local::WorkspacePaneSummary {
                        pane_id: "pane-2".to_owned(),
                        cols: 40,
                        rows: 10,
                        resize_policy: protocol::ResizePolicy::Fixed,
                        split_axis: protocol::SplitAxis::None,
                        children: Vec::new(),
                    },
                ],
            }),
            tabs: Vec::new(),
        };

        assert_eq!(
            live_mouse_dispatch_for_workspace_size(
                SgrMouseInput {
                    row: 4,
                    col: 2,
                    button: protocol::MouseButton::Left,
                    action: protocol::MouseAction::Press,
                    modifiers: 0,
                },
                &workspace,
                "right active",
                None,
                None,
                local::TerminalModeSummary::default(),
                None,
                100,
                20,
            ),
            Some(LiveMouseDispatch::FocusPane("pane-1".to_owned()))
        );

        assert_eq!(
            live_mouse_dispatch_for_workspace_size(
                SgrMouseInput {
                    row: 4,
                    col: 2,
                    button: protocol::MouseButton::WheelDown,
                    action: protocol::MouseAction::Press,
                    modifiers: 0,
                },
                &workspace,
                "right active",
                None,
                None,
                local::TerminalModeSummary::default(),
                None,
                100,
                20,
            ),
            None,
            "wheel over the tree is consumed until nmux-owned scrolling exists"
        );
    }

    #[test]
    fn live_mouse_menu_click_opens_menu_overlay() {
        let workspace = local::WorkspaceSummary {
            session_id: "local".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            cols: 20,
            rows: 8,
            resize_policy: protocol::ResizePolicy::Fixed,
            pane_tree: None,
            tabs: Vec::new(),
        };

        assert_eq!(
            live_mouse_dispatch_for_workspace_size(
                SgrMouseInput {
                    row: 0,
                    col: 1,
                    button: protocol::MouseButton::Left,
                    action: protocol::MouseAction::Press,
                    modifiers: 0,
                },
                &workspace,
                "ready",
                None,
                None,
                local::TerminalModeSummary::default(),
                None,
                80,
                24,
            ),
            Some(LiveMouseDispatch::Menu(tui::MenuAction::Sessions))
        );

        let overlay = menu_overlay_for_action(
            tui::MenuAction::Windows,
            &workspace,
            &LiveSurfaceState {
                current_surface_metadata: local::TerminalMetadataSummary::default(),
                current_modes: local::TerminalModeSummary::default(),
                current_surface_text: String::new(),
                current_pane_surfaces: BTreeMap::new(),
                current_pane_surface_summaries: BTreeMap::new(),
                current_pane_modes: BTreeMap::new(),
                scrollback_views: BTreeMap::new(),
            },
        );
        assert_eq!(overlay.title, "windows");
        assert!(
            overlay
                .lines
                .iter()
                .any(|line| line.text.contains("* pane-1"))
        );

        let new_session_overlay = menu_overlay_for_action(
            tui::MenuAction::NewSession,
            &workspace,
            &LiveSurfaceState {
                current_surface_metadata: local::TerminalMetadataSummary::default(),
                current_modes: local::TerminalModeSummary::default(),
                current_surface_text: String::new(),
                current_pane_surfaces: BTreeMap::new(),
                current_pane_surface_summaries: BTreeMap::new(),
                current_pane_modes: BTreeMap::new(),
                scrollback_views: BTreeMap::new(),
            },
        );
        assert_eq!(new_session_overlay.title, "new session");
        assert!(
            new_session_overlay
                .lines
                .iter()
                .any(|line| line.text.contains("new named session"))
        );
    }

    #[test]
    fn live_new_session_fallback_is_limited_to_single_actor_rejection() {
        let fallback = local::ServerError {
            error: local::ErrorSummary {
                code: protocol::ErrorCode::Unknown,
                message: "session new requires daemon registry routing".to_owned(),
                retryable: false,
                pane_id: None,
                input_seq: 1,
            },
        };
        assert!(live_session_new_should_fallback(&fallback));

        let duplicate = local::ServerError {
            error: local::ErrorSummary {
                code: protocol::ErrorCode::Unknown,
                message: "session already exists: work".to_owned(),
                retryable: false,
                pane_id: None,
                input_seq: 1,
            },
        };
        assert!(!live_session_new_should_fallback(&duplicate));
        assert!(!live_session_new_should_fallback(&io::Error::other(
            "transport failed"
        )));
    }

    #[test]
    fn live_mouse_sessions_overlay_click_switches_session() {
        let root = local::WorkspacePaneSummary {
            pane_id: "pane-1".to_owned(),
            cols: 20,
            rows: 8,
            resize_policy: protocol::ResizePolicy::Fixed,
            split_axis: protocol::SplitAxis::None,
            children: Vec::new(),
        };
        let workspace = local::WorkspaceSummary {
            session_id: "local".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            cols: 20,
            rows: 8,
            resize_policy: protocol::ResizePolicy::Fixed,
            pane_tree: Some(root.clone()),
            tabs: vec![
                local::WorkspaceTabSummary {
                    tab_id: "tab-1".to_owned(),
                    title: "main".to_owned(),
                    active_pane_id: "pane-1".to_owned(),
                    root: root.clone(),
                },
                local::WorkspaceTabSummary {
                    tab_id: "tab-2".to_owned(),
                    title: "work".to_owned(),
                    active_pane_id: "tab-2-pane-1".to_owned(),
                    root: local::WorkspacePaneSummary {
                        pane_id: "tab-2-pane-1".to_owned(),
                        cols: 20,
                        rows: 8,
                        resize_policy: protocol::ResizePolicy::Fixed,
                        split_axis: protocol::SplitAxis::None,
                        children: Vec::new(),
                    },
                },
            ],
        };
        let overlay = menu_overlay_for_action(
            tui::MenuAction::Sessions,
            &workspace,
            &LiveSurfaceState {
                current_surface_metadata: local::TerminalMetadataSummary::default(),
                current_modes: local::TerminalModeSummary::default(),
                current_surface_text: String::new(),
                current_pane_surfaces: BTreeMap::new(),
                current_pane_surface_summaries: BTreeMap::new(),
                current_pane_modes: BTreeMap::new(),
                scrollback_views: BTreeMap::new(),
            },
        );
        assert!(
            overlay
                .lines
                .iter()
                .any(|line| line.text.contains("session local"))
        );

        let inventory = local::SessionInventorySummary {
            active_session_id: "local".to_owned(),
            sessions: vec![
                local::SessionInventoryItemSummary {
                    session_id: "local".to_owned(),
                    title: "main".to_owned(),
                },
                local::SessionInventoryItemSummary {
                    session_id: "work".to_owned(),
                    title: "Work".to_owned(),
                },
            ],
        };
        let overlay = menu_overlay_for_action_with_session_inventory(
            tui::MenuAction::Sessions,
            &workspace,
            &LiveSurfaceState {
                current_surface_metadata: local::TerminalMetadataSummary::default(),
                current_modes: local::TerminalModeSummary::default(),
                current_surface_text: String::new(),
                current_pane_surfaces: BTreeMap::new(),
                current_pane_surface_summaries: BTreeMap::new(),
                current_pane_modes: BTreeMap::new(),
                scrollback_views: BTreeMap::new(),
            },
            Some(&inventory),
        );
        assert!(
            overlay
                .lines
                .iter()
                .any(|line| line.text.contains("work Work")
                    && line.action == Some(tui::OverlayAction::SwitchSession("work".to_owned())))
        );

        assert_eq!(
            live_mouse_dispatch_for_workspace_size(
                SgrMouseInput {
                    row: 10,
                    col: 39,
                    button: protocol::MouseButton::Left,
                    action: protocol::MouseAction::Press,
                    modifiers: 0,
                },
                &workspace,
                "ready",
                None,
                None,
                local::TerminalModeSummary::default(),
                Some(&overlay),
                80,
                24,
            ),
            Some(LiveMouseDispatch::Overlay(
                tui::OverlayAction::SwitchSession("work".to_owned())
            ))
        );
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
