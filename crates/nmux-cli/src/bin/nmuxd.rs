use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::str::FromStr;
use std::thread;
use std::time::{Duration, Instant};

use clap::{ArgAction, Parser, ValueEnum};
use nmux_cli::local;
use nmux_core::host::{CommandSpec, LocalPtyHost, ProcessHost};
use nmux_core::session::Session;
use nmux_core::terminal::{PaneTerminalEngines, TerminalEngineKind};
use nmux_proto::protocol;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const RESIZE_POLICY_NAMES: &[&str] = &["fixed", "leader", "active-client", "manual"];
const SPLIT_AXIS_NAMES: &[&str] = &["horizontal", "vertical"];
const TERMINAL_ENGINE_NAMES: &[&str] = &["interim", "libghostty-vt"];

fn main() {
    if let Err(err) = run() {
        eprintln!("nmuxd: {err}");
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
            println!("{}", local::version_json("nmuxd", VERSION));
        } else {
            println!("nmuxd {VERSION}");
        }
        return Ok(());
    }

    if args.list_daemon_choices_json {
        println!("{}", format_daemon_choices_json());
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

    let listener = match local::bind_listener(&args.socket_path) {
        Ok(listener) => listener,
        Err(err) => {
            report_ready_json_error(&args, &err)?;
            return Err(err.into());
        }
    };
    let _socket_cleanup = SocketCleanup::new(args.socket_path.clone());
    eprintln!("nmuxd: listening on {}", args.socket_path.display());
    let ready_json = if args.ready_json {
        Some(format_ready_json(&args))
    } else {
        None
    };

    let mut session = Session::initial();
    if let Some(command) = args.command.as_deref() {
        session.tabs[0].root.host.command = CommandSpec::new("sh").with_args(["-lc", &command]);
    }
    if let Some((cols, rows)) = args.initial_size {
        session.tabs[0].root.cols = cols;
        session.tabs[0].root.rows = rows;
        session.tabs[0].root.host.command.initial_size = Some((cols, rows));
    }
    if let Some(working_dir) = args.working_dir.as_ref() {
        session.tabs[0].root.host.command.working_dir = Some(working_dir.clone());
    }
    if !args.env.is_empty() {
        session.tabs[0]
            .root
            .host
            .command
            .env
            .extend(args.env.iter().cloned());
    }
    if let Some(axis) = args.initial_split {
        let host = session
            .pane_host("pane-1")
            .ok_or("initial pane missing host")?
            .clone();
        if !session.split_active_pane(axis, "pane-2", host) {
            return Err("failed to create initial split pane".into());
        }
    }
    let pane_ids = session.leaf_pane_ids();
    let inherited_origin = inherited_nmux_origin();
    for pane_id in &pane_ids {
        session.set_pane_resize_policy(pane_id, args.resize_policy);
        session.set_pane_nmux_environment(
            pane_id,
            args.socket_path.display().to_string(),
            inherited_origin.as_deref(),
        );
    }
    let mut pty_host = LocalPtyHost::default();
    for pane_id in &pane_ids {
        let host_spec = session
            .pane_host(pane_id)
            .ok_or_else(|| format!("pane {pane_id} missing host"))?
            .clone();
        if let Err(err) = pty_host.start_pane(pane_id, &host_spec) {
            report_ready_json_error(&args, &err)?;
            return Err(Box::new(err));
        }
    }
    let mut terminal_engines = PaneTerminalEngines::new(args.terminal_engine_kind);
    if let Err(err) = wait_for_panes_output(
        &mut session,
        &mut pty_host,
        &pane_ids,
        &mut terminal_engines,
    ) {
        report_ready_json_error(&args, err.as_ref())?;
        return Err(err);
    }
    if let Some(ready_json) = ready_json {
        println!("{ready_json}");
        io::stdout().flush()?;
    }

    if args.live || args.live_forever || args.live_cycles.is_some() || args.live_clients.is_some() {
        let cycles = args.live_cycles.unwrap_or(usize::MAX);
        let clients = if args.live_forever {
            usize::MAX
        } else {
            args.live_clients.unwrap_or(1)
        };
        let serve_result = local::serve_live_n_with_host_and_engines(
            &listener,
            &mut session,
            &mut pty_host,
            clients,
            cycles,
            &mut terminal_engines,
        );
        let stop_result = stop_panes(&mut pty_host, &pane_ids);
        serve_result?;
        stop_result?;
        return Ok(());
    }

    if args.one_shot {
        let serve_result = local::serve_n_with_host_and_engines(
            &listener,
            &mut session,
            &mut pty_host,
            1,
            &mut terminal_engines,
        );
        let stop_result = stop_panes(&mut pty_host, &pane_ids);
        serve_result?;
        stop_result?;
        return Ok(());
    }

    loop {
        local::serve_n_with_host_and_engines(
            &listener,
            &mut session,
            &mut pty_host,
            1,
            &mut terminal_engines,
        )?;
    }
}

