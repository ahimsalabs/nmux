use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_PATH_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn nmux_help_lists_live_client_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .arg("--help")
        .output()
        .expect("run nmux --help");

    assert!(
        output.status.success(),
        "nmux --help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"));
    assert!(stdout.contains("--connect-timeout-ms MS"));
    assert!(stdout.contains("--stdin-bytes"));
    assert!(stdout.contains("--local-echo off|tty"));
    assert!(stdout.contains("--cols COUNT"));
    assert!(stdout.contains("--redraw"));
    assert!(stdout.contains("interim text surface"));
    assert!(stdout.contains("not a VT-correct terminal emulator"));
    assert!(stdout.contains("Default socket: valid absolute $XDG_RUNTIME_DIR/nmux/nmuxd.sock"));
    assert!(stdout.contains("else /tmp/nmux-$UID/nmuxd.sock"));
    assert!(stdout.contains("Examples:"));
    assert!(stdout.contains("nmux --live --iterations 2"));
    assert!(stdout.contains("nmux --live --stdin-bytes --redraw"));
}

#[test]
fn nmuxd_help_lists_live_server_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .arg("--help")
        .output()
        .expect("run nmuxd --help");

    assert!(
        output.status.success(),
        "nmuxd --help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"));
    assert!(stdout.contains("--live-cycles COUNT"));
    assert!(stdout.contains("--live-forever"));
    assert!(stdout.contains("--live-clients COUNT"));
    assert!(stdout.contains("--resize-policy fixed|leader|active-client|manual"));
    assert!(stdout.contains("--terminal-engine interim"));
    assert!(stdout.contains("--command SHELL"));
    assert!(stdout.contains("Default socket: valid absolute $XDG_RUNTIME_DIR/nmux/nmuxd.sock"));
    assert!(stdout.contains("else /tmp/nmux-$UID/nmuxd.sock"));
    assert!(stdout.contains("Existing socket paths are not replaced automatically"));
    assert!(stdout.contains("only implemented terminal engine is the interim text surface"));
    assert!(stdout.contains("Examples:"));
    assert!(stdout.contains("nmuxd --one-shot"));
    assert!(stdout.contains("nmuxd --live"));
    assert!(stdout.contains("nmuxd --live-forever"));
    assert!(stdout.contains("nmuxd --live-clients 2"));
}

#[test]
fn nmux_rejects_live_only_flags_outside_live_mode() {
    assert_nmux_rejects(&["--stdin"], "nmux: --stdin requires --live");
    assert_nmux_rejects(&["--stdin-bytes"], "nmux: --stdin-bytes requires --live");
    assert_nmux_rejects(&["--redraw"], "nmux: --redraw requires --live");
    assert_nmux_rejects(
        &["--cols", "100", "--rows", "30"],
        "nmux: --cols and --rows require --live",
    );
}

#[test]
fn nmux_rejects_conflicting_frontend_modes() {
    assert_nmux_rejects(
        &["--live", "--follow"],
        "nmux: --follow cannot be combined with --live",
    );
    assert_nmux_rejects(
        &["--local-echo", "tty"],
        "nmux: --local-echo requires --stdin-bytes",
    );
    assert_nmux_rejects(
        &["--iterations", "1"],
        "nmux: --iterations requires --live or --follow",
    );
    assert_nmux_rejects(
        &["--live", "--iterations", "0"],
        "nmux: --iterations must be greater than 0",
    );
    assert_nmux_rejects(
        &["--live", "--interval-ms", "0"],
        "nmux: --interval-ms must be greater than 0",
    );
    assert_nmux_rejects(
        &["--connect-timeout-ms", "0", "--no-input"],
        "nmux: --connect-timeout-ms must be greater than 0",
    );
    assert_nmux_rejects(
        &["--scrollback-start", "0", "--no-input"],
        "nmux: --scrollback-start must be greater than 0",
    );
    assert_nmux_rejects(
        &["--scrollback-count", "0", "--no-input"],
        "nmux: --scrollback-count must be greater than 0",
    );
    assert_nmux_rejects(
        &["--live", "--cols", "0", "--rows", "24"],
        "nmux: --cols and --rows must be between 1 and 65535",
    );
    assert_nmux_rejects(
        &["--live", "--cols", "80", "--rows", "0"],
        "nmux: --cols and --rows must be between 1 and 65535",
    );
    assert_nmux_rejects(
        &["--live", "--cols", "65536", "--rows", "24"],
        "nmux: --cols and --rows must be between 1 and 65535",
    );
    assert_nmux_rejects(
        &["--live", "--cols", "80", "--rows", "65536"],
        "nmux: --cols and --rows must be between 1 and 65535",
    );
    assert_nmux_rejects(
        &["--live", "--key", "ping", "--stdin"],
        "nmux: --key cannot be combined with --stdin",
    );
    assert_nmux_rejects(
        &["--live", "--key", "ping", "--stdin-bytes"],
        "nmux: --key cannot be combined with --stdin-bytes",
    );
    assert_nmux_rejects(
        &["--live", "--no-input", "--stdin"],
        "nmux: --no-input cannot be combined with --stdin",
    );
    assert_nmux_rejects(
        &["--live", "--no-input", "--stdin-bytes"],
        "nmux: --no-input cannot be combined with --stdin-bytes",
    );
    assert_nmux_rejects(
        &["--key", "ping", "--no-input"],
        "nmux: --key cannot be combined with --no-input",
    );
}

