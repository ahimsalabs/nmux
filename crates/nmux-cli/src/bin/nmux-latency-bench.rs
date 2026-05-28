use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nmux_cli::{local, observability};
use nmux_core::session::AttachMode;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tracing::instrument;

const DEFAULT_ITERATIONS: usize = 50;
const DEFAULT_WARMUP: usize = 5;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);
const SAMPLE_COOLDOWN: Duration = Duration::from_millis(10);
const STDIN_BYTES_DETACH: u8 = 0x1d;
const AGED_HISTORY_SETUP_TIMEOUT: Duration = Duration::from_secs(20);
const AGED_HISTORY_CYCLES: usize = 4;
const AGED_HISTORY_LINES_PER_CYCLE: usize = 700;
const AGED_HISTORY_ALT_FRAMES_PER_CYCLE: usize = 20;
const REPEATED_OUTPUT_LINES_PER_SAMPLE: usize = 240;

fn main() {
    if let Err(err) = run() {
        eprintln!("nmux-latency-bench: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--echo-helper")) {
        return run_echo_helper();
    }
    let args = Args::parse(std::env::args_os().skip(1))?;
    if args.help {
        print!("{}", usage());
        return Ok(());
    }
    let workspace = BenchWorkspace::new(args.socket_path.clone(), args.trace_path.clone())?;
    if let Some(trace_path) = workspace.trace_path.as_ref() {
        let _ = fs::remove_file(trace_path);
        observability::init_with_file(trace_path)?;
    } else {
        observability::init_from_env()?;
    }
    let nmux = nmux_binary_path()?;
    let report = run_latency_suite(
        &nmux,
        &workspace.socket_path,
        workspace.trace_path.as_deref(),
        args.iterations,
        args.warmup,
    )?;
    if args.json {
        println!("{}", report.to_json(workspace.trace_path.as_deref()));
    } else {
        println!("{}", report.display(workspace.trace_path.as_deref()));
    }
    Ok(())
}

#[instrument(level = "info", skip(nmux, socket_path, trace_path))]
fn run_latency_suite(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
    iterations: usize,
    warmup: usize,
) -> Result<LatencySuiteReport, Box<dyn std::error::Error>> {
    let cases = [
        BenchCase {
            name: "key-input-echo",
            pane_id: "pane-1",
            input_kind: InputKind::Key,
        },
        BenchCase {
            name: "raw-input-echo",
            pane_id: "pane-1",
            input_kind: InputKind::Raw,
        },
        BenchCase {
            name: "paste-input-echo",
            pane_id: "pane-1",
            input_kind: InputKind::Paste,
        },
    ];
    let mut reports = Vec::with_capacity(cases.len());
    for case in cases {
        let case_socket_path = case_socket_path(socket_path, case.name);
        let report = run_latency_case(
            nmux,
            &case_socket_path,
            trace_path,
            case,
            iterations,
            warmup,
        )?;
        reports.push(report);
    }
    let interactive_socket_path = case_socket_path(socket_path, "interactive-redraw-stdin-bytes");
    reports.push(run_interactive_redraw_case(
        nmux,
        &interactive_socket_path,
        trace_path,
        iterations,
        warmup,
    )?);
    let aged_socket_path = case_socket_path(socket_path, "interactive-redraw-aged-history");
    reports.push(run_interactive_aged_history_case(
        nmux,
        &aged_socket_path,
        trace_path,
        iterations,
        warmup,
    )?);
    let repeated_socket_path =
        case_socket_path(socket_path, "interactive-redraw-after-repeated-output");
    reports.push(run_interactive_repeated_output_case(
        nmux,
        &repeated_socket_path,
        trace_path,
        iterations,
        warmup,
    )?);
    let speculative_socket_path = case_socket_path(
        socket_path,
        "interactive-redraw-speculative-after-repeated-output",
    );
    reports.push(run_interactive_speculative_repeated_output_case(
        nmux,
        &speculative_socket_path,
        trace_path,
        iterations,
        warmup,
    )?);
    Ok(LatencySuiteReport { cases: reports })
}

