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
    assert!(stdout.contains("--startup-timeout-ms MS"));
    assert!(stdout.contains("--print-context"));
    assert!(stdout.contains("--print-context-json"));
    assert!(stdout.contains("--print-socket"));
    assert!(stdout.contains("--print-socket-json"));
    assert!(stdout.contains("--tcp HOST:PORT"));
    assert!(stdout.contains("--tcp-token TOKEN"));
    assert!(stdout.contains("-V, --version"));
    assert!(stdout.contains("--version-json"));
    assert!(stdout.contains("--key-name NAME"));
    assert!(stdout.contains("Send a supported named key"));
    assert!(stdout.contains("--list-key-names"));
    assert!(stdout.contains("--list-key-names-json"));
    assert!(stdout.contains("--list-input-choices-json"));
    assert!(stdout.contains("--json"));
    assert!(stdout.contains("Print attach output as JSON; live uses JSON lines"));
    assert!(stdout.contains("--json emits one object per one-shot/follow attach"));
    assert!(stdout.contains("--paste TEXT"));
    assert!(stdout.contains("--focus gained|lost"));
    assert!(stdout.contains("daemon rejects if reporting is off"));
    assert!(stdout.contains("--stdin-bytes"));
    assert!(stdout.contains("--local-echo off|tty"));
    assert!(stdout.contains("--detach-key ctrl-]|none"));
    assert!(stdout.contains("--detach-key none passes it through"));
    assert!(stdout.contains("--no-scrollback"));
    assert!(stdout.contains("--record PATH"));
    assert!(stdout.contains("timestamped live JSON events"));
    assert!(stdout.contains("--cols COUNT"));
    assert!(stdout.contains("Live ResizeIntent columns; both dimensions required"));
    assert!(stdout.contains("--mouse-modifiers MODS"));
    assert!(stdout.contains("--redraw"));
    assert!(stdout.contains("--start"));
    assert!(stdout.contains("--shell"));
    assert!(stdout.contains("--command SHELL"));
    assert!(stdout.contains("--cwd DIR"));
    assert!(stdout.contains("Existing pane working directory"));
    assert!(stdout.contains("--env KEY=VALUE"));
    assert!(stdout.contains("--start waits for nmuxd --ready-json"));
    assert!(stdout.contains("--startup-timeout-ms controls that managed readiness wait"));
    assert!(stdout.contains("Without an explicit input or resize flag"));
    assert!(stdout.contains("interim text surface"));
    assert!(
        stdout.contains("Informational flags exit before mode validation or socket/state work")
    );
    assert!(stdout.contains("NMUX_ORIGIN records the local hop chain"));
    assert!(stdout.contains("not a VT-correct terminal emulator"));
    assert!(stdout.contains("Default socket: --socket, else valid absolute $NMUX_SOCKET"));
    assert!(stdout.contains("valid absolute $XDG_RUNTIME_DIR/nmux/nmuxd.sock"));
    assert!(stdout.contains("else /tmp/nmux-$UID/nmuxd.sock"));
    assert!(stdout.contains("Examples:"));
    assert!(stdout.contains("nmux --live --iterations 2"));
    assert!(stdout.contains("nmux --live --cols 100 --rows 30"));
    assert!(stdout.contains("nmux --live --no-input"));
    assert!(stdout.contains("nmux --live --stdin-bytes --redraw"));
    assert!(stdout.contains("nmux --shell"));
    assert!(stdout.contains("nmux replay PATH"));
    assert!(stdout.contains("Print surface frames from a recorded live session"));
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
    assert!(stdout.contains("--tcp-listen HOST:PORT"));
    assert!(stdout.contains("--tcp-token TOKEN"));
    assert!(stdout.contains("--print-socket"));
    assert!(stdout.contains("--print-socket-json"));
    assert!(stdout.contains("--ready-json"));
    assert!(stdout.contains("--list-daemon-choices-json"));
    assert!(stdout.contains("-V, --version"));
    assert!(stdout.contains("--version-json"));
    assert!(stdout.contains("--live-forever"));
    assert!(stdout.contains("--live-clients COUNT"));
    assert!(stdout.contains("--resize-policy fixed|leader|active-client|manual"));
    assert!(stdout.contains("--terminal-engine interim|libghostty-vt"));
    assert!(stdout.contains("--command SHELL"));
    assert!(stdout.contains("Run the pane command from existing DIR"));
    assert!(stdout.contains("Default socket: --socket, else valid absolute $NMUX_SOCKET"));
    assert!(stdout.contains("valid absolute $XDG_RUNTIME_DIR/nmux/nmuxd.sock"));
    assert!(stdout.contains("else /tmp/nmux-$UID/nmuxd.sock"));
    assert!(
        stdout
            .contains("Informational flags exit before daemon-mode validation or socket/PTY work")
    );
    assert!(stdout.contains("--ready-json does not exit"));
    assert!(stdout.contains("Existing socket paths are not replaced automatically"));
    assert!(stdout.contains("NMUX_ORIGIN is appended for child pane commands"));
    assert!(stdout.contains("libghostty-vt requires building nmux"));
    assert!(stdout.contains("Examples:"));
    assert!(stdout.contains("nmuxd --one-shot"));
    assert!(stdout.contains("nmuxd --live"));
    assert!(stdout.contains("nmuxd --live-forever"));
    assert!(stdout.contains("nmuxd --live-clients 2"));
}

