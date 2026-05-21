use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use nmux_cli::local;
use nmux_core::host::{CommandSpec, LocalPtyHost, ProcessHost};
use nmux_core::session::Session;
use nmux_proto::protocol;

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

    let listener = local::bind_listener(&args.socket_path)?;
    eprintln!("nmuxd: listening on {}", args.socket_path.display());

    let mut session = Session::initial();
    if let Some(command) = args.command {
        session.tabs[0].root.host.command = CommandSpec::new("sh").with_args(["-lc", &command]);
    }
    session.set_pane_resize_policy("pane-1", args.resize_policy);
    let pane_id = "pane-1";
    let host_spec = session.tabs[0].root.host.clone();
    let mut pty_host = LocalPtyHost::default();
    pty_host.start_pane(pane_id, &host_spec)?;
    wait_for_pane_output(&mut session, &mut pty_host, pane_id)?;

    if args.live || args.live_cycles.is_some() || args.live_clients.is_some() {
        let cycles = args.live_cycles.unwrap_or(usize::MAX);
        let clients = args.live_clients.unwrap_or(1);
        let serve_result =
            local::serve_live_n_with_host(&listener, &mut session, &mut pty_host, clients, cycles);
        let stop_result = pty_host.stop_pane(pane_id);
        serve_result?;
        stop_result?;
        return Ok(());
    }

    if args.one_shot {
        let serve_result = local::serve_one_with_host(&listener, &mut session, &mut pty_host);
        let stop_result = pty_host.stop_pane(pane_id);
        serve_result?;
        stop_result?;
        return Ok(());
    }

    loop {
        local::serve_one_with_host(&listener, &mut session, &mut pty_host)?;
    }
}

struct Args {
    help: bool,
    socket_path: PathBuf,
    one_shot: bool,
    live: bool,
    live_cycles: Option<usize>,
    live_clients: Option<usize>,
    command: Option<String>,
    resize_policy: protocol::ResizePolicy,
}

fn args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut help = false;
    let mut socket_path = local::default_socket_path();
    let mut one_shot = false;
    let mut live = false;
    let mut live_cycles = None;
    let mut live_clients = None;
    let mut command = None;
    let mut resize_policy = protocol::ResizePolicy::Fixed;
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
            "--one-shot" => one_shot = true,
            "--live" => live = true,
            "--live-cycles" => {
                live_cycles = Some(
                    args.next()
                        .ok_or("--live-cycles requires a count")?
                        .parse()?,
                );
            }
            "--live-clients" => {
                live_clients = Some(
                    args.next()
                        .ok_or("--live-clients requires a count")?
                        .parse()?,
                );
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
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    validate_mode_args(one_shot, live, live_cycles, live_clients)?;

    Ok(Args {
        help,
        socket_path,
        one_shot,
        live,
        live_cycles,
        live_clients,
        command,
        resize_policy,
    })
}

fn validate_mode_args(
    one_shot: bool,
    live: bool,
    live_cycles: Option<usize>,
    live_clients: Option<usize>,
) -> Result<(), &'static str> {
    if one_shot && (live || live_cycles.is_some() || live_clients.is_some()) {
        return Err("--one-shot cannot be combined with live daemon modes");
    }
    if live && (live_cycles.is_some() || live_clients.is_some()) {
        return Err("--live cannot be combined with --live-cycles or --live-clients");
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
  --one-shot                            Serve one attach client
  --live                                Serve one live client until detach
  --live-cycles COUNT                   Serve a bounded live client
  --live-clients COUNT                  Serve bounded sequential live clients
  --command SHELL                       Run a shell command in the pane PTY
  --resize-policy fixed|leader|active-client|manual
                                         Publish and enforce pane resize policy
  -h, --help                            Show this help

Notes:
  Default socket: $XDG_RUNTIME_DIR/nmux/nmuxd.sock, else /tmp/nmux-$UID/nmuxd.sock.

Examples:
  nmuxd --one-shot --command \"printf 'ready\\n'; cat >/dev/null\"
  nmuxd --live --command \"printf 'ready\\n'; cat\"
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

#[cfg(test)]
mod tests {
    use super::{parse_resize_policy, usage, validate_mode_args};
    use nmux_proto::protocol;

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
    fn usage_mentions_live_and_resize_policy_flags() {
        let usage = usage();
        assert!(usage.contains("--live"));
        assert!(usage.contains("--live-cycles COUNT"));
        assert!(usage.contains("--live-clients COUNT"));
        assert!(usage.contains("--resize-policy fixed|leader|active-client|manual"));
    }

    #[test]
    fn mode_validation_rejects_ambiguous_daemon_modes() {
        assert_eq!(
            validate_mode_args(true, false, None, Some(2)),
            Err("--one-shot cannot be combined with live daemon modes")
        );
        assert_eq!(
            validate_mode_args(false, true, Some(1), None),
            Err("--live cannot be combined with --live-cycles or --live-clients")
        );
        assert_eq!(
            validate_mode_args(false, false, Some(0), None),
            Err("--live-cycles must be greater than 0")
        );
        assert_eq!(
            validate_mode_args(false, false, None, Some(0)),
            Err("--live-clients must be greater than 0")
        );
        assert!(validate_mode_args(false, false, Some(2), Some(3)).is_ok());
    }
}

fn wait_for_pane_output(
    session: &mut Session,
    output: &mut LocalPtyHost,
    pane_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        if local::poll_pane_output(session, output, pane_id)? {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