#[instrument(level = "info", skip(nmux, socket_path, trace_path))]
fn run_latency_case(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
    case: BenchCase,
    iterations: usize,
    warmup: usize,
) -> Result<LatencyCaseReport, Box<dyn std::error::Error>> {
    let _ = fs::remove_file(socket_path);
    let mut daemon = start_daemon(nmux, socket_path, trace_path, None)?;
    let result = run_single_client_case(socket_path, case, iterations, warmup);
    let _ = daemon.kill();
    let _ = daemon.wait();
    let _ = fs::remove_file(socket_path);
    result
}

fn run_single_client_case(
    socket_path: &Path,
    case: BenchCase,
    iterations: usize,
    warmup: usize,
) -> Result<LatencyCaseReport, Box<dyn std::error::Error>> {
    let mut stream = local::connect_to_daemon_with_timeout(socket_path, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(DEFAULT_TIMEOUT))?;
    let pane_id = attach_live_stream(&mut stream, AttachMode::ReadWrite, case.pane_id)?;
    let mut sequence = local::ClientFrameSequence::default();
    let mut samples = Vec::with_capacity(iterations);

    for index in 0..(warmup + iterations) {
        let token = format!("{}-{}-{}", case.name, std::process::id(), index);
        let elapsed = measure_echo_latency(
            &mut stream,
            &mut sequence,
            &pane_id,
            &token,
            case.input_kind,
        )?;
        if index >= warmup {
            samples.push(elapsed);
        }
        std::thread::sleep(SAMPLE_COOLDOWN);
    }

    Ok(LatencyCaseReport::from_samples(case.name, samples))
}

fn run_interactive_redraw_case(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
    iterations: usize,
    warmup: usize,
) -> Result<LatencyCaseReport, Box<dyn std::error::Error>> {
    let _ = fs::remove_file(socket_path);
    let mut daemon = start_daemon(nmux, socket_path, trace_path, None)?;
    let result = run_interactive_redraw_client(nmux, socket_path, trace_path, iterations, warmup);
    let _ = daemon.kill();
    let _ = daemon.wait();
    let _ = fs::remove_file(socket_path);
    result
}

fn run_interactive_redraw_client(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
    iterations: usize,
    warmup: usize,
) -> Result<LatencyCaseReport, Box<dyn std::error::Error>> {
    let mut client = spawn_interactive_nmux_client(nmux, socket_path, trace_path)?;
    client.wait_for_output("pane-1", DEFAULT_TIMEOUT)?;
    let mut samples = Vec::with_capacity(iterations);

    for index in 0..(warmup + iterations) {
        let token = format!(
            "interactive-redraw-stdin-bytes-{}-{index}",
            std::process::id()
        );
        let input = format!("{token}\n");
        let sample_span = tracing::trace_span!("interactive_redraw_sample", token = %token, index);
        let elapsed = sample_span.in_scope(|| {
            let start = Instant::now();
            client.write_input(input.as_bytes())?;
            client.wait_for_output(&token, DEFAULT_TIMEOUT)?;
            Ok::<Duration, Box<dyn std::error::Error>>(start.elapsed())
        })?;
        if index >= warmup {
            samples.push(elapsed);
        }
        std::thread::sleep(SAMPLE_COOLDOWN);
    }

    client.detach();
    let _ = client.wait();
    Ok(LatencyCaseReport::from_samples(
        "interactive-redraw-stdin-bytes",
        samples,
    ))
}

fn run_interactive_aged_history_case(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
    iterations: usize,
    warmup: usize,
) -> Result<LatencyCaseReport, Box<dyn std::error::Error>> {
    let _ = fs::remove_file(socket_path);
    let mut daemon = start_daemon(nmux, socket_path, trace_path, None)?;
    let result =
        run_interactive_aged_history_client(nmux, socket_path, trace_path, iterations, warmup);
    let _ = daemon.kill();
    let _ = daemon.wait();
    let _ = fs::remove_file(socket_path);
    result
}

