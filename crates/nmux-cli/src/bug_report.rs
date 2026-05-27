use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use nmux_proto::protocol;
use serde_json::Value;

use crate::json::json_string;

const BUG_REPORT_DIR_ENV: &str = "NMUX_BUG_REPORT_DIR";
const TERMINAL_OUTPUT_TRACE_FILE: &str = "nmux-terminal-output.jsonl";
static BUG_REPORT_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_bug_report_dir(path: PathBuf) {
    let _ = BUG_REPORT_DIR.set(path);
}

pub fn record_process_error(
    binary: &str,
    err: &(dyn std::error::Error + 'static),
    args: impl IntoIterator<Item = OsString>,
) {
    let Some(dir) = bug_report_dir_from_env() else {
        return;
    };
    let args = args
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if let Err(write_err) = write_process_error(&dir, binary, err, &args) {
        eprintln!(
            "nmux: failed to write bug report to {}: {write_err}",
            dir.display()
        );
    }
}

pub fn record_signal_interrupt(binary: &str, signal: &str, args: &[String]) {
    let Some(dir) = bug_report_dir_from_env() else {
        return;
    };
    if let Err(write_err) = write_signal_interrupt(&dir, binary, signal, args) {
        eprintln!(
            "nmux: failed to write bug report to {}: {write_err}",
            dir.display()
        );
    }
}

pub fn record_frame_decode_error(context: &str, frame: &[u8], err: &dyn std::error::Error) {
    let Some(dir) = bug_report_dir_from_env() else {
        return;
    };
    if let Err(write_err) = write_frame_decode_error(&dir, context, frame, err) {
        eprintln!(
            "nmux: failed to write bug report to {}: {write_err}",
            dir.display()
        );
    }
}

pub fn record_terminal_output(pane_id: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let Some(dir) = bug_report_dir_from_env() else {
        return;
    };
    if let Err(write_err) = append_terminal_output_trace(&dir, pane_id, bytes) {
        eprintln!(
            "nmux: failed to write terminal output trace to {}: {write_err}",
            dir.display()
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalOutputTraceEntry {
    pub pane_id: String,
    pub bytes: Vec<u8>,
}

pub fn read_terminal_output_trace(path: &Path) -> io::Result<Vec<TerminalOutputTraceEntry>> {
    let file = fs::File::open(path)?;
    let mut entries = Vec::new();
    for (line_index, line) in io::BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        entries.push(parse_terminal_output_trace_line(&line).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "invalid terminal output trace line {}: {err}",
                    line_index + 1
                ),
            )
        })?);
    }
    Ok(entries)
}

fn bug_report_dir_from_env() -> Option<PathBuf> {
    if let Some(path) = BUG_REPORT_DIR.get() {
        return Some(path.clone());
    }
    std::env::var_os(BUG_REPORT_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn append_terminal_output_trace(dir: &Path, pane_id: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = ReportStamp::now();
    let trace_path = terminal_output_trace_path(dir);
    let mut trace = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&trace_path)?;
    writeln!(
        trace,
        "{{\"kind\":\"terminal-output\",\"timestamp_ms\":{},\"pid\":{},\"pane_id\":{},\"bytes_len\":{},\"bytes_hex\":{}}}",
        stamp.timestamp_ms,
        std::process::id(),
        json_string(pane_id),
        bytes.len(),
        json_string(&hex_encode(bytes)),
    )?;
    Ok(trace_path)
}

fn terminal_output_trace_path(dir: &Path) -> PathBuf {
    dir.join(TERMINAL_OUTPUT_TRACE_FILE)
}

fn write_process_error(
    dir: &Path,
    binary: &str,
    err: &(dyn std::error::Error + 'static),
    args: &[String],
) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = ReportStamp::now();
    let metadata_path = dir.join(format!("{}-process-error.json", stamp.file_prefix()));
    fs::write(
        &metadata_path,
        format!(
            "{{\"kind\":\"process-error\",\"timestamp_ms\":{},\"pid\":{},\"binary\":{},\"cwd\":{},\"error\":{},\"args\":[{}]}}\n",
            stamp.timestamp_ms,
            std::process::id(),
            json_string(binary),
            json_string(&cwd_for_report()),
            json_string(&err.to_string()),
            args.iter()
                .map(|arg| json_string(arg))
                .collect::<Vec<_>>()
                .join(",")
        ),
    )?;
    Ok(metadata_path)
}

fn write_signal_interrupt(
    dir: &Path,
    binary: &str,
    signal: &str,
    args: &[String],
) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = ReportStamp::now();
    let metadata_path = dir.join(format!("{}-signal-interrupt.json", stamp.file_prefix()));
    fs::write(
        &metadata_path,
        format!(
            "{{\"kind\":\"signal-interrupt\",\"timestamp_ms\":{},\"pid\":{},\"binary\":{},\"cwd\":{},\"signal\":{},\"args\":[{}]}}\n",
            stamp.timestamp_ms,
            std::process::id(),
            json_string(binary),
            json_string(&cwd_for_report()),
            json_string(signal),
            args.iter()
                .map(|arg| json_string(arg))
                .collect::<Vec<_>>()
                .join(",")
        ),
    )?;
    Ok(metadata_path)
}

