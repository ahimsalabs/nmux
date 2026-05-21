use std::io::{self, BufRead, Read};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, TryRecvError};
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
    if args.live {
        return run_live(&args);
    }

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

fn run_live(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let mut client_state = match args.state_path.as_deref() {
        Some(path) => local::ClientAttachState::load(path)?,
        None => local::ClientAttachState::default(),
    };
    let mut stream = UnixStream::connect(&args.socket_path)?;
    stream.set_read_timeout(Some(Duration::from_millis(args.interval_ms)))?;
    let stdin = io::stdin();
    let mut stdin_lines = if args.stdin_input {
        Some(stdin.lock().lines())
    } else {
        None
    };
    let stdin_bytes = if args.stdin_bytes {
        Some(spawn_stdin_byte_reader())
    } else {
        None
    };
    let mut stdin_bytes_closed = false;

    let mut options = local::AttachOptions {
        input_text: args.input_text.clone(),
        scrollback_start_line: args.scrollback_start_line,
        scrollback_line_count: args.scrollback_line_count,
        ..local::AttachOptions::default()
    };
    if args.stdin_input || args.stdin_bytes {
        options.request.mode = AttachMode::ReadWrite;
    } else if options.input_text.is_none() {
        options.request.mode = AttachMode::ReadOnly;
    }

    options.request.known_surfaces = client_state.known_surfaces();
    local::write_attach_request(&mut stream, &options.request)?;
    let snapshot = local::attach_from_stream(&mut stream)?;
    let rendered = client_state.render_attach(snapshot)?;
    print_rendered(rendered);

    let cycle_limit = args.iterations.or_else(|| {
        (!args.stdin_input && !args.stdin_bytes && options.request.mode == AttachMode::ReadWrite)
            .then_some(1)
    });
    let mut cycles = 0;
    loop {
        if cycle_limit.is_some_and(|iterations| cycles >= iterations) {
            break;
        }

        if options.request.mode == AttachMode::ReadWrite {
            if let Some((cols, rows)) = args.live_resize {
                local::send_resize_intent(&mut stream, "pane-1", cols, rows)?;
            }
            let input_text = if let Some(receiver) = stdin_bytes.as_ref() {
                match receiver.try_recv() {
                    Ok(StdinByteRead::Input(input)) => Some(input),
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
                let stdin_line = next_stdin_line(stdin_lines.as_mut())?;
                match stdin_line {
                    Some(line) => Some(line),
                    None if args.iterations.is_none() => break,
                    None => None,
                }
            } else {
                options.input_text.as_deref().map(ToOwned::to_owned)
            };
            if let Some(input_text) = input_text.as_deref() {
                local::send_key_input(&mut stream, "pane-1", input_text)?;
            }
        }

        match local::read_live_surface_update_from_stream(&mut stream)? {
            local::LiveSurfaceRead::Update(update) => {
                println!("{}", client_state.render_surface_update(&update)?);
            }
            local::LiveSurfaceRead::NoFrame => {}
            local::LiveSurfaceRead::Closed => break,
        }
        if stdin_bytes_closed && args.iterations.is_none() {
            break;
        }
        cycles += 1;
    }

    if let Some(path) = args.state_path.as_deref() {
        client_state.save(path)?;
    }
    Ok(())
}

fn next_stdin_line(
    stdin_lines: Option<&mut io::Lines<io::StdinLock<'_>>>,
) -> io::Result<Option<String>> {
    let Some(stdin_lines) = stdin_lines else {
        return Ok(None);
    };
    let Some(line) = stdin_lines.next() else {
        return Ok(None);
    };
    let mut line = line?;
    line.push('\n');
    Ok(Some(line))
}

fn spawn_stdin_byte_reader() -> mpsc::Receiver<StdinByteRead> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buffer = [0_u8; 1024];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) => {
                    let _ = tx.send(StdinByteRead::Closed);
                    break;
                }
                Ok(count) => {
                    let input = String::from_utf8_lossy(&buffer[..count]).into_owned();
                    if tx.send(StdinByteRead::Input(input)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(StdinByteRead::Error(err.to_string()));
                    break;
                }
            }
        }
    });
    rx
}

enum StdinByteRead {
    Input(String),
    Closed,
    Error(String),
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
    live: bool,
    stdin_input: bool,
    stdin_bytes: bool,
    live_resize: Option<(u32, u32)>,
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
    let mut live = false;
    let mut stdin_input = false;
    let mut stdin_bytes = false;
    let mut live_cols = None;
    let mut live_rows = None;
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
            "--live" => {
                live = true;
            }
            "--stdin" => {
                stdin_input = true;
            }
            "--stdin-bytes" => {
                stdin_bytes = true;
            }
            "--cols" => {
                live_cols = Some(args.next().ok_or("--cols requires a count")?.parse()?);
            }
            "--rows" => {
                live_rows = Some(args.next().ok_or("--rows requires a count")?.parse()?);
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
    let live_resize = match (live_cols, live_rows) {
        (Some(cols), Some(rows)) => Some((cols, rows)),
        (None, None) => None,
        _ => return Err("--cols and --rows must be provided together".into()),
    };
    if stdin_input && stdin_bytes {
        return Err("--stdin and --stdin-bytes cannot be used together".into());
    }

    Ok(Args {
        socket_path,
        input_text,
        scrollback_start_line,
        scrollback_line_count,
        state_path,
        follow,
        live,
        stdin_input,
        stdin_bytes,
        live_resize,
        interval_ms,
        iterations,
    })
}
