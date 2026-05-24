#![cfg(feature = "libghostty-vt")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    hash::{Hash, Hasher},
};

use serde_json::{Value, json};

static NEXT_PATH_ID: AtomicU64 = AtomicU64::new(0);

fn daemon_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nmux"));
    command.arg("daemon");
    command
}

#[test]
fn renderer_equivalence_smoke_captures_structured_nmux_state() {
    for fixture in load_fixtures() {
        if fixture.requires_kitty_graphics
            && !nmux_core::terminal::libghostty_vt_supports_kitty_graphics()
        {
            continue;
        }
        run_fixture(fixture);
    }
}

fn run_fixture(fixture: RendererFixture) {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server_command = daemon_command();
    server_command.args([
        "--socket",
        socket_path.to_str().expect("socket path"),
        "--terminal-engine",
        "libghostty-vt",
    ]);
    if let Some(size) = fixture.initial_size {
        let cols = size.cols.to_string();
        let rows = size.rows.to_string();
        server_command.args(["--cols", cols.as_str(), "--rows", rows.as_str()]);
    }
    if fixture.resize.is_some() {
        server_command.args(["--live-clients", "2", "--live-cycles", "1"]);
    } else {
        server_command.arg("--one-shot");
    }
    let mut server = server_command
        .args(["--command", fixture.command.as_str()])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    if let Some(size) = &fixture.resize {
        let cols = size.cols.to_string();
        let rows = size.rows.to_string();
        let resize_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
            .args([
                "--socket",
                socket_path.to_str().expect("socket path"),
                "--connect-timeout-ms",
                "5000",
                "--live",
                "--iterations",
                "3",
                "--cols",
                cols.as_str(),
                "--rows",
                rows.as_str(),
                "--interval-ms",
                "100",
            ])
            .output()
            .expect("run nmux resize client");
        assert!(
            resize_client.status.success(),
            "nmux resize client failed: {}",
            String::from_utf8_lossy(&resize_client.stderr)
        );
    }

    let mut client_command = Command::new(env!("CARGO_BIN_EXE_nmux"));
    client_command.args([
        "--socket",
        socket_path.to_str().expect("socket path"),
        "--connect-timeout-ms",
        "5000",
        "--json",
        "--scrollback-start",
        "1",
        "--scrollback-count",
        "20",
    ]);
    if fixture.resize.is_some() {
        client_command.args([
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--interval-ms",
            "100",
        ]);
    }
    let client = client_command.output().expect("run nmux --json");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --json failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8(client.stdout).expect("json stdout is utf-8");
    let decoded = decode_renderer_client_json(&stdout, fixture.resize.is_some());
    let workspace = materialize_workspace(&decoded);
    let surface = materialize_surface(&decoded);
    let scrollback = materialize_scrollback(&decoded);
    let canonical = canonical_artifact_json(&workspace, &surface, &scrollback);
    write_artifacts_if_requested(&fixture.name, &stdout, &canonical);
    compare_oracle_if_requested(&fixture.name, &canonical);

    assert_eq!(workspace, fixture.expected_workspace);

    assert_eq!(surface, fixture.expected_surface);

    assert_eq!(scrollback, fixture.expected_scrollback);
    for needle in fixture.absent_substrings {
        assert_absent(&stdout, &needle);
    }
}

fn decode_renderer_client_json(stdout: &str, live: bool) -> Value {
    if !live {
        return serde_json::from_str(stdout).expect("decode nmux json");
    }
    for line in stdout.lines() {
        let decoded: Value = serde_json::from_str(line).expect("decode live nmux json line");
        if decoded.get("event").and_then(Value::as_str) == Some("attach") {
            return decoded
                .get("attach")
                .expect("live attach event has attach object")
                .clone();
        }
    }
    panic!("live nmux json did not include attach event:\n{stdout}");
}