#[test]
fn version_flags_report_binary_versions_without_side_effects() {
    let client_output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .arg("--version")
        .output()
        .expect("run nmux --version");

    assert!(
        client_output.status.success(),
        "nmux --version failed: {}",
        String::from_utf8_lossy(&client_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&client_output.stdout).trim(),
        concat!("nmux ", env!("CARGO_PKG_VERSION"))
    );

    let daemon_socket_path = test_socket_path();
    let daemon_output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            daemon_socket_path.to_str().expect("socket path"),
            "--version",
        ])
        .output()
        .expect("run nmuxd --version");

    assert!(
        daemon_output.status.success(),
        "nmuxd --version failed: {}",
        String::from_utf8_lossy(&daemon_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&daemon_output.stdout).trim(),
        concat!("nmuxd ", env!("CARGO_PKG_VERSION"))
    );
    assert!(
        !daemon_socket_path.exists(),
        "nmuxd --version should not bind a socket path"
    );
}

#[test]
fn version_json_flags_report_binary_versions_without_side_effects() {
    let client_output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .arg("--version-json")
        .output()
        .expect("run nmux --version-json");

    assert!(
        client_output.status.success(),
        "nmux --version-json failed: {}",
        String::from_utf8_lossy(&client_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&client_output.stdout).trim(),
        format!(
            "{{\"binary\":\"nmux\",\"version\":\"{}\"}}",
            env!("CARGO_PKG_VERSION")
        )
    );

    let daemon_socket_path = test_socket_path();
    let daemon_output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            daemon_socket_path.to_str().expect("socket path"),
            "--version-json",
        ])
        .output()
        .expect("run nmuxd --version-json");

    assert!(
        daemon_output.status.success(),
        "nmuxd --version-json failed: {}",
        String::from_utf8_lossy(&daemon_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&daemon_output.stdout).trim(),
        format!(
            "{{\"binary\":\"nmuxd\",\"version\":\"{}\"}}",
            env!("CARGO_PKG_VERSION")
        )
    );
    assert!(
        !daemon_socket_path.exists(),
        "nmuxd --version-json should not bind a socket path"
    );
}