fn run_interactive_aged_history_client(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
    iterations: usize,
    warmup: usize,
) -> Result<LatencyCaseReport, Box<dyn std::error::Error>> {
    let mut client = spawn_interactive_nmux_client(nmux, socket_path, trace_path)?;
    client.wait_for_output("pane-1", DEFAULT_TIMEOUT)?;

    for cycle in 0..AGED_HISTORY_CYCLES {
        let fill_marker = format!("aged-history-fill-{}-{cycle}", std::process::id());
        client.write_input(
            format!("__nmux_bench_fill:{fill_marker}:{AGED_HISTORY_LINES_PER_CYCLE}\n").as_bytes(),
        )?;
        client.wait_for_output(&fill_marker, AGED_HISTORY_SETUP_TIMEOUT)?;

        let clear_marker = format!("aged-history-clear-{}-{cycle}", std::process::id());
        client.write_input(format!("__nmux_bench_clear:{clear_marker}\n").as_bytes())?;
        client.wait_for_output(&clear_marker, AGED_HISTORY_SETUP_TIMEOUT)?;

        let alt_marker = format!("aged-history-alt-{}-{cycle}", std::process::id());
        client.write_input(
            format!("__nmux_bench_alt:{alt_marker}:{AGED_HISTORY_ALT_FRAMES_PER_CYCLE}\n")
                .as_bytes(),
        )?;
        client.wait_for_output(&alt_marker, AGED_HISTORY_SETUP_TIMEOUT)?;
    }

    let mut samples = Vec::with_capacity(iterations);
    for index in 0..(warmup + iterations) {
        let token = format!(
            "interactive-redraw-aged-history-{}-{index}",
            std::process::id()
        );
        let input = format!("{token}\n");
        let sample_span =
            tracing::trace_span!("interactive_aged_history_sample", token = %token, index);
        let elapsed = sample_span.in_scope(|| {
            let start = Instant::now();
            client.write_input(input.as_bytes())?;
            client.wait_for_output(&token, DEFAULT_TIMEOUT)?;
            Ok::<Duration, Box<dyn std::error::Error>>(start.elapsed())
        })?;
        if index >= warmup {
            samples.push(elapsed);
        }
        std::thread::sleep(SAMPLE_COOLDOWN);
    }

    client.detach();
    let _ = client.wait();
    Ok(LatencyCaseReport::from_samples(
        "interactive-redraw-aged-history",
        samples,
    ))
}

fn run_interactive_repeated_output_case(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
    iterations: usize,
    warmup: usize,
) -> Result<LatencyCaseReport, Box<dyn std::error::Error>> {
    let _ = fs::remove_file(socket_path);
    let mut daemon = start_daemon(nmux, socket_path, trace_path, None)?;
    let result =
        run_interactive_repeated_output_client(nmux, socket_path, trace_path, iterations, warmup);
    let _ = daemon.kill();
    let _ = daemon.wait();
    let _ = fs::remove_file(socket_path);
    result
}

fn run_interactive_repeated_output_client(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
    iterations: usize,
    warmup: usize,
) -> Result<LatencyCaseReport, Box<dyn std::error::Error>> {
    let mut client = spawn_interactive_nmux_client(nmux, socket_path, trace_path)?;
    client.wait_for_output("pane-1", DEFAULT_TIMEOUT)?;
    let mut samples = Vec::with_capacity(iterations);

    for index in 0..(warmup + iterations) {
        let token = format!(
            "interactive-redraw-after-repeated-output-{}-{index}",
            std::process::id()
        );
        let burst_marker = format!(
            "interactive-redraw-repeated-output-burst-{}-{index}",
            std::process::id()
        );
        let burst = format!(
            "__nmux_bench_repeated_output:{burst_marker}:{REPEATED_OUTPUT_LINES_PER_SAMPLE}\n"
        );
        let input = format!("{token}\n");
        let sample_span =
            tracing::trace_span!("interactive_repeated_output_sample", token = %token, index);
        let elapsed = sample_span.in_scope(|| {
            let start = Instant::now();
            client.write_input(burst.as_bytes())?;
            client.write_input(input.as_bytes())?;
            client.wait_for_output(&token, DEFAULT_TIMEOUT)?;
            Ok::<Duration, Box<dyn std::error::Error>>(start.elapsed())
        })?;
        if index >= warmup {
            samples.push(elapsed);
        }
        std::thread::sleep(SAMPLE_COOLDOWN);
    }

    client.detach();
    let _ = client.wait();
    Ok(LatencyCaseReport::from_samples(
        "interactive-redraw-after-repeated-output",
        samples,
    ))
}