#[derive(Debug)]
struct RendererFixture {
    name: String,
    command: String,
    requires_kitty_graphics: bool,
    initial_size: Option<FixtureSize>,
    resize: Option<FixtureSize>,
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
    colors: CanonicalColors,
    styles: Vec<CanonicalStyle>,
    rows: Vec<CanonicalRow>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CanonicalCursor {
    row: u64,
    col: u64,
    visible: bool,
    shape: String,
    blinking: bool,
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
struct CanonicalColors {
    default_fg_rgba: u64,
    default_bg_rgba: u64,
    cursor_rgba: u64,
    cursor_rgba_set: bool,
    palette_len: usize,
    palette_prefix: Vec<u64>,
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
    index: u64,
    text: String,
    dirty_hash: u64,
    row_state_hash: u64,
    semantic_prompt: String,
    dirty: bool,
    kitty_virtual_placeholder: bool,
    runs: Vec<CanonicalRun>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct CanonicalRun {
    text: String,
    cell_widths: Vec<u8>,
    style_id: u64,
    flags: u64,
    hyperlink_id: u64,
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
    let json = fs::read_to_string(path)
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
        requires_kitty_graphics: decoded
            .get("requires_kitty_graphics")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        initial_size: decoded.get("initial_size").map(materialize_fixture_size),
        resize: decoded.get("resize").map(materialize_fixture_size),
        expected_workspace: materialize_workspace(expected),
        expected_surface: materialize_surface(expected),
        expected_scrollback: expected
            .get("scrollback")
            .and_then(Value::as_array)
            .expect("expected scrollback rows")
            .iter()
            .enumerate()
            .map(|(index, row)| materialize_row(row, "line", index as u64 + 1))
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
        colors: materialize_colors(surface.get("colors").expect("surface colors")),
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
            .enumerate()
            .map(|(index, row)| materialize_row(row, "row", index as u64))
            .collect(),
    }
}

fn materialize_colors(colors: &Value) -> CanonicalColors {
    let palette = colors.get("palette_rgba").and_then(Value::as_array);
    let palette_prefix = match palette {
        Some(palette) => palette
            .iter()
            .take(16)
            .map(|value| value.as_u64().expect("palette entry is numeric"))
            .collect(),
        None => colors
            .get("palette_prefix")
            .and_then(Value::as_array)
            .expect("palette prefix")
            .iter()
            .map(|value| value.as_u64().expect("palette prefix entry is numeric"))
            .collect(),
    };
    CanonicalColors {
        default_fg_rgba: numeric_field(colors, "default_fg_rgba"),
        default_bg_rgba: numeric_field(colors, "default_bg_rgba"),
        cursor_rgba: numeric_field(colors, "cursor_rgba"),
        cursor_rgba_set: bool_field(colors, "cursor_rgba_set"),
        palette_len: palette
            .map(|palette| palette.len())
            .unwrap_or_else(|| numeric_field(colors, "palette_len") as usize),
        palette_prefix,
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
        blinking: bool_field(cursor, "blinking"),
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
        .enumerate()
        .map(|(index, row)| materialize_row(row, "line", index as u64 + 1))
        .collect()
}

fn materialize_row(row: &Value, index_field: &str, fallback_index: u64) -> CanonicalRow {
    let text = string_field(row, "text");
    let semantic_prompt = string_field(row, "semantic_prompt");
    let dirty = bool_field(row, "dirty");
    let kitty_virtual_placeholder = bool_field(row, "kitty_virtual_placeholder");
    let runs = row
        .get("runs")
        .and_then(Value::as_array)
        .expect("row runs")
        .iter()
        .map(materialize_run)
        .collect::<Vec<_>>();
    CanonicalRow {
        index: optional_numeric_field(row, index_field).unwrap_or(fallback_index),
        dirty_hash: optional_numeric_field(row, "dirty_hash")
            .unwrap_or_else(|| stable_row_hash(&text)),
        row_state_hash: optional_numeric_field(row, "row_state_hash").unwrap_or_else(|| {
            row_state_hash(&runs, &semantic_prompt, dirty, kitty_virtual_placeholder)
        }),
        text,
        semantic_prompt,
        dirty,
        kitty_virtual_placeholder,
        runs,
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
        hyperlink_id: optional_numeric_field(run, "hyperlink_id").unwrap_or(0),
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
    optional_numeric_field(value, name)
        .unwrap_or_else(|| panic!("missing numeric field {name} in {value:?}"))
}

fn optional_numeric_field(value: &Value, name: &str) -> Option<u64> {
    value.get(name).and_then(Value::as_u64)
}

fn stable_row_hash(line: &str) -> u64 {
    let mut hasher = StableHasher::new();
    hasher.write(line.as_bytes());
    hasher.finish()
}

fn row_state_hash(
    runs: &[CanonicalRun],
    semantic_prompt: &str,
    dirty: bool,
    kitty_virtual_placeholder: bool,
) -> u64 {
    let mut hasher = StableHasher::new();
    for run in runs {
        run.text.hash(&mut hasher);
        run.cell_widths.hash(&mut hasher);
        (run.style_id as u32).hash(&mut hasher);
        (run.flags as u32).hash(&mut hasher);
        (run.hyperlink_id as u32).hash(&mut hasher);
        cell_semantic_content_value(&run.semantic_content).hash(&mut hasher);
    }
    row_semantic_prompt_value(semantic_prompt).hash(&mut hasher);
    dirty.hash(&mut hasher);
    kitty_virtual_placeholder.hash(&mut hasher);
    hasher.finish()
}

fn row_semantic_prompt_value(name: &str) -> u8 {
    match name {
        "none" => 0,
        "prompt" => 1,
        "continuation" => 2,
        _ => panic!("unknown row semantic prompt {name}"),
    }
}

fn cell_semantic_content_value(name: &str) -> u8 {
    match name {
        "output" => 0,
        "input" => 1,
        "prompt" => 2,
        _ => panic!("unknown cell semantic content {name}"),
    }
}

struct StableHasher(u64);

impl StableHasher {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325_u64)
    }
}

impl Hasher for StableHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
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

fn write_artifacts_if_requested(fixture_name: &str, raw_json: &str, canonical: &Value) {
    let Some(dir) = std::env::var_os("NMUX_RENDERER_EQUIVALENCE_ARTIFACT_DIR") else {
        return;
    };
    let dir = workspace_path(PathBuf::from(dir));
    fs::create_dir_all(&dir).expect("create renderer-equivalence artifact dir");
    fs::write(dir.join(format!("{fixture_name}.json")), raw_json)
        .expect("write raw renderer-equivalence artifact");
    let canonical_json = serde_json::to_string_pretty(canonical)
        .expect("encode canonical renderer-equivalence artifact");
    fs::write(
        dir.join(format!("{fixture_name}.canonical.json")),
        format!("{canonical_json}\n"),
    )
    .expect("write canonical renderer-equivalence artifact");
}

fn compare_oracle_if_requested(fixture_name: &str, actual: &Value) {
    let Some(dir) = std::env::var_os("NMUX_RENDERER_EQUIVALENCE_ORACLE_DIR") else {
        return;
    };
    compare_oracle_fixture(fixture_name, actual, &workspace_path(PathBuf::from(dir)));
}

fn compare_oracle_fixture(fixture_name: &str, actual: &Value, oracle_dir: &Path) {
    let oracle_path = oracle_dir.join(format!("{fixture_name}.canonical.json"));
    let oracle_json = fs::read_to_string(&oracle_path).unwrap_or_else(|error| {
        panic!(
            "read renderer-equivalence oracle {}: {error}",
            oracle_path.display()
        )
    });
    let expected: Value = serde_json::from_str(&oracle_json).unwrap_or_else(|error| {
        panic!(
            "decode renderer-equivalence oracle {}: {error}",
            oracle_path.display()
        )
    });
    assert_json_eq_with_path(
        &expected,
        actual,
        &format!("renderer-equivalence oracle mismatch for {fixture_name}"),
    );
}

fn assert_json_eq_with_path(expected: &Value, actual: &Value, context: &str) {
    if expected == actual {
        return;
    }
    let path = json_mismatch_path(expected, actual);
    panic!("{context} at {path}\nexpected: {expected}\nactual: {actual}");
}

fn json_mismatch_path(expected: &Value, actual: &Value) -> String {
    json_mismatch_path_inner(expected, actual, "$").unwrap_or_else(|| "$".to_owned())
}

fn json_mismatch_path_inner(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            let keys: BTreeSet<&String> = expected.keys().chain(actual.keys()).collect();
            for key in keys {
                let child = json_path_field(path, key);
                match (expected.get(key), actual.get(key)) {
                    (Some(expected), Some(actual)) => {
                        if let Some(path) = json_mismatch_path_inner(expected, actual, &child) {
                            return Some(path);
                        }
                    }
                    _ => return Some(child),
                }
            }
            None
        }
        (Value::Array(expected), Value::Array(actual)) => {
            let shared = expected.len().min(actual.len());
            for index in 0..shared {
                let child = format!("{path}[{index}]");
                if let Some(path) =
                    json_mismatch_path_inner(&expected[index], &actual[index], &child)
                {
                    return Some(path);
                }
            }
            (expected.len() != actual.len()).then(|| format!("{path}.length"))
        }
        _ => (expected != actual).then(|| path.to_owned()),
    }
}

