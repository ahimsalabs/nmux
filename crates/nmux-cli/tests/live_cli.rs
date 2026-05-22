use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_PATH_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn live_cli_streams_command_output_and_committed_resize() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "3",
            "--key",
            "ping\n",
            "--cols",
            "100",
            "--rows",
            "30",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(stdout.contains("session=local tab=tab-1 pane=pane-1 size=80x24 resize=fixed"));
    assert!(stdout.contains("session=local tab=tab-1 pane=pane-1 size=100x30 resize=fixed"));
    assert!(
        stdout.contains("echo:ping"),
        "missing streamed echo output:\n{stdout}"
    );
}

#[test]
fn live_cli_displays_daemon_resize_policy() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--resize-policy",
            "active-client",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("session=local tab=tab-1 pane=pane-1 size=80x24 resize=active-client"),
        "missing active-client resize policy:\n{stdout}"
    );
}

#[test]
fn live_daemon_removes_socket_after_bounded_exit() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for nmuxd");

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");
    assert!(
        !socket_path.exists(),
        "nmuxd left socket after bounded exit: {}",
        socket_path.display()
    );
}

#[test]
fn live_cli_can_wait_for_daemon_socket() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--connect-timeout-ms",
            "2000",
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux");

    thread::sleep(Duration::from_millis(100));

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn nmuxd");

    let client = client.wait_with_output().expect("wait for nmux");
    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(server_status.success(), "nmuxd failed: {server_status}");
    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("ready"),
        "client did not attach after waiting for socket:\n{stdout}"
    );
}

#[test]
fn live_cli_renders_initial_scrollback_range() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\nhistory\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--scrollback-start",
            "1",
            "--scrollback-count",
            "5",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("scrollback 1..5:"),
        "missing requested live scrollback header:\n{stdout}"
    );
    assert!(
        stdout.contains("ready"),
        "missing live scrollback command output:\n{stdout}"
    );
    assert!(
        stdout.contains("history"),
        "missing live scrollback command output:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_cli_can_use_libghostty_vt_terminal_engine() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf '\\033[31mred\\033[0m\nplain\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--scrollback-start",
            "1",
            "--scrollback-count",
            "5",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("red"),
        "missing VT-rendered red text:\n{stdout}"
    );
    assert!(
        stdout.contains("plain"),
        "missing VT-rendered plain text:\n{stdout}"
    );
    assert!(
        !stdout.contains("[31m") && !stdout.contains("[0m"),
        "ANSI control sequences leaked into output:\n{stdout}"
    );
}

#[test]
fn live_cli_redraw_includes_initial_scrollback_range() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\nhistory\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--redraw",
            "--no-input",
            "--iterations",
            "1",
            "--scrollback-start",
            "1",
            "--scrollback-count",
            "5",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains(
            "\x1b[2J\x1b[Hsession=local tab=tab-1 pane=pane-1 size=80x24 resize=fixed\nscrollback 1..5:"
        ),
        "missing redraw scrollback context:\n{stdout:?}"
    );
    assert!(
        stdout.contains("ready"),
        "missing live scrollback command output:\n{stdout}"
    );
    assert!(
        stdout.contains("history"),
        "missing live scrollback command output:\n{stdout}"
    );
}

#[test]
fn live_cli_uses_shared_default_socket_from_runtime_dir() {
    let runtime_dir = test_runtime_dir();
    let socket_path = runtime_dir.join("nmux").join("nmuxd.sock");
    let _ = fs::remove_dir_all(&runtime_dir);
    fs::create_dir_all(&runtime_dir).expect("create runtime dir");

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .args([
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .args([
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_dir_all(&runtime_dir);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("ready"),
        "default-socket live attach missed daemon output:\n{stdout}"
    );
}

fn test_runtime_dir() -> PathBuf {
    let id = NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(format!("/tmp/nmuxrt{}-{id}", std::process::id()))
}

#[test]
fn live_cli_warns_when_resize_request_conflicts_with_manual_policy() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--resize-policy",
            "manual",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--cols",
            "100",
            "--rows",
            "30",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("nmux: resize request ignored by manual resize policy"),
        "missing resize policy warning:\n{stderr}"
    );
    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("session=local tab=tab-1 pane=pane-1 size=80x24 resize=manual"),
        "missing manual resize policy summary:\n{stdout}"
    );
    assert!(
        !stdout.contains("size=100x30"),
        "manual policy should not publish committed resize:\n{stdout}"
    );
}

#[test]
fn live_cli_redraw_repaints_surface_in_place() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--redraw",
            "--iterations",
            "1",
            "--key",
            "paint\n",
            "--cols",
            "100",
            "--rows",
            "30",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\x1b[2J\x1b[H"),
        "missing clear-and-home redraw sequence:\n{stdout:?}"
    );
    assert!(
        stdout.contains("echo:paint"),
        "missing rendered command output:\n{stdout}"
    );
    assert!(
        stdout.contains("session=local tab=tab-1 pane=pane-1 size=100x30 resize=fixed"),
        "missing redraw workspace status after resize:\n{stdout:?}"
    );
}