fn run_interactive_speculative_repeated_output_case(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
    iterations: usize,
    warmup: usize,
) -> Result<LatencyCaseReport, Box<dyn std::error::Error>> {
    let _ = fs::remove_file(socket_path);
    let mut daemon = start_daemon(nmux, socket_path, trace_path, None)?;
    let result = run_interactive_speculative_repeated_output_client(
        nmux,
        socket_path,
        trace_path,
        iterations,
        warmup,
    );
    let _ = daemon.kill();
    let _ = daemon.wait();
    let _ = fs::remove_file(socket_path);
    result
}

fn run_interactive_speculative_repeated_output_client(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
    iterations: usize,
    warmup: usize,
) -> Result<LatencyCaseReport, Box<dyn std::error::Error>> {
    let mut client = spawn_interactive_nmux_client(nmux, socket_path, trace_path)?;
    client.wait_for_output("pane-1", DEFAULT_TIMEOUT)?;
    let mut samples = Vec::with_capacity(iterations);

    for index in 0..(warmup + iterations) {
        let anchor = format!(
            "interactive-redraw-speculative-anchor-{}-{index}",
            std::process::id()
        );
        let expected = "\x1b[4mx";
        let burst_marker = format!(
            "interactive-redraw-speculative-burst-{}-{index}",
            std::process::id()
        );
        client.write_input(format!("{anchor}\n").as_bytes())?;
        client.wait_for_output(&anchor, DEFAULT_TIMEOUT)?;

        let burst = format!(
            "__nmux_bench_repeated_output:{burst_marker}:{REPEATED_OUTPUT_LINES_PER_SAMPLE}\n"
        );
        client.write_input(burst.as_bytes())?;
        std::thread::sleep(Duration::from_millis(1));
        client.clear_output();

        let sample_span = tracing::trace_span!(
            "interactive_speculative_repeated_output_sample",
            anchor = %anchor,
            index
        );
        let elapsed = sample_span.in_scope(|| {
            let start = Instant::now();
            client.write_input(b"x")?;
            client.wait_for_output(&expected, DEFAULT_TIMEOUT)?;
            Ok::<Duration, Box<dyn std::error::Error>>(start.elapsed())
        })?;
        if index >= warmup {
            samples.push(elapsed);
        }
        client.write_input(b"\n")?;
        std::thread::sleep(SAMPLE_COOLDOWN);
    }

    client.detach();
    let _ = client.wait();
    Ok(LatencyCaseReport::from_samples(
        "interactive-redraw-speculative-after-repeated-output",
        samples,
    ))
}

struct InteractiveNmuxClient {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output_rx: mpsc::Receiver<Vec<u8>>,
    reader_thread: thread::JoinHandle<()>,
    output: Vec<u8>,
}

fn spawn_interactive_nmux_client(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
) -> Result<InteractiveNmuxClient, Box<dyn std::error::Error>> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 100,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut command = CommandBuilder::new(nmux);
    command.args([
        "--socket",
        socket_path
            .to_str()
            .ok_or("socket path is not valid UTF-8")?,
        "--live",
        "--stdin-bytes",
        "--redraw",
        "--no-scrollback",
        "--interval-ms",
        "16",
        "--connect-timeout-ms",
        "5000",
    ]);
    command.env("TERM", "xterm-256color");
    if let Some(trace_path) = trace_path {
        command.env("NMUX_TRACE_FILE", trace_path);
        command.env(
            "NMUX_TRACE",
            "info,nmux_cli=trace,nmux_core=trace,nmux=trace",
        );
    }
    let child = pair.slave.spawn_command(command)?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let (output_tx, output_rx) = mpsc::channel();
    let reader_thread = thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if output_tx.send(buffer[..count].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    Ok(InteractiveNmuxClient {
        child,
        writer,
        output_rx,
        reader_thread,
        output: Vec::new(),
    })
}