fn json_path_field(parent: &str, key: &str) -> String {
    if key
        .chars()
        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        return format!("{parent}.{key}");
    }
    let mut escaped = String::new();
    write!(
        &mut escaped,
        "{}[{}]",
        parent,
        serde_json::to_string(key).expect("encode json path key")
    )
    .expect("write json path");
    escaped
}

fn canonical_artifact_json(
    workspace: &CanonicalWorkspace,
    surface: &CanonicalSurface,
    scrollback: &[CanonicalRow],
) -> Value {
    json!({
        "workspace": canonical_workspace_json(workspace),
        "terminal": canonical_terminal_json(surface),
        "surface": canonical_surface_json(surface),
        "scrollback": canonical_rows_json(scrollback, "line"),
    })
}

fn canonical_workspace_json(workspace: &CanonicalWorkspace) -> Value {
    json!({
        "session_id": workspace.session_id,
        "tab_id": workspace.tab_id,
        "pane_id": workspace.pane_id,
        "cols": workspace.cols,
        "rows": workspace.rows,
        "resize_policy": workspace.resize_policy,
    })
}

fn canonical_terminal_json(surface: &CanonicalSurface) -> Value {
    json!({
        "title": surface.title,
        "working_directory": surface.working_directory,
        "surface_kind": surface.surface_kind,
        "cursor": {
            "row": surface.cursor.row,
            "col": surface.cursor.col,
            "visible": surface.cursor.visible,
            "shape": surface.cursor.shape,
            "blinking": surface.cursor.blinking,
        },
        "modes": {
            "bracketed_paste": surface.modes.bracketed_paste,
            "mouse_tracking": surface.modes.mouse_tracking,
            "focus_reporting": surface.modes.focus_reporting,
            "application_keypad": surface.modes.application_keypad,
            "application_cursor": surface.modes.application_cursor,
            "origin": surface.modes.origin,
            "wraparound": surface.modes.wraparound,
            "mouse_tracking_mode": surface.modes.mouse_tracking_mode,
            "mouse_format": surface.modes.mouse_format,
        },
    })
}