fn inherited_nmux_origin() -> Option<String> {
    if std::env::var("NMUX").ok().as_deref() != Some("1") {
        return None;
    }

    std::env::var("NMUX_ORIGIN")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

struct SocketCleanup {
    path: PathBuf,
    identity: Option<local::SocketIdentity>,
}

impl SocketCleanup {
    fn new(path: PathBuf) -> Self {
        let identity = local::socket_identity(&path).ok();
        Self { path, identity }
    }
}

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        if self.identity.is_some() && local::socket_identity(&self.path).ok() == self.identity {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn wait_for_panes_output(
    session: &mut Session,
    output: &mut LocalPtyHost,
    pane_ids: &[String],
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        let mut changed = false;
        for pane_id in pane_ids {
            changed |= local::poll_pane_output_with_engines(session, engines, output, pane_id)?;
        }
        if changed {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn stop_panes(
    host: &mut LocalPtyHost,
    pane_ids: &[String],
) -> Result<(), nmux_core::host::HostError> {
    for pane_id in pane_ids {
        host.stop_pane(pane_id)?;
    }
    Ok(())
}

struct Args {
    help: bool,
    version: bool,
    version_json: bool,
    list_daemon_choices_json: bool,
    print_socket: bool,
    print_socket_json: bool,
    ready_json: bool,
    socket_path: PathBuf,
    socket_source: local::SocketPathSource,
    one_shot: bool,
    live: bool,
    live_forever: bool,
    live_cycles: Option<usize>,
    live_clients: Option<usize>,
    command: Option<String>,
    working_dir: Option<String>,
    env: Vec<(String, String)>,
    initial_size: Option<(u32, u32)>,
    initial_split: Option<protocol::SplitAxis>,
    resize_policy: protocol::ResizePolicy,
    terminal_engine_kind: TerminalEngineKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ResizePolicyArg {
    Fixed,
    Leader,
    #[value(name = "active-client")]
    ActiveClient,
    Manual,
}

impl From<ResizePolicyArg> for protocol::ResizePolicy {
    fn from(value: ResizePolicyArg) -> Self {
        match value {
            ResizePolicyArg::Fixed => protocol::ResizePolicy::Fixed,
            ResizePolicyArg::Leader => protocol::ResizePolicy::Leader,
            ResizePolicyArg::ActiveClient => protocol::ResizePolicy::ActiveClient,
            ResizePolicyArg::Manual => protocol::ResizePolicy::Manual,
        }
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum TerminalEngineArg {
    Interim,
    #[value(name = "libghostty-vt")]
    LibghosttyVt,
}

impl TerminalEngineArg {
    fn into_terminal_engine_kind(self) -> Result<TerminalEngineKind, &'static str> {
        match self {
            Self::Interim => Ok(TerminalEngineKind::InterimText),
            #[cfg(feature = "libghostty-vt")]
            Self::LibghosttyVt => Ok(TerminalEngineKind::LibghosttyVt),
            #[cfg(not(feature = "libghostty-vt"))]
            Self::LibghosttyVt => Err("libghostty-vt requires the libghostty-vt feature"),
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "nmuxd",
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
    #[arg(long = "list-daemon-choices-json", action = ArgAction::SetTrue)]
    list_daemon_choices_json: bool,
    #[arg(long = "print-socket", action = ArgAction::SetTrue)]
    print_socket: bool,
    #[arg(long = "print-socket-json", action = ArgAction::SetTrue)]
    print_socket_json: bool,
    #[arg(long = "ready-json", action = ArgAction::SetTrue)]
    ready_json: bool,
    #[arg(long = "socket", value_name = "PATH")]
    socket_path: Option<PathBuf>,
    #[arg(long = "one-shot", action = ArgAction::SetTrue)]
    one_shot: bool,
    #[arg(long = "live", action = ArgAction::SetTrue)]
    live: bool,
    #[arg(long = "live-forever", action = ArgAction::SetTrue)]
    live_forever: bool,
    #[arg(
        long = "live-cycles",
        value_name = "COUNT",
        value_parser = parse_live_cycles_arg
    )]
    live_cycles: Option<usize>,
    #[arg(
        long = "live-clients",
        value_name = "COUNT",
        value_parser = parse_live_clients_arg
    )]
    live_clients: Option<usize>,
    #[arg(long = "command", value_name = "SHELL", allow_hyphen_values = true)]
    command: Option<String>,
    #[arg(long = "cwd", value_name = "DIR", allow_hyphen_values = true)]
    working_dir: Option<String>,
    #[arg(
        long = "env",
        value_name = "KEY=VALUE",
        value_parser = parse_env_assignment,
        allow_hyphen_values = true
    )]
    env: Vec<(String, String)>,
    #[arg(long = "cols", value_name = "COUNT", value_parser = parse_cols_arg)]
    cols: Option<u32>,
    #[arg(long = "rows", value_name = "COUNT", value_parser = parse_rows_arg)]
    rows: Option<u32>,
    #[arg(long = "split", value_name = "horizontal|vertical")]
    initial_split: Option<SplitAxisArg>,
    #[arg(
        long = "resize-policy",
        value_name = "fixed|leader|active-client|manual"
    )]
    resize_policy: Option<ResizePolicyArg>,
    #[arg(long = "terminal-engine", value_name = "interim|libghostty-vt")]
    terminal_engine_kind: Option<TerminalEngineArg>,
}

