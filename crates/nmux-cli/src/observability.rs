use std::path::Path;
use std::sync::OnceLock;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const TRACE_ENV: &str = "NMUX_TRACE";
const TRACE_FILE_ENV: &str = "NMUX_TRACE_FILE";
const DEFAULT_TRACE_FILTER: &str = "info,nmux_cli=trace,nmux_latency_bench=trace";
static TRACE_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

pub fn init_from_env() -> Result<(), Box<dyn std::error::Error>> {
    let trace_file = std::env::var_os(TRACE_FILE_ENV);
    let trace_enabled = std::env::var_os(TRACE_ENV).is_some() || trace_file.is_some();
    if !trace_enabled {
        return Ok(());
    }

    let filter = EnvFilter::try_from_env(TRACE_ENV)
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_TRACE_FILTER));
    match trace_file {
        Some(path) => init_with_file_and_filter(Path::new(&path), filter),
        None => init_stderr_with_filter(filter),
    }
}

pub fn init_with_file(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let filter = EnvFilter::try_from_env(TRACE_ENV)
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_TRACE_FILTER));
    init_with_file_and_filter(path, filter)
}

fn init_with_file_and_filter(
    path: &Path,
    filter: EnvFilter,
) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent)?;
    }
    let directory = parent.unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .ok_or("trace path must include a file name")?
        .to_owned();
    let appender = tracing_appender::rolling::never(directory, filename);
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let _ = TRACE_GUARD.set(guard);
    let layer = tracing_subscriber::fmt::layer()
        .json()
        .with_span_events(FmtSpan::CLOSE)
        .with_writer(writer);
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init();
    Ok(())
}

fn init_stderr_with_filter(filter: EnvFilter) -> Result<(), Box<dyn std::error::Error>> {
    let layer = tracing_subscriber::fmt::layer()
        .json()
        .with_span_events(FmtSpan::CLOSE);
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init();
    Ok(())
}