#[test]
fn print_context_reports_inherited_nmux_context_without_connecting() {
    let socket_path = test_socket_path();
    let output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .arg("--print-context")
        .env("NMUX", "1")
        .env("NMUX_SESSION_ID", "session-7")
        .env("NMUX_PANE_ID", "pane-3")
        .env("NMUX_SOCKET", &socket_path)
        .env("NMUX_ORIGIN", "local")
        .output()
        .expect("run nmux --print-context");

    assert!(
        output.status.success(),
        "nmux --print-context failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("NMUX=1"));
    assert!(stdout.contains("NMUX_SESSION_ID=session-7"));
    assert!(stdout.contains("NMUX_PANE_ID=pane-3"));
    assert!(stdout.contains(&format!("NMUX_SOCKET={}", socket_path.display())));
    assert!(stdout.contains("NMUX_ORIGIN=local"));
    assert!(
        !socket_path.exists(),
        "nmux --print-context should not connect or create a socket path"
    );
}

#[test]
fn print_context_json_reports_inherited_nmux_context_without_connecting() {
    let socket_path = test_socket_path();
    let output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .arg("--print-context-json")
        .env("NMUX", "1")
        .env("NMUX_SESSION_ID", "session-7")
        .env("NMUX_PANE_ID", "pane-3")
        .env("NMUX_SOCKET", &socket_path)
        .env("NMUX_ORIGIN", "local")
        .output()
        .expect("run nmux --print-context-json");

    assert!(
        output.status.success(),
        "nmux --print-context-json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!(
            "{{\"NMUX\":\"1\",\"NMUX_SESSION_ID\":\"session-7\",\"NMUX_PANE_ID\":\"pane-3\",\"NMUX_SOCKET\":\"{}\",\"NMUX_ORIGIN\":\"local\"}}",
            socket_path.display()
        )
    );
    assert!(
        !socket_path.exists(),
        "nmux --print-context-json should not connect or create a socket path"
    );
}

#[test]
fn print_context_rejects_missing_nmux_context() {
    let output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .arg("--print-context")
        .env_remove("NMUX")
        .env_remove("NMUX_SESSION_ID")
        .env_remove("NMUX_PANE_ID")
        .env_remove("NMUX_SOCKET")
        .env_remove("NMUX_ORIGIN")
        .output()
        .expect("run nmux --print-context");

    assert!(!output.status.success(), "nmux unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nmux: not running inside an nmux pane"),
        "missing context error:\n{stderr}"
    );
}

#[test]
fn print_context_json_reports_missing_nmux_context_as_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .arg("--print-context-json")
        .env_remove("NMUX")
        .env_remove("NMUX_SESSION_ID")
        .env_remove("NMUX_PANE_ID")
        .env_remove("NMUX_SOCKET")
        .env_remove("NMUX_ORIGIN")
        .output()
        .expect("run nmux --print-context-json");

    assert!(!output.status.success(), "nmux unexpectedly succeeded");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"error\"")
            && stdout.contains("not running inside an nmux pane")
            && stdout.contains("NMUX=1 is not set"),
        "missing JSON context error:\n{stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nmux: not running inside an nmux pane"),
        "missing stderr context error:\n{stderr}"
    );
}

