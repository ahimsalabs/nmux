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
    assert!(stdout.contains("--resize-policy fixed|leader|active-client|manual"));
    assert!(stdout.contains("--command SHELL"));
}