impl InteractiveNmuxClient {
    fn write_input(&mut self, input: &[u8]) -> io::Result<()> {
        self.writer.write_all(input)?;
        self.writer.flush()
    }

    fn clear_output(&mut self) {
        self.output.clear();
        while let Ok(chunk) = self.output_rx.try_recv() {
            drop(chunk);
        }
    }

    fn wait_for_output(
        &mut self,
        needle: &str,
        timeout: Duration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if String::from_utf8_lossy(&self.output).contains(needle) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self
                .output_rx
                .recv_timeout(remaining.min(Duration::from_millis(50)))
            {
                Ok(chunk) => self.output.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        Err(format!("timed out waiting for interactive output {needle:?}").into())
    }

    fn detach(&mut self) {
        let _ = self.write_input(&[STDIN_BYTES_DETACH]);
    }

    fn wait(mut self) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.child.wait()?;
        drop(self.output_rx);
        let _ = self.reader_thread.join();
        Ok(())
    }
}

fn attach_live_stream(
    stream: &mut UnixStream,
    mode: AttachMode,
    pane_id: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let request = local::AttachRequest {
        actor_id: format!("latency-bench-{pane_id}-{mode:?}"),
        user_id: "latency-bench".to_owned(),
        display_name: "latency bench".to_owned(),
        hostname: "latency-bench".to_owned(),
        client_kind: "nmux-latency-bench".to_owned(),
        mode,
        focused_pane_id: Some(pane_id.to_owned()),
        known_surfaces: Vec::new(),
        known_viewports: Vec::new(),
        subscribe_client_inventory: false,
    };
    local::write_attach_request(stream, &request)?;
    let snapshot = local::attach_from_stream(stream)?;
    Ok(snapshot.status.pane_id)
}

#[instrument(level = "trace", skip(stream, sequence, token), fields(pane_id = %pane_id, token = %token))]
fn measure_echo_latency(
    stream: &mut UnixStream,
    sequence: &mut local::ClientFrameSequence,
    pane_id: &str,
    token: &str,
    input_kind: InputKind,
) -> Result<Duration, Box<dyn std::error::Error>> {
    let input = format!("{token}\n");
    let expected = token.to_owned();
    let start = Instant::now();
    let deadline = start + DEFAULT_TIMEOUT;
    send_input(stream, sequence, pane_id, &input, input_kind)?;
    loop {
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for {expected}").into());
        }
        let read_span = tracing::trace_span!(
            "client.read_live_surface_update",
            pane_id = %pane_id,
            token = %token
        );
        match read_span.in_scope(|| local::read_live_surface_update_from_stream(stream))? {
            local::LiveSurfaceRead::Update(update) => {
                if update.text.contains(&expected)
                    || update
                        .row_updates
                        .iter()
                        .any(|row| row.text.contains(&expected))
                {
                    return Ok(start.elapsed());
                }
            }
            local::LiveSurfaceRead::Error(error) => {
                return Err(format!("server error while waiting for {expected}: {error}").into());
            }
            local::LiveSurfaceRead::Closed => {
                return Err(format!("server closed while waiting for {expected}").into());
            }
            local::LiveSurfaceRead::Workspace(_)
            | local::LiveSurfaceRead::Presence(_)
            | local::LiveSurfaceRead::ClientInventorySnapshot(_)
            | local::LiveSurfaceRead::ClientInventoryPatch(_)
            | local::LiveSurfaceRead::Pong(_)
            | local::LiveSurfaceRead::NoFrame => {}
        }
    }
}

