use std::process::Command;

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