#[test]
fn no_connect_client_flags_skip_attach_mode_validation() {
    let socket_path = test_socket_path();
    let context_output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--print-context",
            "--stdin",
            "--stdin-bytes",
            "--cols",
            "100",
            "--iterations",
            "0",
        ])
        .env("NMUX", "1")
        .env("NMUX_SESSION_ID", "session-7")
        .env("NMUX_PANE_ID", "pane-3")
        .env("NMUX_SOCKET", &socket_path)
        .env("NMUX_ORIGIN", "local")
        .output()
        .expect("run nmux --print-context with attach flags");

    assert!(
        context_output.status.success(),
        "nmux --print-context should exit before attach-mode validation: {}",
        String::from_utf8_lossy(&context_output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&context_output.stdout).contains("NMUX_PANE_ID=pane-3"),
        "missing context output"
    );

    let socket_output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--print-socket",
            "--redraw",
            "--cols",
            "100",
            "--iterations",
            "0",
        ])
        .output()
        .expect("run nmux --print-socket with attach flags");

    assert!(
        socket_output.status.success(),
        "nmux --print-socket should exit before attach-mode validation: {}",
        String::from_utf8_lossy(&socket_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&socket_output.stdout).trim(),
        socket_path.to_str().expect("socket path")
    );
    assert!(
        !socket_path.exists(),
        "no-connect flags should not create a socket path"
    );

    let json_socket_path = test_socket_path();
    let json_socket_output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            json_socket_path.to_str().expect("socket path"),
            "--print-socket-json",
            "--redraw",
            "--cols",
            "100",
            "--iterations",
            "0",
        ])
        .output()
        .expect("run nmux --print-socket-json with attach flags");

    assert!(
        json_socket_output.status.success(),
        "nmux --print-socket-json should exit before attach-mode validation: {}",
        String::from_utf8_lossy(&json_socket_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&json_socket_output.stdout).trim(),
        format!(
            "{{\"NMUX_SOCKET\":\"{}\",\"source\":\"--socket\"}}",
            json_socket_path.display()
        )
    );
    assert!(
        !json_socket_path.exists(),
        "json no-connect flags should not create a socket path"
    );

    let key_names_output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--list-key-names-json",
            "--cols",
            "100",
            "--iterations",
            "0",
        ])
        .output()
        .expect("run nmux --list-key-names-json with attach flags");

    assert!(
        key_names_output.status.success(),
        "nmux --list-key-names-json should exit before attach-mode validation: {}",
        String::from_utf8_lossy(&key_names_output.stderr)
    );
    let key_names_stdout = String::from_utf8_lossy(&key_names_output.stdout);
    assert!(key_names_stdout.contains("\"names\""));
    assert!(key_names_stdout.contains("\"numpad-enter\""));
    assert!(key_names_stdout.contains("{\"alias\":\"esc\",\"canonical\":\"escape\"}"));
    assert!(
        !socket_path.exists(),
        "key-name list flags should not create a socket path"
    );

    let input_choices_output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--list-input-choices-json",
            "--cols",
            "100",
            "--iterations",
            "0",
        ])
        .output()
        .expect("run nmux --list-input-choices-json with attach flags");

    assert!(
        input_choices_output.status.success(),
        "nmux --list-input-choices-json should exit before attach-mode validation: {}",
        String::from_utf8_lossy(&input_choices_output.stderr)
    );
    let input_choices_stdout = String::from_utf8_lossy(&input_choices_output.stdout);
    assert!(input_choices_stdout.contains("\"key_modifiers\""));
    assert!(input_choices_stdout.contains("\"focus_events\":[\"gained\",\"lost\"]"));
    assert!(input_choices_stdout.contains("\"mouse_buttons\""));
    assert!(
        !socket_path.exists(),
        "input-choice list flags should not create a socket path"
    );
}

#[test]
fn no_bind_daemon_flags_skip_daemon_mode_validation() {
    let socket_path = test_socket_path();
    let socket_output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--print-socket",
            "--one-shot",
            "--live-forever",
            "--live-clients",
            "0",
        ])
        .output()
        .expect("run nmuxd --print-socket with daemon flags");

    assert!(
        socket_output.status.success(),
        "nmuxd --print-socket should exit before daemon-mode validation: {}",
        String::from_utf8_lossy(&socket_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&socket_output.stdout).trim(),
        socket_path.to_str().expect("socket path")
    );
    assert!(
        !socket_path.exists(),
        "nmuxd no-bind flags should not bind a socket path"
    );

    let json_socket_path = test_socket_path();
    let json_socket_output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            json_socket_path.to_str().expect("socket path"),
            "--print-socket-json",
            "--one-shot",
            "--live-forever",
            "--live-clients",
            "0",
        ])
        .output()
        .expect("run nmuxd --print-socket-json with daemon flags");

    assert!(
        json_socket_output.status.success(),
        "nmuxd --print-socket-json should exit before daemon-mode validation: {}",
        String::from_utf8_lossy(&json_socket_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&json_socket_output.stdout).trim(),
        format!(
            "{{\"NMUX_SOCKET\":\"{}\",\"source\":\"--socket\"}}",
            json_socket_path.display()
        )
    );
    assert!(
        !json_socket_path.exists(),
        "nmuxd json no-bind flags should not bind a socket path"
    );

    let choices_socket_path = test_socket_path();
    let choices_output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            choices_socket_path.to_str().expect("socket path"),
            "--list-daemon-choices-json",
            "--one-shot",
            "--live-forever",
            "--live-clients",
            "0",
        ])
        .output()
        .expect("run nmuxd --list-daemon-choices-json with daemon flags");

    assert!(
        choices_output.status.success(),
        "nmuxd --list-daemon-choices-json should exit before daemon-mode validation: {}",
        String::from_utf8_lossy(&choices_output.stderr)
    );
    let choices_stdout = String::from_utf8_lossy(&choices_output.stdout);
    assert!(choices_stdout.contains("\"resize_policies\""));
    assert!(choices_stdout.contains("\"terminal_engines\""));
    assert!(choices_stdout.contains("{\"name\":\"interim\",\"available\":true}"));
    assert!(
        !choices_socket_path.exists(),
        "nmuxd daemon-choice list flags should not bind a socket path"
    );

    let version_socket_path = test_socket_path();
    let version_output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            version_socket_path.to_str().expect("socket path"),
            "--version",
            "--one-shot",
            "--live",
        ])
        .output()
        .expect("run nmuxd --version with daemon flags");

    assert!(
        version_output.status.success(),
        "nmuxd --version should exit before daemon-mode validation: {}",
        String::from_utf8_lossy(&version_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&version_output.stdout).trim(),
        concat!("nmuxd ", env!("CARGO_PKG_VERSION"))
    );
    assert!(
        !version_socket_path.exists(),
        "nmuxd --version should not bind a socket path"
    );
}