#[test]
fn nmuxd_rejects_conflicting_server_modes() {
    assert_nmuxd_rejects(
        &["--one-shot", "--live-clients", "2"],
        "nmuxd: --one-shot cannot be combined with live daemon modes",
    );
    assert_nmuxd_rejects(
        &["--one-shot", "--live-forever"],
        "nmuxd: --one-shot cannot be combined with live daemon modes",
    );
    assert_nmuxd_rejects(
        &["--live", "--live-cycles", "1"],
        "nmuxd: --live cannot be combined with --live-forever, --live-cycles, or --live-clients",
    );
    assert_nmuxd_rejects(
        &["--live", "--live-forever"],
        "nmuxd: --live cannot be combined with --live-forever, --live-cycles, or --live-clients",
    );
    assert_nmuxd_rejects(
        &["--live-forever", "--live-clients", "2"],
        "nmuxd: --live-forever cannot be combined with --live-cycles or --live-clients",
    );
    assert_nmuxd_rejects(
        &["--live-cycles", "0"],
        "nmuxd: --live-cycles must be greater than 0",
    );
    assert_nmuxd_rejects(
        &["--live-clients", "0"],
        "nmuxd: --live-clients must be greater than 0",
    );
    assert_nmuxd_rejects(
        &["--terminal-engine", "libghostty-vt"],
        "nmuxd: --terminal-engine requires interim",
    );
}

#[test]
fn nmuxd_reports_existing_socket_path() {
    let socket_path = test_socket_path();
    fs::write(&socket_path, "not a socket").expect("write placeholder");

    let output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
        ])
        .output()
        .expect("run nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(!output.status.success(), "nmuxd unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nmuxd: socket path already exists"),
        "missing existing socket context:\n{stderr}"
    );
    assert!(
        stderr.contains(socket_path.to_str().expect("socket path")),
        "missing socket path:\n{stderr}"
    );
    assert!(
        stderr.contains("pass --socket PATH"),
        "missing recovery hint:\n{stderr}"
    );
}

#[test]
fn nmuxd_reports_socket_path_when_bind_fails() {
    let socket_path = std::env::temp_dir().join(format!("nmux-{}.sock", "x".repeat(160)));

    let output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
        ])
        .output()
        .expect("run nmuxd");

    assert!(!output.status.success(), "nmuxd unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nmuxd: failed to bind nmux daemon socket at"),
        "missing bind context:\n{stderr}"
    );
    assert!(
        stderr.contains(socket_path.to_str().expect("socket path")),
        "missing socket path:\n{stderr}"
    );
}

#[test]
fn nmux_reports_state_load_path_before_connecting() {
    let state_path = test_state_path();
    fs::write(&state_path, "not nmux state\n").expect("write bad state");

    let output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--state",
            state_path.to_str().expect("state path"),
            "--no-input",
        ])
        .output()
        .expect("run nmux");
    let _ = fs::remove_file(&state_path);

    assert!(!output.status.success(), "nmux unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nmux: failed to load client state"),
        "missing state-load context:\n{stderr}"
    );
    assert!(
        stderr.contains(state_path.to_str().expect("state path")),
        "missing state path:\n{stderr}"
    );
    assert!(
        stderr.contains("invalid nmux client state header"),
        "missing parser error:\n{stderr}"
    );
}

#[test]
fn nmux_reports_socket_path_when_daemon_is_missing() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--no-input",
        ])
        .output()
        .expect("run nmux");

    assert!(!output.status.success(), "nmux unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nmux: failed to connect to nmux daemon at"),
        "missing connect context:\n{stderr}"
    );
    assert!(
        stderr.contains(socket_path.to_str().expect("socket path")),
        "missing socket path:\n{stderr}"
    );
}

fn assert_nmux_rejects(args: &[&str], expected_stderr: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args(args)
        .output()
        .expect("run nmux");

    assert!(
        !output.status.success(),
        "nmux unexpectedly succeeded for args {args:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected_stderr),
        "missing expected error {expected_stderr:?} for args {args:?}:\n{stderr}"
    );
}

fn assert_nmuxd_rejects(args: &[&str], expected_stderr: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args(args)
        .output()
        .expect("run nmuxd");

    assert!(
        !output.status.success(),
        "nmuxd unexpectedly succeeded for args {args:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected_stderr),
        "missing expected error {expected_stderr:?} for args {args:?}:\n{stderr}"
    );
}

fn test_state_path() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let id = NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "nmux-cli-help-state-{}-{nanos}-{id}.state",
        std::process::id()
    ))
}

fn test_socket_path() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let id = NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "nmux-cli-help-socket-{}-{nanos}-{id}.sock",
        std::process::id()
    ))
}
