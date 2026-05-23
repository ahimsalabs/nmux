use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::thread;
use std::time::{Duration, Instant};

use nmux_cli::local;
use nmux_core::host::{CommandSpec, LocalPtyHost, ProcessHost};
use nmux_core::session::Session;
use nmux_core::terminal::{PaneTerminalEngines, TerminalEngineKind};
use nmux_proto::protocol;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const RESIZE_POLICY_NAMES: &[&str] = &["fixed", "leader", "active-client", "manual"];
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

    let listener = local::bind_listener(&args.socket_path)?;
    let _socket_cleanup = SocketCleanup::new(args.socket_path.clone());
    eprintln!("nmuxd: listening on {}", args.socket_path.display());

    let mut session = Session::initial();
    if let Some(command) = args.command {
        session.tabs[0].root.host.command = CommandSpec::new("sh").with_args(["-lc", &command]);
    }
    session.set_pane_resize_policy("pane-1", args.resize_policy);
    let pane_id = "pane-1";
    let inherited_origin = inherited_nmux_origin();
    session.set_pane_nmux_environment(
        pane_id,
        args.socket_path.display().to_string(),
        inherited_origin.as_deref(),
    );
    let host_spec = session.tabs[0].root.host.clone();
    let mut pty_host = LocalPtyHost::default();
    pty_host.start_pane(pane_id, &host_spec)?;
    let mut terminal_engines = PaneTerminalEngines::new(args.terminal_engine_kind);
    wait_for_pane_output(&mut session, &mut pty_host, pane_id, &mut terminal_engines)?;

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
        let stop_result = pty_host.stop_pane(pane_id);
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
        let stop_result = pty_host.stop_pane(pane_id);
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

fn wait_for_pane_output(
    session: &mut Session,
    output: &mut LocalPtyHost,
    pane_id: &str,
    engines: &mut PaneTerminalEngines,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        if local::poll_pane_output_with_engines(session, engines, output, pane_id)? {
            break;
        }
        thread::sleep(Duration::from_millis(10));
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
    socket_path: PathBuf,
    socket_source: local::SocketPathSource,
    one_shot: bool,
    live: bool,
    live_forever: bool,
    live_cycles: Option<usize>,
    live_clients: Option<usize>,
    command: Option<String>,
    resize_policy: protocol::ResizePolicy,
    terminal_engine_kind: TerminalEngineKind,
}

fn args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut help = false;
    let mut version = false;
    let mut version_json = false;
    let mut list_daemon_choices_json = false;
    let mut print_socket = false;
    let mut print_socket_json = false;
    let (mut socket_path, mut socket_source) = local::default_socket_path_and_source();
    let mut one_shot = false;
    let mut live = false;
    let mut live_forever = false;
    let mut live_cycles = None;
    let mut live_clients = None;
    let mut command = None;
    let mut resize_policy = protocol::ResizePolicy::Fixed;
    let mut terminal_engine_kind = TerminalEngineKind::InterimText;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                help = true;
            }
            "--version" | "-V" => {
                version = true;
            }
            "--version-json" => {
                version_json = true;
            }
            "--list-daemon-choices-json" => {
                list_daemon_choices_json = true;
            }
            "--print-socket" => {
                print_socket = true;
            }
            "--print-socket-json" => {
                print_socket_json = true;
            }
            "--socket" => {
                socket_path = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--socket requires a path")?;
                socket_source = local::SocketPathSource::Explicit;
            }
            "--one-shot" => one_shot = true,
            "--live" => live = true,
            "--live-forever" => live_forever = true,
            "--live-cycles" => {
                live_cycles = Some(parse_numeric_arg(
                    "--live-cycles",
                    args.next().ok_or("--live-cycles requires a count")?,
                )?);
            }
            "--live-clients" => {
                live_clients = Some(parse_numeric_arg(
                    "--live-clients",
                    args.next().ok_or("--live-clients requires a count")?,
                )?);
            }
            "--command" => {
                command = Some(args.next().ok_or("--command requires a shell command")?);
            }
            "--resize-policy" => {
                resize_policy =
                    parse_resize_policy(&args.next().ok_or(
                        "--resize-policy requires fixed, leader, active-client, or manual",
                    )?)
                    .map_err(|err| format!("--resize-policy {err}"))?;
            }
            "--terminal-engine" => {
                terminal_engine_kind = parse_terminal_engine_kind(
                    &args
                        .next()
                        .ok_or("--terminal-engine requires interim or libghostty-vt")?,
                )
                .map_err(|err| format!("--terminal-engine {err}"))?;
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    if !(help
        || version
        || version_json
        || list_daemon_choices_json
        || print_socket
        || print_socket_json)
    {
        validate_mode_args(one_shot, live, live_forever, live_cycles, live_clients)?;
    }

    Ok(Args {
        help,
        version,
        version_json,
        list_daemon_choices_json,
        print_socket,
        print_socket_json,
        socket_path,
        socket_source,
        one_shot,
        live,
        live_forever,
        live_cycles,
        live_clients,
        command,
        resize_policy,
        terminal_engine_kind,
    })
}