#[test]
fn print_socket_reports_resolved_socket_without_side_effects() {
    let env_socket_path = test_socket_path();
    let client_env_output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .arg("--print-socket")
        .env("NMUX_SOCKET", &env_socket_path)
        .output()
        .expect("run nmux --print-socket with NMUX_SOCKET");

    assert!(
        client_env_output.status.success(),
        "nmux --print-socket with NMUX_SOCKET failed: {}",
        String::from_utf8_lossy(&client_env_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&client_env_output.stdout).trim(),
        env_socket_path.to_str().expect("socket path")
    );
    assert!(
        !env_socket_path.exists(),
        "nmux --print-socket should not create an NMUX_SOCKET path"
    );

    let daemon_env_output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .arg("--print-socket")
        .env("NMUX_SOCKET", &env_socket_path)
        .output()
        .expect("run nmuxd --print-socket with NMUX_SOCKET");

    assert!(
        daemon_env_output.status.success(),
        "nmuxd --print-socket with NMUX_SOCKET failed: {}",
        String::from_utf8_lossy(&daemon_env_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&daemon_env_output.stdout).trim(),
        env_socket_path.to_str().expect("socket path")
    );
    assert!(
        !env_socket_path.exists(),
        "nmuxd --print-socket should not bind an NMUX_SOCKET path"
    );

    let client_socket_path = test_socket_path();
    let client_env_override_path = test_socket_path();
    let client_output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            client_socket_path.to_str().expect("socket path"),
            "--print-socket",
        ])
        .env("NMUX_SOCKET", &client_env_override_path)
        .output()
        .expect("run nmux --print-socket");

    assert!(
        client_output.status.success(),
        "nmux --print-socket failed: {}",
        String::from_utf8_lossy(&client_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&client_output.stdout).trim(),
        client_socket_path.to_str().expect("socket path")
    );
    assert!(
        !client_socket_path.exists(),
        "nmux --print-socket should not create a socket path"
    );
    assert!(
        !client_env_override_path.exists(),
        "explicit --socket should win without touching NMUX_SOCKET"
    );

    let daemon_socket_path = test_socket_path();
    let daemon_env_override_path = test_socket_path();
    let daemon_output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            daemon_socket_path.to_str().expect("socket path"),
            "--print-socket",
        ])
        .env("NMUX_SOCKET", &daemon_env_override_path)
        .output()
        .expect("run nmuxd --print-socket");

    assert!(
        daemon_output.status.success(),
        "nmuxd --print-socket failed: {}",
        String::from_utf8_lossy(&daemon_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&daemon_output.stdout).trim(),
        daemon_socket_path.to_str().expect("socket path")
    );
    assert!(
        !daemon_socket_path.exists(),
        "nmuxd --print-socket should not bind a socket path"
    );
    assert!(
        !daemon_env_override_path.exists(),
        "explicit --socket should win without touching NMUX_SOCKET"
    );
}

