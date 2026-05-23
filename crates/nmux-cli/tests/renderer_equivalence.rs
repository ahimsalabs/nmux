#![cfg(feature = "libghostty-vt")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

static NEXT_PATH_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn renderer_equivalence_smoke_captures_structured_nmux_state() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = concat!(
        "printf '\\033]0;renderer fixture\\007'; ",
        "printf '\\033]7;file://localhost/tmp/nmux-renderer\\033\\\\'; ",
        "printf '\\033[31mred\\033[0m plain'; printf '\\n'; ",
        "printf 'wide:中'; printf '\\n'; ",
        "printf '\\033]8;;https://example.invalid\\033\\\\link\\033]8;;\\033\\\\'; printf '\\n'; ",
        "printf 'main-scroll'; printf '\\n'; ",
        "printf '\\033[?1049hALT-ONLY\\033[?1049l'; ",
        "cat >/dev/null"
    );

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--connect-timeout-ms",
            "5000",
            "--json",
            "--scrollback-start",
            "1",
            "--scrollback-count",
            "20",
        ])
        .output()
        .expect("run nmux --json");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --json failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8(client.stdout).expect("json stdout is utf-8");
    write_artifact_if_requested(&stdout);

    let decoded: Value = serde_json::from_str(&stdout).expect("decode nmux json");
    let surface = materialize_surface(&decoded);
    let expected_surface = CanonicalSurface {
        title: "renderer fixture".to_owned(),
        working_directory: "file://localhost/tmp/nmux-renderer".to_owned(),
        surface_kind: "main".to_owned(),
        rows: vec![
            CanonicalRow {
                text: "red plain".to_owned(),
                runs: vec![
                    CanonicalRun {
                        text: "red".to_owned(),
                        cell_widths: vec![1, 1, 1],
                        style_id: 1,
                        flags: 0,
                    },
                    CanonicalRun {
                        text: " plain".to_owned(),
                        cell_widths: vec![1, 1, 1, 1, 1, 1],
                        style_id: 0,
                        flags: 0,
                    },
                ],
            },
            CanonicalRow {
                text: "wide:中".to_owned(),
                runs: vec![CanonicalRun {
                    text: "wide:中".to_owned(),
                    cell_widths: vec![1, 1, 1, 1, 1, 2],
                    style_id: 0,
                    flags: 0,
                }],
            },
            CanonicalRow {
                text: "link".to_owned(),
                runs: vec![CanonicalRun {
                    text: "link".to_owned(),
                    cell_widths: vec![1, 1, 1, 1],
                    style_id: 0,
                    flags: 1,
                }],
            },
            CanonicalRow {
                text: "main-scroll".to_owned(),
                runs: vec![CanonicalRun {
                    text: "main-scroll".to_owned(),
                    cell_widths: vec![1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
                    style_id: 0,
                    flags: 0,
                }],
            },
        ],
    };
    assert_eq!(surface, expected_surface);

    let scrollback = materialize_scrollback(&decoded);
    assert_eq!(
        scrollback,
        vec![
            expected_surface.rows[0].clone(),
            expected_surface.rows[1].clone(),
            expected_surface.rows[2].clone(),
            expected_surface.rows[3].clone(),
        ]
    );
    assert_absent(&stdout, "\u{1b}[31m");
    assert_absent(&stdout, "ALT-ONLY");
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CanonicalSurface {
    title: String,
    working_directory: String,
    surface_kind: String,
    rows: Vec<CanonicalRow>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CanonicalRow {
    text: String,
    runs: Vec<CanonicalRun>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CanonicalRun {
    text: String,
    cell_widths: Vec<u8>,
    style_id: u64,
    flags: u64,
}

fn materialize_surface(decoded: &Value) -> CanonicalSurface {
    let terminal = decoded.get("terminal").expect("terminal object");
    let surface = decoded.get("surface").expect("surface object");
    CanonicalSurface {
        title: string_field(terminal, "title"),
        working_directory: string_field(terminal, "working_directory"),
        surface_kind: string_field(terminal, "surface_kind"),
        rows: surface
            .get("row_updates")
            .and_then(Value::as_array)
            .expect("surface row updates")
            .iter()
            .map(materialize_row)
            .collect(),
    }
}

fn materialize_scrollback(decoded: &Value) -> Vec<CanonicalRow> {
    decoded
        .get("scrollback")
        .expect("scrollback object")
        .get("lines")
        .and_then(Value::as_array)
        .expect("scrollback lines")
        .iter()
        .filter(|row| !string_field(row, "text").is_empty())
        .map(materialize_row)
        .collect()
}

fn materialize_row(row: &Value) -> CanonicalRow {
    CanonicalRow {
        text: string_field(row, "text"),
        runs: row
            .get("runs")
            .and_then(Value::as_array)
            .expect("row runs")
            .iter()
            .map(materialize_run)
            .collect(),
    }
}

fn materialize_run(run: &Value) -> CanonicalRun {
    CanonicalRun {
        text: string_field(run, "text"),
        cell_widths: run
            .get("cell_widths")
            .and_then(Value::as_array)
            .expect("cell widths")
            .iter()
            .map(|width| {
                width
                    .as_u64()
                    .and_then(|value| u8::try_from(value).ok())
                    .expect("cell width fits u8")
            })
            .collect(),
        style_id: numeric_field(run, "style_id"),
        flags: numeric_field(run, "flags"),
    }
}

fn string_field(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string field {name} in {value:?}"))
        .to_owned()
}

fn numeric_field(value: &Value, name: &str) -> u64 {
    value
        .get(name)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("missing numeric field {name} in {value:?}"))
}

fn test_socket_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let id = NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(format!(
        "/tmp/nmux-renderer-{}-{nanos:x}-{id}.sock",
        std::process::id()
    ))
}

fn wait_for_socket(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("socket did not appear: {}", path.display());
}

fn write_artifact_if_requested(json: &str) {
    let Some(dir) = std::env::var_os("NMUX_RENDERER_EQUIVALENCE_ARTIFACT_DIR") else {
        return;
    };
    let dir = workspace_path(PathBuf::from(dir));
    fs::create_dir_all(&dir).expect("create renderer-equivalence artifact dir");
    fs::write(dir.join("libghostty-vt-smoke.json"), json)
        .expect("write renderer-equivalence artifact");
}

fn workspace_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn assert_absent(haystack: &str, needle: &str) {
    assert!(
        !haystack.contains(needle),
        "unexpected {needle:?}:\n{haystack}"
    );
}