fn format_daemon_choices_json() -> String {
    let resize_policies = format_json_string_array(RESIZE_POLICY_NAMES);
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
    format!("{{\"resize_policies\":{resize_policies},\"terminal_engines\":[{terminal_engines}]}}")
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

fn validate_mode_args(
    one_shot: bool,
    live: bool,
    live_forever: bool,
    live_cycles: Option<usize>,
    live_clients: Option<usize>,
) -> Result<(), &'static str> {
    if one_shot && (live || live_forever || live_cycles.is_some() || live_clients.is_some()) {
        return Err("--one-shot cannot be combined with live daemon modes");
    }
    if live && (live_forever || live_cycles.is_some() || live_clients.is_some()) {
        return Err(
            "--live cannot be combined with --live-forever, --live-cycles, or --live-clients",
        );
    }
    if live_forever && (live_cycles.is_some() || live_clients.is_some()) {
        return Err("--live-forever cannot be combined with --live-cycles or --live-clients");
    }
    if live_cycles == Some(0) {
        return Err("--live-cycles must be greater than 0");
    }
    if live_clients == Some(0) {
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
  --one-shot                            Serve one attach client
  --live                                Serve one live client until detach
  --live-forever                        Serve sequential live clients until stopped
  --live-cycles COUNT                   Serve a bounded live client
  --live-clients COUNT                  Serve bounded sequential live clients
  --command SHELL                       Run a shell command in the pane PTY
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

fn parse_resize_policy(value: &str) -> Result<protocol::ResizePolicy, &'static str> {
    match value {
        "fixed" => Ok(protocol::ResizePolicy::Fixed),
        "leader" => Ok(protocol::ResizePolicy::Leader),
        "active-client" => Ok(protocol::ResizePolicy::ActiveClient),
        "manual" => Ok(protocol::ResizePolicy::Manual),
        _ => Err("requires fixed, leader, active-client, or manual"),
    }
}

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
        SocketCleanup, format_daemon_choices_json, parse_numeric_arg, parse_resize_policy,
        parse_terminal_engine_kind, usage, validate_mode_args,
    };
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
        assert!(json.contains("{\"name\":\"interim\",\"available\":true}"));
        #[cfg(feature = "libghostty-vt")]
        assert!(json.contains("{\"name\":\"libghostty-vt\",\"available\":true}"));
        #[cfg(not(feature = "libghostty-vt"))]
        assert!(json.contains("{\"name\":\"libghostty-vt\",\"available\":false}"));
    }

    #[test]
    fn usage_mentions_live_and_resize_policy_flags() {
        let usage = usage();
        assert!(usage.contains("--live"));
        assert!(usage.contains("--print-socket"));
        assert!(usage.contains("--print-socket-json"));
        assert!(usage.contains("--list-daemon-choices-json"));
        assert!(usage.contains("--version-json"));
        assert!(usage.contains("-V, --version"));
        assert!(usage.contains("--live-forever"));
        assert!(usage.contains("--live-cycles COUNT"));
        assert!(usage.contains("--live-clients COUNT"));
        assert!(usage.contains("--resize-policy fixed|leader|active-client|manual"));
        assert!(usage.contains("--terminal-engine interim|libghostty-vt"));
        assert!(usage.contains("libghostty-vt requires building nmux"));
        assert!(usage.contains("Existing socket paths are not replaced automatically"));
    }

    #[test]
    fn mode_validation_rejects_ambiguous_daemon_modes() {
        assert_eq!(
            validate_mode_args(true, false, false, None, Some(2)),
            Err("--one-shot cannot be combined with live daemon modes")
        );
        assert_eq!(
            validate_mode_args(false, true, true, None, None),
            Err("--live cannot be combined with --live-forever, --live-cycles, or --live-clients")
        );
        assert_eq!(
            validate_mode_args(false, true, false, Some(1), None),
            Err("--live cannot be combined with --live-forever, --live-cycles, or --live-clients")
        );
        assert_eq!(
            validate_mode_args(false, false, true, Some(1), None),
            Err("--live-forever cannot be combined with --live-cycles or --live-clients")
        );
        assert_eq!(
            validate_mode_args(false, false, false, Some(0), None),
            Err("--live-cycles must be greater than 0")
        );
        assert_eq!(
            validate_mode_args(false, false, false, None, Some(0)),
            Err("--live-clients must be greater than 0")
        );
        assert!(validate_mode_args(false, false, false, Some(2), Some(3)).is_ok());
        assert!(validate_mode_args(false, false, true, None, None).is_ok());
    }

    #[test]
    fn numeric_args_report_flag_names_on_parse_errors() {
        let err = parse_numeric_arg::<usize>("--live-cycles", "many".to_owned())
            .expect_err("invalid live cycle count should include flag name");
        assert!(err.contains("--live-cycles requires a valid number"));
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
