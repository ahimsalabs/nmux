use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use nmux_cli::local;
use nmux_core::host::{CommandSpec, LocalPtyHost, ProcessHost};
use nmux_core::session::Session;

fn main() {
    if let Err(err) = run() {
        eprintln!("nmuxd: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = args()?;
    let listener = local::bind_listener(&args.socket_path)?;
    eprintln!("nmuxd: listening on {}", args.socket_path.display());

    let mut session = Session::initial();
    if let Some(command) = args.command {
        session.tabs[0].root.host.command = CommandSpec::new("sh").with_args(["-lc", &command]);
    }
    let pane_id = "pane-1";
    let host_spec = session.tabs[0].root.host.clone();
    let mut pty_host = LocalPtyHost::default();
    pty_host.start_pane(pane_id, &host_spec)?;
    wait_for_pane_output(&mut session, &mut pty_host, pane_id)?;

    if args.live || args.live_cycles.is_some() {
        let cycles = args.live_cycles.unwrap_or(usize::MAX);
        let serve_result =
            local::serve_live_one_with_host(&listener, &mut session, &mut pty_host, cycles);
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
    socket_path: PathBuf,
    one_shot: bool,
    live: bool,
    live_cycles: Option<usize>,
    command: Option<String>,
}

fn args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut socket_path = local::default_socket_path();
    let mut one_shot = false;
    let mut live = false;
    let mut live_cycles = None;
    let mut command = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
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
            "--command" => {
                command = Some(args.next().ok_or("--command requires a shell command")?);
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }

    Ok(Args {
        socket_path,
        one_shot,
        live,
        live_cycles,
        command,
    })
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
