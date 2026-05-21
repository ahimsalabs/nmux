use std::path::PathBuf;

use nmux_cli::local;
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
    if one_shot {
        local::serve_one(&listener, &mut session)?;
        return Ok(());
    }

    loop {
        local::serve_one(&listener, &mut session)?;
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