fn start_daemon(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
    concurrent_cycles: Option<usize>,
) -> Result<Child, Box<dyn std::error::Error>> {
    let helper = std::env::current_exe()?;
    let command = format!("{} --echo-helper", shell_quote(&helper));
    let mut args = vec![
        "daemon".to_owned(),
        "--socket".to_owned(),
        socket_path
            .to_str()
            .ok_or("socket path is not UTF-8")?
            .to_owned(),
        "--ready-json".to_owned(),
        "--command".to_owned(),
        command,
    ];
    if let Some(cycles) = concurrent_cycles {
        args.extend([
            "--live-clients".to_owned(),
            "2".to_owned(),
            "--live-cycles".to_owned(),
            cycles.to_string(),
        ]);
    } else {
        args.push("--live-forever".to_owned());
    }
    let mut child = Command::new(nmux)
        .args(args)
        .envs(trace_path.map(|path| ("NMUX_TRACE_FILE", path.as_os_str())))
        .envs(trace_path.map(|_| {
            (
                "NMUX_TRACE",
                "info,nmux_cli=trace,nmux_core=trace,nmux=trace",
            )
        }))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("failed to spawn nmux daemon: {err}"))?;
    wait_for_ready(&mut child)?;
    Ok(child)
}

fn run_echo_helper() -> Result<(), Box<dyn std::error::Error>> {
    configure_raw_echo_stdin()?;
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut buffer = [0_u8; 4096];
    let mut line = Vec::new();
    loop {
        let count = stdin.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        for byte in &buffer[..count] {
            if *byte == b'\n' {
                while line.last() == Some(&b'\r') {
                    line.pop();
                }
                write_echo_helper_line(&mut stdout, &line)?;
                stdout.flush()?;
                line.clear();
            } else {
                line.push(*byte);
            }
        }
    }
}

fn write_echo_helper_line<W: Write>(writer: &mut W, line: &[u8]) -> io::Result<()> {
    let text = String::from_utf8_lossy(line);
    if let Some(rest) = text.strip_prefix("__nmux_bench_fill:") {
        let Some((marker, count)) = rest.rsplit_once(':') else {
            return Ok(());
        };
        let count = count.parse::<usize>().unwrap_or(0);
        for index in 0..count {
            writeln!(writer, "aged history line {index:04} {marker}")?;
        }
        writeln!(writer, "{marker}")?;
    } else if let Some(marker) = text.strip_prefix("__nmux_bench_clear:") {
        write!(writer, "\x1b[2J\x1b[H{marker}\r\n")?;
    } else if let Some(rest) = text.strip_prefix("__nmux_bench_alt:") {
        let Some((marker, frames)) = rest.rsplit_once(':') else {
            return Ok(());
        };
        let frames = frames.parse::<usize>().unwrap_or(0);
        write!(writer, "\x1b[?1049h")?;
        for frame in 0..frames {
            write!(
                writer,
                "\x1b[Halternate frame {frame:03} {marker}\r\n{}",
                "x".repeat(80)
            )?;
        }
        write!(writer, "\x1b[?1049l{marker}\r\n")?;
    } else if let Some(rest) = text.strip_prefix("__nmux_bench_repeated_output:") {
        let Some((marker, count)) = rest.rsplit_once(':') else {
            return Ok(());
        };
        let count = count.parse::<usize>().unwrap_or(0);
        for index in 0..count {
            writeln!(writer, "repeated output line {index:04} {marker}")?;
        }
    } else {
        writer.write_all(b"\r")?;
        writer.write_all(line)?;
    }
    Ok(())
}

