use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nmux_cli::{local, observability};
use nmux_core::session::AttachMode;
use tracing::instrument;

const DEFAULT_ITERATIONS: usize = 50;
const DEFAULT_WARMUP: usize = 5;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

fn main() {
    if let Err(err) = run() {
        eprintln!("nmux-latency-bench: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
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
    let mut daemon = start_daemon(
        &nmux,
        &workspace.socket_path,
        workspace.trace_path.as_deref(),
    )?;
    let result = run_latency_suite(&workspace.socket_path, args.iterations, args.warmup);
    let _ = daemon.kill();
    let _ = daemon.wait();
    let report = result?;
    if args.json {
        println!("{}", report.to_json(workspace.trace_path.as_deref()));
    } else {
        println!("{}", report.display(workspace.trace_path.as_deref()));
    }
    Ok(())
}

#[instrument(level = "info", skip(socket_path))]
fn run_latency_suite(
    socket_path: &Path,
    iterations: usize,
    warmup: usize,
) -> Result<LatencyReport, Box<dyn std::error::Error>> {
    let mut stream = local::connect_to_daemon_with_timeout(socket_path, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(DEFAULT_TIMEOUT))?;
    let request = local::AttachRequest {
        actor_id: "latency-bench".to_owned(),
        user_id: "latency-bench".to_owned(),
        display_name: "latency bench".to_owned(),
        mode: AttachMode::ReadWrite,
        focused_pane_id: Some("pane-1".to_owned()),
        known_surfaces: Vec::new(),
    };
    local::write_attach_request(&mut stream, &request)?;
    let snapshot = local::attach_from_stream(&mut stream)?;
    let pane_id = snapshot.status.pane_id;
    let mut sequence = local::ClientFrameSequence::default();
    let mut samples = Vec::with_capacity(iterations);

    for index in 0..(warmup + iterations) {
        let token = format!("{}-{}", std::process::id(), index);
        let elapsed = measure_echo_latency(&mut stream, &mut sequence, &pane_id, &token)?;
        if index >= warmup {
            samples.push(elapsed);
        }
    }

    Ok(LatencyReport::from_samples(samples))
}

#[instrument(level = "trace", skip(stream, sequence, token), fields(pane_id = %pane_id, token = %token))]
fn measure_echo_latency(
    stream: &mut UnixStream,
    sequence: &mut local::ClientFrameSequence,
    pane_id: &str,
    token: &str,
) -> Result<Duration, Box<dyn std::error::Error>> {
    let input = format!("{token}\n");
    let expected = format!("nmux-latency:{token}");
    let start = Instant::now();
    local::send_key_input_with_sequence(stream, sequence, pane_id, &input)?;
    loop {
        match local::read_live_surface_update_from_stream(stream)? {
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
            | local::LiveSurfaceRead::NoFrame => {}
        }
    }
}

fn start_daemon(
    nmux: &Path,
    socket_path: &Path,
    trace_path: Option<&Path>,
) -> Result<Child, Box<dyn std::error::Error>> {
    let command =
        "stty -echo; while IFS= read -r line; do printf 'nmux-latency:%s\\n' \"$line\"; done";
    let mut child = Command::new(nmux)
        .args([
            "daemon",
            "--socket",
            socket_path.to_str().ok_or("socket path is not UTF-8")?,
            "--ready-json",
            "--live-forever",
            "--command",
            command,
        ])
        .envs(trace_path.map(|path| ("NMUX_TRACE_FILE", path.as_os_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("failed to spawn nmux daemon: {err}"))?;
    wait_for_ready(&mut child)?;
    Ok(child)
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

#[derive(Debug)]
struct LatencyReport {
    samples: Vec<Duration>,
    min: Duration,
    p50: Duration,
    p90: Duration,
    p99: Duration,
    max: Duration,
    mean: Duration,
}

impl LatencyReport {
    fn from_samples(mut samples: Vec<Duration>) -> Self {
        samples.sort_unstable();
        let total_nanos: u128 = samples.iter().map(Duration::as_nanos).sum();
        let mean = Duration::from_nanos((total_nanos / samples.len() as u128) as u64);
        Self {
            min: samples[0],
            p50: percentile(&samples, 50),
            p90: percentile(&samples, 90),
            p99: percentile(&samples, 99),
            max: *samples.last().expect("samples are non-empty"),
            mean,
            samples,
        }
    }

    fn display(&self, trace_path: Option<&Path>) -> String {
        let mut output = format!(
            "input-to-display latency: n={} min={} mean={} p50={} p90={} p99={} max={}",
            self.samples.len(),
            format_duration(self.min),
            format_duration(self.mean),
            format_duration(self.p50),
            format_duration(self.p90),
            format_duration(self.p99),
            format_duration(self.max)
        );
        if let Some(trace_path) = trace_path {
            output.push_str(&format!("\ntrace={}", trace_path.display()));
        }
        output
    }

    fn to_json(&self, trace_path: Option<&Path>) -> String {
        format!(
            "{{\"samples\":{},\"min_us\":{},\"mean_us\":{},\"p50_us\":{},\"p90_us\":{},\"p99_us\":{},\"max_us\":{},\"trace_path\":{}}}",
            self.samples.len(),
            self.min.as_micros(),
            self.mean.as_micros(),
            self.p50.as_micros(),
            self.p90.as_micros(),
            self.p99.as_micros(),
            self.max.as_micros(),
            trace_path
                .map(|path| nmux_cli::json::json_string(&path.display().to_string()))
                .unwrap_or_else(|| "null".to_owned())
        )
    }
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    let index = ((samples.len() - 1) * percentile).div_ceil(100);
    samples[index]
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
