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
    assert!(stdout.contains("--stdin-bytes"));
    assert!(stdout.contains("--local-echo off|tty"));
    assert!(stdout.contains("--cols COUNT"));
    assert!(stdout.contains("--redraw"));
    assert!(stdout.contains("interim text surface"));
    assert!(stdout.contains("not a VT-correct terminal emulator"));
    assert!(stdout.contains("Examples:"));
    assert!(stdout.contains("nmux --socket /tmp/nmux.sock --live --iterations 2"));
    assert!(stdout.contains("nmux --socket /tmp/nmux.sock --live --stdin-bytes --redraw"));
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
    assert!(stdout.contains("--live-clients COUNT"));
    assert!(stdout.contains("--resize-policy fixed|leader|active-client|manual"));
    assert!(stdout.contains("--command SHELL"));
    assert!(stdout.contains("Examples:"));
    assert!(stdout.contains("nmuxd --socket /tmp/nmux.sock --one-shot"));
    assert!(stdout.contains("nmuxd --socket /tmp/nmux.sock --live"));
    assert!(stdout.contains("nmuxd --socket /tmp/nmux.sock --live-clients 2"));
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
        &["--live", "--live-cycles", "1"],
        "nmuxd: --live cannot be combined with --live-cycles or --live-clients",
    );
    assert_nmuxd_rejects(
        &["--live-cycles", "0"],
        "nmuxd: --live-cycles must be greater than 0",
    );
    assert_nmuxd_rejects(
        &["--live-clients", "0"],
        "nmuxd: --live-clients must be greater than 0",
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