fn configure_raw_echo_stdin() -> io::Result<()> {
    let fd = 0;
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: fd 0 (stdin) is open; termios is a valid MaybeUninit pointer.
    if unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: tcgetattr succeeded, so termios is fully initialized.
    let mut termios = unsafe { termios.assume_init() };
    termios.c_lflag &= !(libc::ECHO | libc::ICANON);
    termios.c_cc[libc::VMIN] = 1;
    termios.c_cc[libc::VTIME] = 0;
    // SAFETY: fd 0 is open and termios is a valid, initialized struct.
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn shell_quote(path: &Path) -> String {
    let value = path.as_os_str().to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn wait_for_ready(child: &mut Child) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = child
        .stdout
        .as_mut()
        .ok_or("daemon stdout was not captured")?;
    let mut line = String::new();
    let mut reader = io::BufReader::new(stdout);
    use std::io::BufRead;
    reader.read_line(&mut line)?;
    if line.contains("\"event\":\"ready\"") {
        return Ok(());
    }
    Err(format!("daemon did not report ready: {}", line.trim()).into())
}

#[derive(Debug, Clone, Copy)]
struct BenchCase {
    name: &'static str,
    pane_id: &'static str,
    input_kind: InputKind,
}

#[derive(Debug, Clone, Copy)]
enum InputKind {
    Key,
    Raw,
    Paste,
}

fn send_input(
    stream: &mut UnixStream,
    sequence: &mut local::ClientFrameSequence,
    pane_id: &str,
    input: &str,
    kind: InputKind,
) -> Result<(), Box<dyn std::error::Error>> {
    match kind {
        InputKind::Key => {
            local::send_key_input_with_sequence(stream, sequence, pane_id, input).map(|_| ())
        }
        InputKind::Raw => {
            local::send_raw_input_with_sequence(stream, sequence, pane_id, input.as_bytes())
                .map(|_| ())
        }
        InputKind::Paste => local::send_paste_input_with_sequence(stream, sequence, pane_id, input),
    }
}

#[derive(Debug)]
struct LatencySuiteReport {
    cases: Vec<LatencyCaseReport>,
}

impl LatencySuiteReport {
    fn display(&self, trace_path: Option<&Path>) -> String {
        let mut output = String::from("input-to-display latency suite");
        for case in &self.cases {
            output.push('\n');
            output.push_str(&case.display());
        }
        if let Some(trace_path) = trace_path {
            output.push_str(&format!("\ntrace={}", trace_path.display()));
        }
        output
    }

    fn to_json(&self, trace_path: Option<&Path>) -> String {
        let cases = self
            .cases
            .iter()
            .map(LatencyCaseReport::to_json)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"cases\":[{cases}],\"trace_path\":{}}}",
            trace_path
                .map(|path| nmux_cli::json::json_string(&path.display().to_string()))
                .unwrap_or_else(|| "null".to_owned())
        )
    }
}

#[derive(Debug)]
struct LatencyCaseReport {
    name: &'static str,
    samples: Vec<Duration>,
    min: Duration,
    p50: Duration,
    p90: Duration,
    p99: Duration,
    max: Duration,
    mean: Duration,
}

impl LatencyCaseReport {
    fn from_samples(name: &'static str, mut samples: Vec<Duration>) -> Self {
        samples.sort_unstable();
        let total_nanos: u128 = samples.iter().map(Duration::as_nanos).sum();
        let mean = Duration::from_nanos((total_nanos / samples.len() as u128) as u64);
        Self {
            name,
            min: samples[0],
            p50: percentile(&samples, 50),
            p90: percentile(&samples, 90),
            p99: percentile(&samples, 99),
            max: *samples.last().expect("samples are non-empty"),
            mean,
            samples,
        }
    }

    fn display(&self) -> String {
        format!(
            "{}: n={} min={} mean={} p50={} p90={} p99={} max={}",
            self.name,
            self.samples.len(),
            format_duration(self.min),
            format_duration(self.mean),
            format_duration(self.p50),
            format_duration(self.p90),
            format_duration(self.p99),
            format_duration(self.max)
        )
    }

    fn to_json(&self) -> String {
        format!(
            "{{\"name\":{},\"samples\":{},\"min_us\":{},\"mean_us\":{},\"p50_us\":{},\"p90_us\":{},\"p99_us\":{},\"max_us\":{}}}",
            nmux_cli::json::json_string(self.name),
            self.samples.len(),
            self.min.as_micros(),
            self.mean.as_micros(),
            self.p50.as_micros(),
            self.p90.as_micros(),
            self.p99.as_micros(),
            self.max.as_micros()
        )
    }
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    let index = ((samples.len() - 1) * percentile).div_ceil(100);
    samples[index]
}