#[test]
fn print_socket_json_reports_resolved_socket_source_without_side_effects() {
    let env_socket_path = test_socket_path();
    let client_env_output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .arg("--print-socket-json")
        .env("NMUX_SOCKET", &env_socket_path)
        .output()
        .expect("run nmux --print-socket-json with NMUX_SOCKET");

    assert!(
        client_env_output.status.success(),
        "nmux --print-socket-json with NMUX_SOCKET failed: {}",
        String::from_utf8_lossy(&client_env_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&client_env_output.stdout).trim(),
        format!(
            "{{\"NMUX_SOCKET\":\"{}\",\"source\":\"NMUX_SOCKET\"}}",
            env_socket_path.display()
        )
    );
    assert!(
        !env_socket_path.exists(),
        "nmux --print-socket-json should not create an NMUX_SOCKET path"
    );

    let daemon_env_output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .arg("--print-socket-json")
        .env("NMUX_SOCKET", &env_socket_path)
        .output()
        .expect("run nmuxd --print-socket-json with NMUX_SOCKET");

    assert!(
        daemon_env_output.status.success(),
        "nmuxd --print-socket-json with NMUX_SOCKET failed: {}",
        String::from_utf8_lossy(&daemon_env_output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&daemon_env_output.stdout).trim(),
        format!(
            "{{\"NMUX_SOCKET\":\"{}\",\"source\":\"NMUX_SOCKET\"}}",
            env_socket_path.display()
        )
    );
    assert!(
        !env_socket_path.exists(),
        "nmuxd --print-socket-json should not bind an NMUX_SOCKET path"
    );
}