fn write_frame_decode_error(
    dir: &Path,
    context: &str,
    frame: &[u8],
    err: &dyn std::error::Error,
) -> io::Result<(PathBuf, PathBuf)> {
    fs::create_dir_all(dir)?;
    let stamp = ReportStamp::now();
    let prefix = stamp.file_prefix();
    let frame_path = dir.join(format!("{prefix}-{context}.fb"));
    let metadata_path = dir.join(format!("{prefix}-{context}.json"));
    fs::write(&frame_path, frame)?;
    fs::write(
        &metadata_path,
        format!(
            "{{\"kind\":\"frame-decode-error\",\"timestamp_ms\":{},\"pid\":{},\"context\":{},\"cwd\":{},\"error\":{},\"frame_path\":{},\"frame_len\":{},\"envelope_body\":{}}}\n",
            stamp.timestamp_ms,
            std::process::id(),
            json_string(context),
            json_string(&cwd_for_report()),
            json_string(&err.to_string()),
            json_string(&frame_path.display().to_string()),
            frame.len(),
            frame_body_json(frame),
        ),
    )?;
    Ok((metadata_path, frame_path))
}

fn parse_terminal_output_trace_line(line: &str) -> Result<TerminalOutputTraceEntry, String> {
    let value = serde_json::from_str::<Value>(line).map_err(|err| err.to_string())?;
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("missing kind")?;
    if kind != "terminal-output" {
        return Err(format!("unexpected kind {kind}"));
    }
    let pane_id = value
        .get("pane_id")
        .and_then(Value::as_str)
        .ok_or("missing pane_id")?
        .to_owned();
    let bytes_hex = value
        .get("bytes_hex")
        .and_then(Value::as_str)
        .ok_or("missing bytes_hex")?;
    let bytes = hex_decode(bytes_hex)?;
    if let Some(bytes_len) = value.get("bytes_len").and_then(Value::as_u64) {
        if bytes_len != bytes.len() as u64 {
            return Err(format!(
                "bytes_len {bytes_len} does not match decoded length {}",
                bytes.len()
            ));
        }
    }
    Ok(TerminalOutputTraceEntry { pane_id, bytes })
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn hex_decode(hex: &str) -> Result<Vec<u8>, String> {
    if hex.len() % 2 != 0 {
        return Err("hex string has odd length".to_owned());
    }
    let mut decoded = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks_exact(2) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        decoded.push((high << 4) | low);
    }
    Ok(decoded)
}

fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(format!("invalid hex byte {byte:#x}")),
    }
}

fn frame_body_json(frame: &[u8]) -> String {
    match protocol::size_prefixed_root_as_envelope(frame) {
        Ok(envelope) => json_string(&format!("{:?}", envelope.body_type())),
        Err(err) => json_string(&format!("invalid envelope: {err}")),
    }
}

fn cwd_for_report() -> String {
    std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|err| format!("<unknown: {err}>"))
}

struct ReportStamp {
    timestamp_ms: u128,
    timestamp_ns: u128,
}