fn args() -> Result<Args, Box<dyn std::error::Error>> {
    args_from_iter(std::env::args())
}

fn args_from_iter<I, S>(args: I) -> Result<Args, Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let raw = RawArgs::try_parse_from(args).map_err(clap_error_message)?;
    let (socket_path, socket_source) = match raw.socket_path {
        Some(path) => (path, local::SocketPathSource::Explicit),
        None => local::default_socket_path_and_source(),
    };
    let live_cycles = raw.live_cycles;
    let live_clients = raw.live_clients;
    let working_dir = match raw.working_dir {
        Some(value) if value.is_empty() => {
            return Err("--cwd requires a non-empty directory path".into());
        }
        value => value,
    };
    let resize_policy = raw
        .resize_policy
        .map(protocol::ResizePolicy::from)
        .unwrap_or(protocol::ResizePolicy::Fixed);
    let initial_size = match (raw.cols, raw.rows) {
        (Some(cols), Some(rows)) => Some((cols, rows)),
        (None, None) => None,
        _ => return Err("--cols and --rows must be provided together".into()),
    };
    let initial_split = raw.initial_split.map(protocol::SplitAxis::from);
    let terminal_engine_kind = raw
        .terminal_engine_kind
        .map(TerminalEngineArg::into_terminal_engine_kind)
        .transpose()
        .map_err(|err| format!("--terminal-engine {err}"))?
        .unwrap_or(TerminalEngineKind::InterimText);

    if !(raw.help
        || raw.version
        || raw.version_json
        || raw.list_daemon_choices_json
        || raw.print_socket
        || raw.print_socket_json)
    {
        validate_mode_args(DaemonModeArgs {
            one_shot: raw.one_shot,
            live: raw.live,
            live_forever: raw.live_forever,
            live_cycles,
            live_clients,
        })?;
        validate_working_dir_arg(working_dir.as_deref())?;
        validate_initial_size(initial_size)?;
    }

    Ok(Args {
        help: raw.help,
        version: raw.version,
        version_json: raw.version_json,
        list_daemon_choices_json: raw.list_daemon_choices_json,
        print_socket: raw.print_socket,
        print_socket_json: raw.print_socket_json,
        ready_json: raw.ready_json,
        socket_path,
        socket_source,
        one_shot: raw.one_shot,
        live: raw.live,
        live_forever: raw.live_forever,
        live_cycles,
        live_clients,
        command: raw.command,
        working_dir,
        env: raw.env,
        initial_size,
        initial_split,
        resize_policy,
        terminal_engine_kind,
    })
}

