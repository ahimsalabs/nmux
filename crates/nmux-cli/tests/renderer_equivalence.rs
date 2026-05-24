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
    for fixture in load_fixtures() {
        run_fixture(fixture);
    }
}

fn run_fixture(fixture: RendererFixture) {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server_command = Command::new(env!("CARGO_BIN_EXE_nmuxd"));
    server_command.args([
        "--socket",
        socket_path.to_str().expect("socket path"),
        "--one-shot",
        "--terminal-engine",
        "libghostty-vt",
    ]);
    if let Some(size) = fixture.initial_size {
        let cols = size.cols.to_string();
        let rows = size.rows.to_string();
        server_command.args(["--cols", cols.as_str(), "--rows", rows.as_str()]);
    }
    let mut server = server_command
        .args(["--command", fixture.command.as_str()])
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
    write_artifact_if_requested(&fixture.name, &stdout);

    let decoded: Value = serde_json::from_str(&stdout).expect("decode nmux json");
    let workspace = materialize_workspace(&decoded);
    assert_eq!(workspace, fixture.expected_workspace);

    let surface = materialize_surface(&decoded);
    assert_eq!(surface, fixture.expected_surface);

    let scrollback = materialize_scrollback(&decoded);
    assert_eq!(scrollback, fixture.expected_scrollback);
    for needle in fixture.absent_substrings {
        assert_absent(&stdout, &needle);
    }
}