fn canonical_surface_json(surface: &CanonicalSurface) -> Value {
    json!({
        "colors": canonical_colors_json(&surface.colors),
        "styles": surface
            .styles
            .iter()
            .map(canonical_style_json)
            .collect::<Vec<_>>(),
        "row_updates": canonical_rows_json(&surface.rows, "row"),
    })
}

fn canonical_colors_json(colors: &CanonicalColors) -> Value {
    json!({
        "default_fg_rgba": colors.default_fg_rgba,
        "default_bg_rgba": colors.default_bg_rgba,
        "cursor_rgba": colors.cursor_rgba,
        "cursor_rgba_set": colors.cursor_rgba_set,
        "palette_len": colors.palette_len,
        "palette_prefix": colors.palette_prefix,
    })
}

fn canonical_style_json(style: &CanonicalStyle) -> Value {
    json!({
        "fg_rgba": style.fg_rgba,
        "bg_rgba": style.bg_rgba,
        "underline_rgba": style.underline_rgba,
        "flags": style.flags,
    })
}

fn canonical_rows_json(rows: &[CanonicalRow], index_field: &str) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| canonical_row_json(row, index_field))
            .collect(),
    )
}

fn canonical_row_json(row: &CanonicalRow, index_field: &str) -> Value {
    let mut row_json = json!({
        "text": row.text,
        "dirty_hash": row.dirty_hash,
        "row_state_hash": row.row_state_hash,
        "semantic_prompt": row.semantic_prompt,
        "dirty": row.dirty,
        "kitty_virtual_placeholder": row.kitty_virtual_placeholder,
        "runs": row.runs.iter().map(canonical_run_json).collect::<Vec<_>>(),
    });
    row_json
        .as_object_mut()
        .expect("canonical row json object")
        .insert(index_field.to_owned(), json!(row.index));
    row_json
}

