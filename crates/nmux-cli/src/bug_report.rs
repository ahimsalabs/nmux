use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use nmux_proto::protocol;

use crate::json::json_string;

const BUG_REPORT_DIR_ENV: &str = "NMUX_BUG_REPORT_DIR";
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

fn bug_report_dir_from_env() -> Option<PathBuf> {
    if let Some(path) = BUG_REPORT_DIR.get() {
        return Some(path.clone());
    }
    std::env::var_os(BUG_REPORT_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
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

    #[test]
    fn frame_decode_report_writes_raw_frame_and_metadata() {
        let dir = std::env::temp_dir().join(format!(
            "nmux-bug-report-test-{}-{}",
            std::process::id(),
            ReportStamp::now().timestamp_ns
        ));
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
}