fn format_daemon_choices_json() -> String {
    let resize_policies = format_json_string_array(RESIZE_POLICY_NAMES);
    let split_axes = format_json_string_array(SPLIT_AXIS_NAMES);
    let terminal_engines = TERMINAL_ENGINE_NAMES
        .iter()
        .map(|name| {
            format!(
                "{{\"name\":{},\"available\":{}}}",
                local::json_string(name),
                terminal_engine_available(name)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"resize_policies\":{resize_policies},\"split_axes\":{split_axes},\"terminal_engines\":[{terminal_engines}]}}"
    )
}

fn format_ready_json(args: &Args) -> String {
    format!(
        "{{\"event\":\"ready\",\"NMUX_SOCKET\":{},\"source\":{},\"mode\":{},\"terminal_engine\":{},\"resize_policy\":{}}}",
        local::json_string(&args.socket_path.display().to_string()),
        local::json_string(args.socket_source.label()),
        local::json_string(daemon_mode_name(args)),
        local::json_string(terminal_engine_kind_name(args.terminal_engine_kind)),
        local::json_string(resize_policy_name(args.resize_policy))
    )
}

fn report_ready_json_error(
    args: &Args,
    error: &(dyn std::error::Error + 'static),
) -> io::Result<()> {
    if args.ready_json {
        println!("{}", format_ready_error_json(error));
        io::stdout().flush()?;
    }
    Ok(())
}

fn format_ready_error_json(error: &(dyn std::error::Error + 'static)) -> String {
    format!(
        "{{\"event\":\"error\",\"error\":{{\"message\":{}}}}}",
        local::json_string(&error.to_string())
    )
}

fn daemon_mode_name(args: &Args) -> &'static str {
    if args.live_forever {
        "live-forever"
    } else if args.live_cycles.is_some() {
        "live-cycles"
    } else if args.live_clients.is_some() {
        "live-clients"
    } else if args.live {
        "live"
    } else if args.one_shot {
        "one-shot"
    } else {
        "serve"
    }
}

fn terminal_engine_kind_name(kind: TerminalEngineKind) -> &'static str {
    match kind {
        TerminalEngineKind::InterimText => "interim",
        #[cfg(feature = "libghostty-vt")]
        TerminalEngineKind::LibghosttyVt => "libghostty-vt",
    }
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

fn terminal_engine_available(name: &str) -> bool {
    match name {
        "interim" => true,
        #[cfg(feature = "libghostty-vt")]
        "libghostty-vt" => true,
        #[cfg(not(feature = "libghostty-vt"))]
        "libghostty-vt" => false,
        _ => false,
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

fn parse_numeric_arg<T>(flag: &str, value: String) -> Result<T, String>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|err| format!("{flag} requires a valid number: {err}"))
}

fn parse_live_cycles_arg(value: &str) -> Result<usize, String> {
    parse_numeric_arg("--live-cycles", value.to_owned())
}

fn parse_live_clients_arg(value: &str) -> Result<usize, String> {
    parse_numeric_arg("--live-clients", value.to_owned())
}

fn parse_cols_arg(value: &str) -> Result<u32, String> {
    parse_numeric_arg("--cols", value.to_owned())
}

fn parse_rows_arg(value: &str) -> Result<u32, String> {
    parse_numeric_arg("--rows", value.to_owned())
}

fn clap_error_message(error: clap::Error) -> String {
    let first_line = error.to_string();
    let first_line = first_line
        .lines()
        .next()
        .unwrap_or("invalid command line")
        .trim_start_matches("error: ")
        .to_owned();
    if let Some(index) = first_line.find(": --") {
        return first_line[index + 2..].to_owned();
    }
    first_line
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

fn validate_initial_size(size: Option<(u32, u32)>) -> Result<(), String> {
    if size.is_some_and(|(cols, rows)| {
        cols == 0 || rows == 0 || cols > u16::MAX as u32 || rows > u16::MAX as u32
    }) {
        return Err("--cols and --rows must be between 1 and 65535".to_owned());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
struct DaemonModeArgs {
    one_shot: bool,
    live: bool,
    live_forever: bool,
    live_cycles: Option<usize>,
    live_clients: Option<usize>,
}

fn validate_mode_args(args: DaemonModeArgs) -> Result<(), &'static str> {
    if args.one_shot
        && (args.live
            || args.live_forever
            || args.live_cycles.is_some()
            || args.live_clients.is_some())
    {
        return Err("--one-shot cannot be combined with live daemon modes");
    }
    if args.live && (args.live_forever || args.live_cycles.is_some() || args.live_clients.is_some())
    {
        return Err(
            "--live cannot be combined with --live-forever, --live-cycles, or --live-clients",
        );
    }
    if args.live_forever && (args.live_cycles.is_some() || args.live_clients.is_some()) {
        return Err("--live-forever cannot be combined with --live-cycles or --live-clients");
    }
    if args.live_cycles == Some(0) {
        return Err("--live-cycles must be greater than 0");
    }
    if args.live_clients == Some(0) {
        return Err("--live-clients must be greater than 0");
    }
    Ok(())
}

fn usage() -> &'static str {
    "\
nmuxd - serve an nmux session over a local Unix socket

Usage:
  nmuxd [OPTIONS]

Options:
  --socket PATH                         Unix socket path
  --print-socket                        Print the resolved socket path and exit
  --print-socket-json                   Print the resolved socket path/source as JSON
  --list-daemon-choices-json            List daemon configuration choices as JSON
  --ready-json                          Print a JSON ready event after bind and pane startup
  --one-shot                            Serve one attach client
  --live                                Serve one live client until detach
  --live-forever                        Serve sequential live clients until stopped
  --live-cycles COUNT                   Serve a bounded live client
  --live-clients COUNT                  Serve bounded sequential live clients
  --command SHELL                       Run a shell command in the pane PTY
  --cwd DIR                             Run the pane command from existing DIR
  --env KEY=VALUE                       Add an environment variable to the pane command
  --cols COUNT                          Initial pane PTY columns; both dimensions required
  --rows COUNT                          Initial pane PTY rows; both dimensions required
  --split horizontal|vertical           Start with pane-1 split into pane-1 and pane-2
  --resize-policy fixed|leader|active-client|manual
                                         Publish and enforce pane resize policy
  --terminal-engine interim|libghostty-vt
                                         Backend terminal engine implementation
  --version-json                         Show version as JSON
  -V, --version                         Show version
  -h, --help                            Show this help

Notes:
  Default socket: --socket, else valid absolute $NMUX_SOCKET, else valid absolute $XDG_RUNTIME_DIR/nmux/nmuxd.sock, else /tmp/nmux-$UID/nmuxd.sock.
  Informational flags exit before daemon-mode validation or socket/PTY work.
  --ready-json does not exit; it emits one stdout line after socket bind and pane startup.
  Existing socket paths are not replaced automatically.
  When started inside nmux, NMUX_ORIGIN is appended for child pane commands.
  libghostty-vt requires building nmux with the libghostty-vt feature.

Examples:
  nmuxd --one-shot --command \"printf 'ready\\n'; cat >/dev/null\"
  nmuxd --live --command \"printf 'ready\\n'; cat\"
  nmuxd --live-forever --command \"printf 'ready\\n'; cat\"
  nmuxd --live-clients 2 --command \"printf 'ready\\n'; cat\"
"
}

#[cfg(test)]
fn parse_resize_policy(value: &str) -> Result<protocol::ResizePolicy, &'static str> {
    match value {
        "fixed" => Ok(protocol::ResizePolicy::Fixed),
        "leader" => Ok(protocol::ResizePolicy::Leader),
        "active-client" => Ok(protocol::ResizePolicy::ActiveClient),
        "manual" => Ok(protocol::ResizePolicy::Manual),
        _ => Err("requires fixed, leader, active-client, or manual"),
    }
}

#[cfg(test)]
fn parse_terminal_engine_kind(value: &str) -> Result<TerminalEngineKind, &'static str> {
    match value {
        "interim" => Ok(TerminalEngineKind::InterimText),
        #[cfg(feature = "libghostty-vt")]
        "libghostty-vt" => Ok(TerminalEngineKind::LibghosttyVt),
        #[cfg(not(feature = "libghostty-vt"))]
        "libghostty-vt" => Err("libghostty-vt requires the libghostty-vt feature"),
        _ => Err("requires interim or libghostty-vt"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Args, DaemonModeArgs, SocketCleanup, args_from_iter, format_daemon_choices_json,
        format_ready_error_json, format_ready_json, parse_env_assignment, parse_numeric_arg,
        parse_resize_policy, parse_terminal_engine_kind, usage, validate_initial_size,
        validate_mode_args,
    };
    use nmux_cli::local;
    use nmux_core::terminal::TerminalEngineKind;
    use nmux_proto::protocol;
    use std::fs;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(0);

    fn test_socket_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("nmd-{}-{nanos:x}-{id}.sock", std::process::id()))
    }

    #[test]
    fn resize_policy_arg_accepts_documented_choices() {
        assert_eq!(
            parse_resize_policy("fixed"),
            Ok(protocol::ResizePolicy::Fixed)
        );
        assert_eq!(
            parse_resize_policy("leader"),
            Ok(protocol::ResizePolicy::Leader)
        );
        assert_eq!(
            parse_resize_policy("active-client"),
            Ok(protocol::ResizePolicy::ActiveClient)
        );
        assert_eq!(
            parse_resize_policy("manual"),
            Ok(protocol::ResizePolicy::Manual)
        );
        assert!(parse_resize_policy("max").is_err());
    }

    #[test]
    fn terminal_engine_arg_accepts_documented_choice() {
        assert_eq!(
            parse_terminal_engine_kind("interim"),
            Ok(TerminalEngineKind::InterimText)
        );
        #[cfg(feature = "libghostty-vt")]
        assert_eq!(
            parse_terminal_engine_kind("libghostty-vt"),
            Ok(TerminalEngineKind::LibghosttyVt)
        );
        #[cfg(not(feature = "libghostty-vt"))]
        assert!(parse_terminal_engine_kind("libghostty-vt").is_err());
    }

    #[test]
    fn daemon_choices_json_lists_config_vocabularies() {
        let json = format_daemon_choices_json();
        assert!(
            json.contains(
                "\"resize_policies\":[\"fixed\",\"leader\",\"active-client\",\"manual\"]"
            )
        );
        assert!(json.contains("\"split_axes\":[\"horizontal\",\"vertical\"]"));
        assert!(json.contains("{\"name\":\"interim\",\"available\":true}"));
        #[cfg(feature = "libghostty-vt")]
        assert!(json.contains("{\"name\":\"libghostty-vt\",\"available\":true}"));
        #[cfg(not(feature = "libghostty-vt"))]
        assert!(json.contains("{\"name\":\"libghostty-vt\",\"available\":false}"));
    }

    #[test]
    fn ready_json_reports_startup_context() {
        let args = Args {
            help: false,
            version: false,
            version_json: false,
            list_daemon_choices_json: false,
            print_socket: false,
            print_socket_json: false,
            ready_json: true,
            socket_path: PathBuf::from("/tmp/nmux-ready.sock"),
            socket_source: local::SocketPathSource::Explicit,
            one_shot: false,
            live: false,
            live_forever: true,
            live_cycles: None,
            live_clients: None,
            command: None,
            working_dir: None,
            env: Vec::new(),
            initial_size: None,
            initial_split: None,
            resize_policy: protocol::ResizePolicy::ActiveClient,
            terminal_engine_kind: TerminalEngineKind::InterimText,
        };

        assert_eq!(
            format_ready_json(&args),
            "{\"event\":\"ready\",\"NMUX_SOCKET\":\"/tmp/nmux-ready.sock\",\"source\":\"--socket\",\"mode\":\"live-forever\",\"terminal_engine\":\"interim\",\"resize_policy\":\"active-client\"}"
        );
    }

    #[test]
    fn ready_error_json_reports_startup_failure() {
        let error = std::io::Error::other("bind failed");

        assert_eq!(
            format_ready_error_json(&error),
            "{\"event\":\"error\",\"error\":{\"message\":\"bind failed\"}}"
        );
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
    fn args_accept_documented_launch_context() {
        let args = args_from_iter([
            "nmuxd",
            "--one-shot",
            "--socket",
            "/tmp/nmuxd-test.sock",
            "--command",
            "printf hi",
            "--cwd",
            "/tmp",
            "--env",
            "NMUX_TEST=one=two",
            "--cols",
            "100",
            "--rows",
            "30",
            "--split",
            "vertical",
            "--resize-policy",
            "active-client",
            "--terminal-engine",
            "interim",
        ])
        .expect("args");

        assert!(args.one_shot);
        assert_eq!(args.socket_path, PathBuf::from("/tmp/nmuxd-test.sock"));
        assert_eq!(args.socket_source, local::SocketPathSource::Explicit);
        assert_eq!(args.command.as_deref(), Some("printf hi"));
        assert_eq!(args.working_dir.as_deref(), Some("/tmp"));
        assert_eq!(
            args.env,
            vec![("NMUX_TEST".to_owned(), "one=two".to_owned())]
        );
        assert_eq!(args.initial_size, Some((100, 30)));
        assert_eq!(args.initial_split, Some(protocol::SplitAxis::Vertical));
        assert_eq!(args.resize_policy, protocol::ResizePolicy::ActiveClient);
        assert_eq!(args.terminal_engine_kind, TerminalEngineKind::InterimText);
    }

    #[test]
    fn usage_mentions_live_and_resize_policy_flags() {
        let usage = usage();
        assert!(usage.contains("--live"));
        assert!(usage.contains("--print-socket"));
        assert!(usage.contains("--print-socket-json"));
        assert!(usage.contains("--ready-json"));
        assert!(usage.contains("--list-daemon-choices-json"));
        assert!(usage.contains("--version-json"));
        assert!(usage.contains("-V, --version"));
        assert!(usage.contains("--live-forever"));
        assert!(usage.contains("--live-cycles COUNT"));
        assert!(usage.contains("--live-clients COUNT"));
        assert!(usage.contains("--cwd DIR"));
        assert!(usage.contains("--env KEY=VALUE"));
        assert!(usage.contains("--cols COUNT"));
        assert!(usage.contains("--rows COUNT"));
        assert!(usage.contains("--split horizontal|vertical"));
        assert!(usage.contains("--resize-policy fixed|leader|active-client|manual"));
        assert!(usage.contains("--terminal-engine interim|libghostty-vt"));
        assert!(usage.contains("libghostty-vt requires building nmux"));
        assert!(usage.contains("--ready-json does not exit"));
        assert!(usage.contains("Existing socket paths are not replaced automatically"));
    }

    #[test]
    fn mode_validation_rejects_ambiguous_daemon_modes() {
        assert_eq!(
            validate_mode_args(DaemonModeArgs {
                one_shot: true,
                live_clients: Some(2),
                ..DaemonModeArgs::default()
            }),
            Err("--one-shot cannot be combined with live daemon modes")
        );
        assert_eq!(
            validate_mode_args(DaemonModeArgs {
                live: true,
                live_forever: true,
                ..DaemonModeArgs::default()
            }),
            Err("--live cannot be combined with --live-forever, --live-cycles, or --live-clients")
        );
        assert_eq!(
            validate_mode_args(DaemonModeArgs {
                live: true,
                live_cycles: Some(1),
                ..DaemonModeArgs::default()
            }),
            Err("--live cannot be combined with --live-forever, --live-cycles, or --live-clients")
        );
        assert_eq!(
            validate_mode_args(DaemonModeArgs {
                live_forever: true,
                live_cycles: Some(1),
                ..DaemonModeArgs::default()
            }),
            Err("--live-forever cannot be combined with --live-cycles or --live-clients")
        );
        assert_eq!(
            validate_mode_args(DaemonModeArgs {
                live_cycles: Some(0),
                ..DaemonModeArgs::default()
            }),
            Err("--live-cycles must be greater than 0")
        );
        assert_eq!(
            validate_mode_args(DaemonModeArgs {
                live_clients: Some(0),
                ..DaemonModeArgs::default()
            }),
            Err("--live-clients must be greater than 0")
        );
        assert!(
            validate_mode_args(DaemonModeArgs {
                live_cycles: Some(2),
                live_clients: Some(3),
                ..DaemonModeArgs::default()
            })
            .is_ok()
        );
        assert!(
            validate_mode_args(DaemonModeArgs {
                live_forever: true,
                ..DaemonModeArgs::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn numeric_args_report_flag_names_on_parse_errors() {
        let err = parse_numeric_arg::<usize>("--live-cycles", "many".to_owned())
            .expect_err("invalid live cycle count should include flag name");
        assert!(err.contains("--live-cycles requires a valid number"));
    }

    #[test]
    fn initial_size_validation_requires_pair_and_valid_range() {
        let err = match args_from_iter(["nmuxd", "--one-shot", "--cols", "80"]) {
            Ok(_) => panic!("missing rows should fail"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("--cols and --rows must be provided together"));

        assert_eq!(validate_initial_size(Some((80, 24))), Ok(()));
        assert_eq!(
            validate_initial_size(Some((0, 24))),
            Err("--cols and --rows must be between 1 and 65535".to_owned())
        );
        assert_eq!(
            validate_initial_size(Some((80, 65536))),
            Err("--cols and --rows must be between 1 and 65535".to_owned())
        );
    }

    #[test]
    fn socket_cleanup_removes_original_socket() {
        let socket_path = test_socket_path();
        let listener = UnixListener::bind(&socket_path).expect("bind socket");
        let cleanup = SocketCleanup::new(socket_path.clone());

        drop(cleanup);
        drop(listener);

        assert!(
            !socket_path.exists(),
            "socket cleanup should remove original socket path"
        );
    }

    #[test]
    fn socket_cleanup_keeps_replaced_path() {
        let socket_path = test_socket_path();
        let listener = UnixListener::bind(&socket_path).expect("bind socket");
        let cleanup = SocketCleanup::new(socket_path.clone());

        fs::remove_file(&socket_path).expect("remove original socket");
        fs::write(&socket_path, "replacement").expect("write replacement");
        drop(cleanup);
        drop(listener);

        assert_eq!(
            fs::read_to_string(&socket_path).expect("read replacement"),
            "replacement"
        );
        let _ = fs::remove_file(socket_path);
    }
}