fn canonical_run_json(run: &CanonicalRun) -> Value {
    json!({
        "text": run.text,
        "cell_widths": run.cell_widths,
        "style_id": run.style_id,
        "flags": run.flags,
        "hyperlink_id": run.hyperlink_id,
        "semantic_content": run.semantic_content,
    })
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

#[test]
fn renderer_equivalence_oracle_fixture_accepts_matching_canonical_json() {
    let dir = test_socket_path().with_extension("oracle");
    fs::create_dir_all(&dir).expect("create oracle dir");
    let actual = json!({
        "workspace": {
            "session_id": "local",
            "tab_id": "tab-1",
            "pane_id": "pane-1",
            "cols": 80,
            "rows": 24,
            "resize_policy": "fixed",
        },
        "terminal": {
            "title": "",
            "working_directory": "",
            "surface_kind": "main",
            "cursor": {
                "row": 0,
                "col": 0,
                "visible": true,
                "shape": "block",
                "blinking": true,
            },
            "modes": {
                "bracketed_paste": false,
                "mouse_tracking": false,
                "focus_reporting": false,
                "application_keypad": false,
                "application_cursor": false,
                "origin": false,
                "wraparound": true,
                "mouse_tracking_mode": "none",
                "mouse_format": "x10",
            },
        },
        "surface": {
            "colors": {
                "default_fg_rgba": 0,
                "default_bg_rgba": 0,
                "cursor_rgba": 0,
                "cursor_rgba_set": false,
                "palette_len": 0,
                "palette_prefix": [],
            },
            "styles": [],
            "row_updates": [],
        },
        "scrollback": [],
    });
    fs::write(
        dir.join("smoke.canonical.json"),
        serde_json::to_string_pretty(&actual).expect("encode oracle"),
    )
    .expect("write oracle");

    compare_oracle_fixture("smoke", &actual, &dir);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn renderer_equivalence_oracle_mismatch_reports_json_path() {
    let expected = json!({
        "surface": {
            "row_updates": [
                {
                    "runs": [
                        {
                            "text": "expected",
                        },
                    ],
                },
            ],
        },
    });
    let actual = json!({
        "surface": {
            "row_updates": [
                {
                    "runs": [
                        {
                            "text": "actual",
                        },
                    ],
                },
            ],
        },
    });

    assert_eq!(
        json_mismatch_path(&expected, &actual),
        "$.surface.row_updates[0].runs[0].text"
    );
}
