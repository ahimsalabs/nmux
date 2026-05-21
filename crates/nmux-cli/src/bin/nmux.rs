use std::path::PathBuf;
use std::thread;
use std::time::Duration;

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

    let iterations = if args.follow {
        args.iterations.unwrap_or(usize::MAX)
    } else {
        1
    };

    for iteration in 0..iterations {
        let rendered = attach_once(&args, &mut client_state)?;
        if let Some(path) = args.state_path.as_deref() {
            client_state.save(path)?;
        }
        print_rendered(rendered);

        if args.follow && iteration + 1 < iterations {
            thread::sleep(Duration::from_millis(args.interval_ms));
        }
    }

    Ok(())
}

fn attach_once(
    args: &Args,
    client_state: &mut local::ClientAttachState,
) -> Result<local::RenderedAttach, Box<dyn std::error::Error>> {
    let mut options = local::AttachOptions {
        input_text: args.input_text.clone(),
        scrollback_start_line: args.scrollback_start_line,
        scrollback_line_count: args.scrollback_line_count,
        ..local::AttachOptions::default()
    };
    if args.follow || options.input_text.is_none() {
        options.request.mode = AttachMode::ReadOnly;
        options.input_text = None;
    }

    local::attach_render_once(&args.socket_path, options, client_state)
}

fn print_rendered(rendered: local::RenderedAttach) {
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
}

struct Args {
    socket_path: PathBuf,
    input_text: Option<String>,
    scrollback_start_line: u64,
    scrollback_line_count: u32,
    state_path: Option<PathBuf>,
    follow: bool,
    interval_ms: u64,
    iterations: Option<usize>,
}

fn args() -> Result<Args, Box<dyn std::error::Error>> {
    let mut socket_path = local::default_socket_path();
    let mut input_text = Some("a".to_owned());
    let mut scrollback_start_line = 1;
    let mut scrollback_line_count = 2;
    let mut state_path = None;
    let mut follow = false;
    let mut interval_ms = 1000;
    let mut iterations = None;
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
            "--follow" => {
                follow = true;
                input_text = None;
            }
            "--interval-ms" => {
                interval_ms = args
                    .next()
                    .ok_or("--interval-ms requires milliseconds")?
                    .parse()?;
            }
            "--iterations" => {
                iterations = Some(
                    args.next()
                        .ok_or("--iterations requires a count")?
                        .parse()?,
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
        follow,
        interval_ms,
        iterations,
    })
}