#[test]
fn nmux_rejects_live_only_flags_outside_live_mode() {
    let missing_cwd = test_socket_path();
    assert_nmux_rejects(&["--stdin"], "nmux: --stdin requires --live");
    assert_nmux_rejects(&["--stdin-bytes"], "nmux: --stdin-bytes requires --live");
    assert_nmux_rejects(&["--redraw"], "nmux: --redraw requires --live");
    assert_nmux_rejects(
        &["--command", "printf hi"],
        "nmux: --command requires --start",
    );
    assert_nmux_rejects(&["--cwd", "/tmp"], "nmux: --cwd requires --start");
    assert_nmux_rejects(&["--env", "NMUX_DEMO=1"], "nmux: --env requires --start");
    assert_nmux_rejects(
        &["--startup-timeout-ms", "100"],
        "nmux: --startup-timeout-ms requires --start",
    );
    assert_nmux_rejects(
        &[
            "--start",
            "--cwd",
            missing_cwd.to_str().expect("missing cwd path"),
        ],
        "nmux: --cwd must be an existing directory",
    );
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
        &["--start", "--follow"],
        "nmux: --start cannot be combined with --follow",
    );
    assert_nmux_rejects(
        &["--follow", "--key", "ping"],
        "nmux: --follow cannot be combined with --key",
    );
    assert_nmux_rejects(
        &["--follow", "--paste", "clip"],
        "nmux: --follow cannot be combined with --paste",
    );
    assert_nmux_rejects(
        &["--follow", "--key-name", "delete"],
        "nmux: --follow cannot be combined with --key-name",
    );
    assert_nmux_rejects(
        &["--follow", "--focus", "gained"],
        "nmux: --follow cannot be combined with --focus",
    );
    assert_nmux_rejects(
        &["--follow", "--mouse", "press:left:1:1"],
        "nmux: --follow cannot be combined with --mouse",
    );
    assert_nmux_rejects(
        &["--local-echo", "tty"],
        "nmux: --local-echo requires --stdin-bytes",
    );
    assert_nmux_rejects(
        &["--detach-key", "none"],
        "nmux: --detach-key requires --stdin-bytes",
    );
    assert_nmux_rejects(
        &["--record", "/tmp/nmux.record"],
        "nmux: --record requires --live",
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
        &["--start", "--startup-timeout-ms", "0"],
        "nmux: --startup-timeout-ms must be greater than 0",
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
        &["--no-scrollback", "--scrollback-tail", "5"],
        "nmux: --no-scrollback cannot be combined with --scrollback-tail",
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
        &["--live", "--no-input", "--cols", "80", "--rows", "24"],
        "nmux: --no-input cannot be combined with --cols/--rows",
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
        &["--live", "--paste", "clip", "--stdin"],
        "nmux: --paste cannot be combined with --stdin",
    );
    assert_nmux_rejects(
        &["--live", "--paste", "clip", "--stdin-bytes"],
        "nmux: --paste cannot be combined with --stdin-bytes",
    );
    assert_nmux_rejects(
        &["--live", "--focus", "gained", "--stdin"],
        "nmux: --focus cannot be combined with --stdin",
    );
    assert_nmux_rejects(
        &["--live", "--focus", "gained", "--stdin-bytes"],
        "nmux: --focus cannot be combined with --stdin-bytes",
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
    assert_nmux_rejects(
        &["--key", "ping", "--paste", "clip"],
        "nmux: --key cannot be combined with --paste",
    );
    assert_nmux_rejects(
        &["--key", "ping", "--key-name", "keypad-enter"],
        "nmux: --key cannot be combined with --key-name",
    );
    assert_nmux_rejects(
        &["--paste", "clip", "--no-input"],
        "nmux: --paste cannot be combined with --no-input",
    );
    assert_nmux_rejects(
        &["--live", "--key-name", "f13"],
        "nmux: --key-name requires a supported named key",
    );
    assert_nmux_rejects(
        &["--live", "--focus", "blurred"],
        "nmux: invalid value 'blurred' for '--focus <gained|lost>'",
    );
    assert_nmux_rejects(
        &["--live", "--key", "ping", "--focus", "gained"],
        "nmux: --key cannot be combined with --focus",
    );
    assert_nmux_rejects(
        &["--live", "--paste", "clip", "--focus", "gained"],
        "nmux: --paste cannot be combined with --focus",
    );
    assert_nmux_rejects(
        &["--live", "--key-name", "keypad-enter", "--focus", "gained"],
        "nmux: --key-name cannot be combined with --focus",
    );
    assert_nmux_rejects(
        &["--live", "--key-name", "keypad-enter", "--paste", "clip"],
        "nmux: --key-name cannot be combined with --paste",
    );
    assert_nmux_rejects(
        &["--live", "--focus", "gained", "--no-input"],
        "nmux: --focus cannot be combined with --no-input",
    );
    assert_nmux_rejects(
        &["--live", "--key-name", "keypad-enter", "--no-input"],
        "nmux: --key-name cannot be combined with --no-input",
    );
}

#[test]
fn nmuxd_rejects_conflicting_server_modes() {
    let missing_cwd = test_socket_path();
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
        &["--live-cycles", "many"],
        "nmuxd: --live-cycles requires a valid number",
    );
    assert_nmuxd_rejects(
        &["--live-clients", "many"],
        "nmuxd: --live-clients requires a valid number",
    );
    assert_nmuxd_rejects(
        &[
            "--cwd",
            missing_cwd.to_str().expect("missing cwd path"),
            "--one-shot",
        ],
        "nmuxd: --cwd must be an existing directory",
    );
    #[cfg(not(feature = "libghostty-vt"))]
    {
        assert_nmuxd_rejects(
            &["--terminal-engine", "libghostty-vt"],
            "nmuxd: --terminal-engine libghostty-vt requires the libghostty-vt feature",
        );
    }
}