impl ReportStamp {
    fn now() -> Self {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            timestamp_ms: duration.as_millis(),
            timestamp_ns: duration.as_nanos(),
        }
    }

    fn file_prefix(&self) -> String {
        format!("nmux-{}-{}", std::process::id(), self.timestamp_ns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_bug_report_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "nmux-bug-report-{name}-{}-{}",
            std::process::id(),
            ReportStamp::now().timestamp_ns
        ))
    }

    #[test]
    fn signal_interrupt_report_writes_metadata() {
        let dir = temp_bug_report_dir("signal-interrupt-test");
        let args = vec![
            "nmux".to_owned(),
            "--bug-report-dir".to_owned(),
            dir.display().to_string(),
        ];

        let metadata_path =
            write_signal_interrupt(&dir, "nmux", "SIGINT", &args).expect("write report");

        let metadata = fs::read_to_string(metadata_path).expect("read metadata");
        assert!(metadata.contains("\"kind\":\"signal-interrupt\""));
        assert!(metadata.contains("\"binary\":\"nmux\""));
        assert!(metadata.contains("\"signal\":\"SIGINT\""));
        assert!(metadata.contains("\"--bug-report-dir\""));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn frame_decode_report_writes_raw_frame_and_metadata() {
        let dir = temp_bug_report_dir("frame-decode-test");
        let frame = [1_u8, 2, 3, 4];
        let err = io::Error::new(io::ErrorKind::InvalidData, "decode failed");

        let (metadata_path, frame_path) =
            write_frame_decode_error(&dir, "live-server-frame", &frame, &err)
                .expect("write report");

        assert_eq!(fs::read(&frame_path).expect("read frame"), frame);
        let metadata = fs::read_to_string(metadata_path).expect("read metadata");
        assert!(metadata.contains("\"kind\":\"frame-decode-error\""));
        assert!(metadata.contains("\"context\":\"live-server-frame\""));
        assert!(metadata.contains("\"error\":\"decode failed\""));
        assert!(metadata.contains("\"frame_len\":4"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn terminal_output_trace_round_trips_chunks() {
        let dir = temp_bug_report_dir("terminal-output-round-trip");

        let path = append_terminal_output_trace(&dir, "pane-1", b"first\n").expect("write first");
        append_terminal_output_trace(&dir, "pane-1", b"\x1b[31msecond\x1b[0m\n")
            .expect("write second");

        let entries = read_terminal_output_trace(&path).expect("read trace");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].pane_id, "pane-1");
        assert_eq!(entries[0].bytes, b"first\n");
        assert_eq!(entries[1].bytes, b"\x1b[31msecond\x1b[0m\n");

        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(feature = "libghostty-vt")]
    #[test]
    fn terminal_output_trace_replays_to_contiguous_scrollback() {
        use nmux_core::terminal::{
            PaneTerminalEngines, TerminalColors, TerminalCursor, TerminalEngineKind, TerminalInput,
            TerminalModes, TerminalUpdate,
        };

        fn input_from_update<'a>(
            update: Option<&'a TerminalUpdate>,
            empty_surface: &'a [String],
            empty_scrollback: &'a [String],
        ) -> TerminalInput<'a> {
            match update {
                Some(update) => TerminalInput {
                    pane_id: "pane-1",
                    cols: 80,
                    rows: update.surface_lines.len() as u32,
                    surface: update.surface,
                    cursor: update.cursor,
                    modes: update.modes,
                    title: &update.title,
                    working_directory: &update.working_directory,
                    colors: update.colors.clone(),
                    styles: &update.styles,
                    surface_lines: &update.surface_lines,
                    surface_row_runs: &update.surface_row_runs,
                    surface_semantic_prompts: &update.surface_semantic_prompts,
                    surface_dirty_rows: &update.surface_dirty_rows,
                    surface_kitty_placeholders: &update.surface_kitty_placeholders,
                    scrollback_lines: &update.scrollback_lines,
                    scrollback_row_runs: &update.scrollback_row_runs,
                    scrollback_semantic_prompts: &update.scrollback_semantic_prompts,
                    scrollback_dirty_rows: &update.scrollback_dirty_rows,
                    scrollback_kitty_placeholders: &update.scrollback_kitty_placeholders,
                },
                None => TerminalInput {
                    pane_id: "pane-1",
                    cols: 80,
                    rows: empty_surface.len() as u32,
                    surface: protocol::SurfaceKind::Main,
                    cursor: TerminalCursor {
                        row: 0,
                        col: 0,
                        visible: true,
                        shape: protocol::CursorShape::Block,
                        blinking: true,
                    },
                    modes: TerminalModes::default(),
                    title: "",
                    working_directory: "",
                    colors: TerminalColors::default(),
                    styles: &[],
                    surface_lines: empty_surface,
                    surface_row_runs: &[],
                    surface_semantic_prompts: &[],
                    surface_dirty_rows: &[],
                    surface_kitty_placeholders: &[],
                    scrollback_lines: empty_scrollback,
                    scrollback_row_runs: &[],
                    scrollback_semantic_prompts: &[],
                    scrollback_dirty_rows: &[],
                    scrollback_kitty_placeholders: &[],
                },
            }
        }

        fn trace_line_number(line: &str) -> Option<usize> {
            let number = line.strip_prefix("trace-line ")?;
            number.trim().parse::<usize>().ok()
        }

        let dir = temp_bug_report_dir("terminal-output-replay");
        let mut output = Vec::new();
        for line in 0..200 {
            output.extend_from_slice(format!("trace-line {line:04}\r\n").as_bytes());
        }
        let mut offset = 0;
        let mut path = terminal_output_trace_path(&dir);
        while offset < output.len() {
            let chunk_len = ((offset * 17) % 41 + 1).min(output.len() - offset);
            path =
                append_terminal_output_trace(&dir, "pane-1", &output[offset..offset + chunk_len])
                    .expect("write terminal output chunk");
            offset += chunk_len;
        }

        let entries = read_terminal_output_trace(&path).expect("read terminal output trace");
        let mut engines = PaneTerminalEngines::new(TerminalEngineKind::LibghosttyVt);
        let empty_surface = vec![String::new(); 12];
        let empty_scrollback = Vec::new();
        let mut update = None;
        for entry in entries {
            assert_eq!(entry.pane_id, "pane-1");
            if let Some(next) = engines.engine_mut("pane-1").apply_output(
                input_from_update(update.as_ref(), &empty_surface, &empty_scrollback),
                &entry.bytes,
            ) {
                update = Some(next);
            }
        }

        let update = update.expect("trace produced terminal update");
        let transcript = update
            .scrollback_lines
            .iter()
            .filter_map(|line| trace_line_number(line))
            .collect::<Vec<_>>();
        assert_eq!(transcript, (0..200).collect::<Vec<_>>());
        let surface = update
            .surface_lines
            .iter()
            .filter_map(|line| trace_line_number(line))
            .collect::<Vec<_>>();
        assert!(!surface.is_empty(), "surface did not include trace rows");
        assert_eq!(surface, transcript[transcript.len() - surface.len()..]);

        let _ = fs::remove_dir_all(dir);
    }
}