fn case_socket_path(base: &Path, case_name: &str) -> PathBuf {
    let stem = base
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("nmux");
    let extension = base.extension().and_then(|extension| extension.to_str());
    let filename = match extension {
        Some(extension) => format!("{stem}-{case_name}.{extension}"),
        None => format!("{stem}-{case_name}"),
    };
    base.with_file_name(filename)
}

fn format_duration(duration: Duration) -> String {
    if duration.as_millis() > 0 {
        format!("{:.3}ms", duration.as_secs_f64() * 1000.0)
    } else {
        format!("{}us", duration.as_micros())
    }
}

struct BenchWorkspace {
    root: Option<PathBuf>,
    socket_path: PathBuf,
    trace_path: Option<PathBuf>,
}

impl BenchWorkspace {
    fn new(
        socket_path: Option<PathBuf>,
        trace_path: Option<PathBuf>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if let Some(socket_path) = socket_path {
            let _ = fs::remove_file(&socket_path);
            return Ok(Self {
                root: None,
                socket_path,
                trace_path,
            });
        }
        let root = PathBuf::from("/tmp").join(format!(
            "nmux-lat-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;
        Ok(Self {
            socket_path: root.join("nmux.sock"),
            trace_path: trace_path.or_else(default_trace_path),
            root: Some(root),
        })
    }
}

impl Drop for BenchWorkspace {
    fn drop(&mut self) {
        if let Some(root) = self.root.as_ref() {
            let _ = fs::remove_dir_all(root);
        } else {
            let _ = fs::remove_file(&self.socket_path);
        }
    }
}

#[derive(Debug)]
struct Args {
    iterations: usize,
    warmup: usize,
    socket_path: Option<PathBuf>,
    trace_path: Option<PathBuf>,
    json: bool,
    help: bool,
}

impl Args {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, Box<dyn std::error::Error>> {
        let mut parsed = Self {
            iterations: DEFAULT_ITERATIONS,
            warmup: DEFAULT_WARMUP,
            socket_path: None,
            trace_path: None,
            json: false,
            help: false,
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("-h" | "--help") => parsed.help = true,
                Some("--json") => parsed.json = true,
                Some("--iterations") => {
                    parsed.iterations = parse_usize_arg("--iterations", args.next())?;
                }
                Some("--warmup") => {
                    parsed.warmup = parse_usize_arg("--warmup", args.next())?;
                }
                Some("--socket") => {
                    parsed.socket_path = Some(PathBuf::from(
                        args.next().ok_or("--socket requires a path")?,
                    ));
                }
                Some("--trace") => {
                    parsed.trace_path =
                        Some(PathBuf::from(args.next().ok_or("--trace requires a path")?));
                }
                Some(other) => return Err(format!("unknown argument: {other}").into()),
                None => return Err(format!("non-UTF-8 argument: {arg:?}").into()),
            }
        }
        if parsed.iterations == 0 {
            return Err("--iterations must be greater than zero".into());
        }
        Ok(parsed)
    }
}

fn parse_usize_arg(
    name: &str,
    value: Option<OsString>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let value = value.ok_or_else(|| format!("{name} requires a value"))?;
    let value = value
        .to_str()
        .ok_or_else(|| format!("{name} value is not UTF-8"))?;
    value
        .parse::<usize>()
        .map_err(|err| format!("invalid {name} value {value:?}: {err}").into())
}

fn nmux_binary_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = std::env::var_os("NMUX_BIN") {
        return Ok(PathBuf::from(path));
    }
    let current = std::env::current_exe()?;
    let sibling = current.with_file_name("nmux");
    if sibling.exists() {
        return Ok(sibling);
    }
    Ok(current)
}

fn usage() -> &'static str {
    "Usage: nmux-latency-bench [--iterations N] [--warmup N] [--socket PATH] [--trace PATH] [--json]\n"
}

fn default_trace_path() -> Option<PathBuf> {
    Some(
        PathBuf::from("target")
            .join("nmux-latency")
            .join("trace.jsonl"),
    )
}