#[test]
fn live_clients_can_reattach_to_persisted_workspace_state() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--iterations",
            "1",
            "--key",
            "reattach\n",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run first nmux");

    assert!(
        first_client.status.success(),
        "first nmux failed: {}",
        String::from_utf8_lossy(&first_client.stderr)
    );

    let second_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--scrollback-start",
            "999",
            "--scrollback-count",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("echo:reattach"),
        "reattached client did not render cached current live surface:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_clients_can_reattach_to_persisted_workspace_state() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf '\\033[32mecho:%s\\033[0m\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--iterations",
            "1",
            "--key",
            "reattach\n",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run first nmux");

    assert!(
        first_client.status.success(),
        "first nmux failed: {}",
        String::from_utf8_lossy(&first_client.stderr)
    );

    let second_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--scrollback-start",
            "1",
            "--scrollback-count",
            "8",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("echo:reattach"),
        "reattached libghostty-vt client did not render cached current live surface:\n{stdout}"
    );
    assert!(
        stdout.contains("scrollback"),
        "reattached libghostty-vt client did not fetch scrollback:\n{stdout}"
    );
    assert!(
        !stdout.contains("[32m") && !stdout.contains("[0m"),
        "ANSI control sequences leaked after libghostty-vt reattach:\n{stdout}"
    );
}

#[test]
fn live_forever_can_serve_sequential_reattach_clients() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-forever",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--key",
            "forever\n",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run first nmux");

    assert!(
        first_client.status.success(),
        "first nmux failed: {}",
        String::from_utf8_lossy(&first_client.stderr)
    );

    let second_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let _ = server.kill();
    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(
        !server_status.success(),
        "nmuxd should have been terminated after test clients"
    );

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("echo:forever"),
        "unbounded reattached client did not see prior live state:\n{stdout}"
    );
}

#[test]
fn live_cli_can_drive_distinct_input_lines_from_stdin() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let mut client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--stdin",
            "--iterations",
            "2",
            "--interval-ms",
            "1000",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux");

    let mut stdin = client.stdin.take().expect("client stdin");
    stdin.write_all(b"ping\n").expect("write ping");
    stdin.flush().expect("flush ping");
    stdin.write_all(b"pong\n").expect("write pong");
    stdin.flush().expect("flush pong");
    drop(stdin);

    let client = client.wait_with_output().expect("wait for nmux");
    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(stdout.contains("echo:ping"), "missing ping echo:\n{stdout}");
    assert!(stdout.contains("echo:pong"), "missing pong echo:\n{stdout}");
}

#[test]
fn live_cli_can_drive_input_chunks_from_stdin_bytes() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let mut client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--stdin-bytes",
            "--interval-ms",
            "1000",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux");

    let stdout = client.stdout.take().expect("client stdout");
    let (lines_tx, lines_rx) = mpsc::channel();
    let stdout_reader = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            lines_tx.send(line.expect("stdout line")).ok();
        }
    });

    let mut stdin = client.stdin.take().expect("client stdin");
    stdin.write_all(b"pi").expect("write partial ping");
    stdin.flush().expect("flush partial ping");
    thread::sleep(Duration::from_millis(30));
    stdin.write_all(b"ng\n").expect("write ping terminator");
    stdin.flush().expect("flush ping");
    let mut lines = read_until_line(&lines_rx, "echo:ping");

    stdin.write_all(b"po").expect("write partial pong");
    stdin.flush().expect("flush partial pong");
    thread::sleep(Duration::from_millis(30));
    stdin.write_all(b"ng\n").expect("write pong terminator");
    stdin.flush().expect("flush pong");
    lines.extend(read_until_line(&lines_rx, "echo:pong"));
    drop(stdin);

    let client = client.wait_with_output().expect("wait for nmux");
    stdout_reader.join().expect("stdout reader");
    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");
    assert!(
        lines.iter().any(|line| line.contains("echo:ping")),
        "missing ping echo:\n{}",
        lines.join("\n")
    );
    assert!(
        lines.iter().any(|line| line.contains("echo:pong")),
        "missing pong echo:\n{}",
        lines.join("\n")
    );
}

#[test]
fn live_stdin_bytes_keeps_polling_before_input_arrives() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; sleep 0.05; printf 'tick-before-input\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let mut client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--stdin-bytes",
            "--interval-ms",
            "1000",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux");

    let stdout = client.stdout.take().expect("client stdout");
    let (lines_tx, lines_rx) = mpsc::channel();
    let stdout_reader = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            lines_tx.send(line.expect("stdout line")).ok();
        }
    });

    let lines = read_until_line(&lines_rx, "tick-before-input");
    drop(client.stdin.take().expect("client stdin"));

    let client = client.wait_with_output().expect("wait for nmux");
    stdout_reader.join().expect("stdout reader");
    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("nmux: stdin EOF; detached"),
        "missing stdin EOF status:\n{stderr}"
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");
    assert!(
        lines.iter().any(|line| line.contains("tick-before-input")),
        "missing delayed output:\n{}",
        lines.join("\n")
    );
}