#[derive(Debug)]
struct RendererFixture {
    name: String,
    command: String,
    initial_size: Option<FixtureSize>,
    expected_workspace: CanonicalWorkspace,
    expected_surface: CanonicalSurface,
    expected_scrollback: Vec<CanonicalRow>,
    absent_substrings: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct FixtureSize {
    cols: u64,
    rows: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CanonicalWorkspace {
    session_id: String,
    tab_id: String,
    pane_id: String,
    cols: u64,
    rows: u64,
    resize_policy: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CanonicalSurface {
    title: String,
    working_directory: String,
    surface_kind: String,
    cursor: CanonicalCursor,
    modes: CanonicalModes,
    styles: Vec<CanonicalStyle>,
    rows: Vec<CanonicalRow>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CanonicalCursor {
    row: u64,
    col: u64,
    visible: bool,
    shape: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CanonicalModes {
    bracketed_paste: bool,
    mouse_tracking: bool,
    focus_reporting: bool,
    application_keypad: bool,
    application_cursor: bool,
    origin: bool,
    wraparound: bool,
    mouse_tracking_mode: String,
    mouse_format: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CanonicalStyle {
    fg_rgba: u64,
    bg_rgba: u64,
    underline_rgba: u64,
    flags: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CanonicalRow {
    text: String,
    semantic_prompt: String,
    runs: Vec<CanonicalRun>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CanonicalRun {
    text: String,
    cell_widths: Vec<u8>,
    style_id: u64,
    flags: u64,
    semantic_content: String,
}

fn load_fixtures() -> Vec<RendererFixture> {
    let dir = workspace_path(PathBuf::from("fixtures/renderer-equivalence"));
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read renderer fixture dir {}: {error}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("read renderer fixture dir entry: {error}"))
                .path()
        })
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "renderer fixture dir {} has no json fixtures",
        dir.display()
    );
    paths
        .into_iter()
        .map(|path| load_fixture_path(&path))
        .collect()
}

fn load_fixture_path(path: &Path) -> RendererFixture {
    let json = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read renderer fixture {}: {error}", path.display()));
    let decoded: Value = serde_json::from_str(&json)
        .unwrap_or_else(|error| panic!("decode renderer fixture {}: {error}", path.display()));
    let expected = decoded.get("expected").expect("expected fixture object");
    RendererFixture {
        name: path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_else(|| panic!("fixture path has no utf-8 stem: {}", path.display()))
            .to_owned(),
        command: string_field(&decoded, "command"),
        initial_size: decoded.get("initial_size").map(materialize_fixture_size),
        expected_workspace: materialize_workspace(expected),
        expected_surface: materialize_surface(expected),
        expected_scrollback: expected
            .get("scrollback")
            .and_then(Value::as_array)
            .expect("expected scrollback rows")
            .iter()
            .map(materialize_row)
            .collect(),
        absent_substrings: decoded
            .get("absent_substrings")
            .and_then(Value::as_array)
            .expect("absent substrings")
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .expect("absent substring is string")
                    .to_owned()
            })
            .collect(),
    }
}

fn materialize_fixture_size(size: &Value) -> FixtureSize {
    FixtureSize {
        cols: numeric_field(size, "cols"),
        rows: numeric_field(size, "rows"),
    }
}

fn materialize_workspace(decoded: &Value) -> CanonicalWorkspace {
    let workspace = decoded.get("workspace").expect("workspace object");
    CanonicalWorkspace {
        session_id: string_field(workspace, "session_id"),
        tab_id: string_field(workspace, "tab_id"),
        pane_id: string_field(workspace, "pane_id"),
        cols: numeric_field(workspace, "cols"),
        rows: numeric_field(workspace, "rows"),
        resize_policy: string_field(workspace, "resize_policy"),
    }
}

fn materialize_surface(decoded: &Value) -> CanonicalSurface {
    let terminal = decoded.get("terminal").expect("terminal object");
    let surface = decoded.get("surface").expect("surface object");
    CanonicalSurface {
        title: string_field(terminal, "title"),
        working_directory: string_field(terminal, "working_directory"),
        surface_kind: string_field(terminal, "surface_kind"),
        cursor: materialize_cursor(terminal.get("cursor").expect("cursor object")),
        modes: materialize_modes(terminal.get("modes").expect("modes object")),
        styles: surface
            .get("styles")
            .and_then(Value::as_array)
            .expect("surface styles")
            .iter()
            .map(materialize_style)
            .collect(),
        rows: surface
            .get("row_updates")
            .and_then(Value::as_array)
            .expect("surface row updates")
            .iter()
            .map(materialize_row)
            .collect(),
    }
}

fn materialize_style(style: &Value) -> CanonicalStyle {
    CanonicalStyle {
        fg_rgba: numeric_field(style, "fg_rgba"),
        bg_rgba: numeric_field(style, "bg_rgba"),
        underline_rgba: numeric_field(style, "underline_rgba"),
        flags: numeric_field(style, "flags"),
    }
}

fn materialize_cursor(cursor: &Value) -> CanonicalCursor {
    CanonicalCursor {
        row: numeric_field(cursor, "row"),
        col: numeric_field(cursor, "col"),
        visible: bool_field(cursor, "visible"),
        shape: string_field(cursor, "shape"),
    }
}

fn materialize_modes(modes: &Value) -> CanonicalModes {
    CanonicalModes {
        bracketed_paste: bool_field(modes, "bracketed_paste"),
        mouse_tracking: bool_field(modes, "mouse_tracking"),
        focus_reporting: bool_field(modes, "focus_reporting"),
        application_keypad: bool_field(modes, "application_keypad"),
        application_cursor: bool_field(modes, "application_cursor"),
        origin: bool_field(modes, "origin"),
        wraparound: bool_field(modes, "wraparound"),
        mouse_tracking_mode: string_field(modes, "mouse_tracking_mode"),
        mouse_format: string_field(modes, "mouse_format"),
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
        semantic_prompt: string_field(row, "semantic_prompt"),
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
        semantic_content: string_field(run, "semantic_content"),
    }
}

fn string_field(value: &Value, name: &str) -> String {
    value
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string field {name} in {value:?}"))
        .to_owned()
}

fn bool_field(value: &Value, name: &str) -> bool {
    value
        .get(name)
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("missing bool field {name} in {value:?}"))
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

fn write_artifact_if_requested(fixture_name: &str, json: &str) {
    let Some(dir) = std::env::var_os("NMUX_RENDERER_EQUIVALENCE_ARTIFACT_DIR") else {
        return;
    };
    let dir = workspace_path(PathBuf::from(dir));
    fs::create_dir_all(&dir).expect("create renderer-equivalence artifact dir");
    fs::write(dir.join(format!("{fixture_name}.json")), json)
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