#[test]
fn parse_errors_use_terse_project_prefixes() {
    assert_nmux_rejects(&["--bogus"], "nmux: unexpected argument '--bogus' found");
    assert_nmux_rejects(
        &["--socket"],
        "nmux: a value is required for '--socket <PATH>'",
    );
    assert_nmuxd_rejects(&["--bogus"], "nmuxd: unexpected argument '--bogus' found");
    assert_nmuxd_rejects(
        &["--socket"],
        "nmuxd: a value is required for '--socket <PATH>'",
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
fn nmuxd_ready_json_reports_existing_socket_error() {
    let socket_path = test_socket_path();
    fs::write(&socket_path, "not a socket").expect("write placeholder");

    let output = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--ready-json",
            "--one-shot",
        ])
        .output()
        .expect("run nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(!output.status.success(), "nmuxd unexpectedly succeeded");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"event\":\"error\""),
        "missing ready-json error event:\n{stdout}"
    );
    assert!(
        stdout.contains("socket path already exists"),
        "missing ready-json error message:\n{stdout}"
    );
    assert!(
        stdout.contains(socket_path.to_str().expect("socket path")),
        "missing socket path:\n{stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nmuxd: socket path already exists"),
        "missing stderr error:\n{stderr}"
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
fn nmux_json_reports_state_load_setup_error() {
    let state_path = test_state_path();
    fs::write(&state_path, "not nmux state\n").expect("write bad state");

    let output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--state",
            state_path.to_str().expect("state path"),
            "--json",
            "--no-input",
        ])
        .output()
        .expect("run nmux --json");
    let _ = fs::remove_file(&state_path);

    assert!(!output.status.success(), "nmux unexpectedly succeeded");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"error\""),
        "missing JSON error object:\n{stdout}"
    );
    assert!(
        stdout.contains("\"message\":\"failed to load client state")
            && stdout.contains(state_path.to_str().expect("state path"))
            && stdout.contains("invalid nmux client state header"),
        "missing state-load JSON error context:\n{stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nmux: failed to load client state"),
        "missing stderr state-load context:\n{stderr}"
    );
}

#[test]
fn state_info_json_reports_setup_errors_as_json() {
    let missing_state = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .arg("--state-info-json")
        .output()
        .expect("run nmux --state-info-json");

    assert!(
        !missing_state.status.success(),
        "nmux unexpectedly succeeded"
    );
    let stdout = String::from_utf8_lossy(&missing_state.stdout);
    assert!(
        stdout.contains("\"error\"") && stdout.contains("--state-info requires --state PATH"),
        "missing JSON missing-state error:\n{stdout}"
    );

    let state_path = test_state_path();
    fs::write(&state_path, "not nmux state\n").expect("write bad state");
    let corrupt_state = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--state",
            state_path.to_str().expect("state path"),
            "--state-info-json",
        ])
        .output()
        .expect("run nmux --state-info-json");
    let _ = fs::remove_file(&state_path);

    assert!(
        !corrupt_state.status.success(),
        "nmux unexpectedly succeeded"
    );
    let stdout = String::from_utf8_lossy(&corrupt_state.stdout);
    assert!(
        stdout.contains("\"message\":\"failed to load client state")
            && stdout.contains(state_path.to_str().expect("state path"))
            && stdout.contains("invalid nmux client state header"),
        "missing JSON corrupt-state error:\n{stdout}"
    );
}

#[test]
fn nmux_live_json_reports_state_load_setup_error() {
    let state_path = test_state_path();
    fs::write(&state_path, "not nmux state\n").expect("write bad state");

    let output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--json",
            "--no-input",
        ])
        .output()
        .expect("run nmux --live --json");
    let _ = fs::remove_file(&state_path);

    assert!(!output.status.success(), "nmux unexpectedly succeeded");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"event\":\"error\""),
        "missing JSON error event:\n{stdout}"
    );
    assert!(
        stdout.contains("\"message\":\"failed to load client state")
            && stdout.contains(state_path.to_str().expect("state path"))
            && stdout.contains("invalid nmux client state header"),
        "missing state-load JSON error context:\n{stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nmux: failed to load client state"),
        "missing stderr state-load context:\n{stderr}"
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

#[test]
fn nmux_live_json_reports_missing_daemon_setup_error() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let output = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--json",
            "--no-input",
        ])
        .output()
        .expect("run nmux --live --json");

    assert!(!output.status.success(), "nmux unexpectedly succeeded");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"event\":\"error\""),
        "missing JSON error event:\n{stdout}"
    );
    assert!(
        stdout.contains("\"message\":\"failed to connect to nmux daemon at")
            && stdout.contains(socket_path.to_str().expect("socket path")),
        "missing socket JSON error context:\n{stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nmux: failed to connect to nmux daemon at"),
        "missing stderr socket context:\n{stderr}"
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
