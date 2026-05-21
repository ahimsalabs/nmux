use std::path::PathBuf;

use nmux_cli::local;

fn main() {
    if let Err(err) = run() {
        eprintln!("nmux: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = socket_arg()?;
    let snapshot = local::attach(&socket_path)?;
    println!("{}", snapshot.workspace.display_line());
    if let Some(surface) = snapshot.surface {
        println!("{surface}");
    }
    Ok(())
}

fn socket_arg() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None => Ok(local::default_socket_path()),
        Some("--socket") => args
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| "--socket requires a path".into()),
        Some(arg) => Err(format!("unknown argument: {arg}").into()),
    }
}
