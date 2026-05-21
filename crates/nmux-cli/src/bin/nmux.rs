use std::path::PathBuf;

use nmux_cli::local;
use nmux_core::session::AttachMode;

fn main() {
    if let Err(err) = run() {
        eprintln!("nmux: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = args()?;
    let mut client_state = match args.state_path.as_deref() {
        Some(path) => local::ClientAttachState::load(path)?,
        None => local::ClientAttachState::default(),
    };
    let mut options = local::AttachOptions {
        input_text: args.input_text,
        scrollback_start_line: args.scrollback_start_line,
        scrollback_line_count: args.scrollback_line_count,
        ..local::AttachOptions::default()
    };
    options.request.known_surfaces = client_state.known_surfaces();
    if options.input_text.is_none() {
        options.request.mode = AttachMode::ReadOnly;
    }
    let snapshot = local::attach_with_client_options(&args.socket_path, options)?;
    let rendered = client_state.render_attach(snapshot)?;
    if let Some(path) = args.state_path.as_deref() {
        client_state.save(path)?;
    }

    println!("{}", rendered.workspace.display_line());
    if let Some(surface_text) = rendered.surface_text {
        println!("{surface_text}");
    }
    if let Some(scrollback) = rendered.scrollback {
        println!(
            "scrollback {}..{}:",
            scrollback.start_line, scrollback.total_lines
        );
        for line in scrollback.lines {
            println!("{}", line.text);
        }
    }
    Ok(())
}

struct Args {
    socket_path: PathBuf,
    input_text: Option<String>,
    scrollback_start_line: u64,
    scrollback_line_count: u32,
    state_path: Option<PathBuf>,
}

fn args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut socket_path = local::default_socket_path();
    let mut input_text = Some("a".to_owned());
    let mut scrollback_start_line = 1;
    let mut scrollback_line_count = 2;
    let mut state_path = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => {
                socket_path = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or("--socket requires a path")?;
            }
            "--key" => {
                input_text = Some(args.next().ok_or("--key requires text")?);
            }
            "--no-input" => {
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
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }

    Ok(Args {
        socket_path,
        input_text,
        scrollback_start_line,
        scrollback_line_count,
        state_path,
    })
}