#[test]
fn live_stdin_bytes_ctrl_right_bracket_detaches() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let mut client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--stdin-bytes",
            "--interval-ms",
            "1000",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux");

    let stdout = client.stdout.take().expect("client stdout");
    let (lines_tx, lines_rx) = mpsc::channel();
    let stdout_reader = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            lines_tx.send(line.expect("stdout line")).ok();
        }
    });

    let mut stdin = client.stdin.take().expect("client stdin");
    stdin.write_all(b"ping\n").expect("write ping");
    stdin.flush().expect("flush ping");
    let lines = read_until_line(&lines_rx, "echo:ping");
    stdin.write_all(b"\x1d").expect("write detach key");
    stdin.flush().expect("flush detach key");

    let client = client.wait_with_output().expect("wait for nmux");
    stdout_reader.join().expect("stdout reader");
    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("nmux: detached by local Ctrl-]"),
        "missing detach status:\n{stderr}"
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");
    assert!(
        lines.iter().any(|line| line.contains("echo:ping")),
        "missing ping echo:\n{}",
        lines.join("\n")
    );
}

#[test]
fn live_read_only_cli_observes_output_without_input() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "3",
            "--command",
            "printf 'ready\n'; sleep 0.05; printf 'tick-one\n'; sleep 0.05; printf 'tick-two\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-input",
            "--iterations",
            "3",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("tick-one"),
        "missing first observed tick:\n{stdout}"
    );
    assert!(
        stdout.contains("tick-two"),
        "missing second observed tick:\n{stdout}"
    );
}

#[test]
fn live_read_only_cli_without_iterations_runs_until_server_closes() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "3",
            "--command",
            "printf 'ready\n'; sleep 0.05; printf 'tick-one\n'; sleep 0.05; printf 'tick-two\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-input",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("nmux: live server closed connection"),
        "missing daemon close status:\n{stderr}"
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("tick-one"),
        "missing first observed tick:\n{stdout}"
    );
    assert!(
        stdout.contains("tick-two"),
        "missing second observed tick:\n{stdout}"
    );
}

#[test]
fn live_stdin_without_iterations_stops_on_eof_without_default_key() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let mut client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--stdin",
            "--interval-ms",
            "1000",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux");

    let mut stdin = client.stdin.take().expect("client stdin");
    stdin.write_all(b"ping\npong\n").expect("write stdin");
    drop(stdin);

    let client = client.wait_with_output().expect("wait for nmux");
    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("nmux: stdin EOF; detached"),
        "missing stdin EOF status:\n{stderr}"
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(stdout.contains("echo:ping"), "missing ping echo:\n{stdout}");
    assert!(stdout.contains("echo:pong"), "missing pong echo:\n{stdout}");
    assert!(
        !stdout.contains("echo:a"),
        "stdin mode fell back to default key input:\n{stdout}"
    );
}

#[test]
fn live_cli_persists_rendered_surface_state() {
    let socket_path = test_socket_path();
    let state_path = test_state_path();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--key",
            "persist\n",
            "--state",
            state_path.to_str().expect("state path"),
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let state = fs::read_to_string(&state_path).expect("read state");
    let _ = fs::remove_file(&state_path);
    assert!(
        state.contains("surface 70616e652d31 4 "),
        "expected pane-1 surface version 4 in state:\n{state}"
    );
    assert!(
        state.contains("6563686f3a70657273697374"),
        "expected rendered live output in state:\n{state}"
    );
}

fn test_socket_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let id = NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(format!(
        "/tmp/nmux-live-cli-{}-{nanos}-{id}.sock",
        std::process::id()
    ))
}

fn test_state_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let id = NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(format!(
        "/tmp/nmux-live-cli-state-{}-{nanos}-{id}.state",
        std::process::id()
    ))
}

fn wait_for_socket(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("socket did not appear: {}", path.display());
}

fn read_until_line(rx: &mpsc::Receiver<String>, expected: &str) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut lines = Vec::new();
    while Instant::now() < deadline {
        let timeout = deadline.saturating_duration_since(Instant::now());
        let Ok(line) = rx.recv_timeout(timeout.min(Duration::from_millis(100))) else {
            continue;
        };
        let matched = line.contains(expected);
        lines.push(line);
        if matched {
            return lines;
        }
    }
    panic!(
        "did not receive line containing {expected:?}; got:\n{}",
        lines.join("\n")
    );
}
