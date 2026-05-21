use std::path::PathBuf;

use nmux_cli::local;
use nmux_core::host::{LocalPtyHost, ProcessHost};
use nmux_core::session::Session;

fn main() {
    if let Err(err) = run() {
        eprintln!("nmuxd: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let (socket_path, one_shot) = args()?;
    let listener = local::bind_listener(&socket_path)?;
    eprintln!("nmuxd: listening on {}", socket_path.display());

    let mut session = Session::initial();
    let pane_id = "pane-1";
    let host_spec = session.tabs[0].root.host.clone();
    let mut pty_host = LocalPtyHost::default();
    pty_host.start_pane(pane_id, &host_spec)?;

    if one_shot {
        let serve_result = local::serve_one_with_output(&listener, &mut session, &mut pty_host);
        let stop_result = pty_host.stop_pane(pane_id);
        serve_result?;
        stop_result?;
        return Ok(());
    }

    loop {
        local::serve_one_with_output(&listener, &mut session, &mut pty_host)?;
    }
}

fn args() -> Result<(PathBuf, bool), Box<dyn std::error::Error>> {
    let mut socket_path = local::default_socket_path();
    let mut one_shot = false;
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
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }

    Ok((socket_path, one_shot))
}
