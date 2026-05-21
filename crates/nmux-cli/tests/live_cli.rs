use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn live_cli_streams_repeated_command_output() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "2",
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
            "2",
            "--key",
            "ping\n",
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
    assert!(stdout.contains("session=local tab=tab-1 pane=pane-1"));
    assert!(
        stdout.matches("echo:ping").count() >= 2,
        "expected repeated streamed echo output, got:\n{stdout}"
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
            "--live-cycles",
            "2",
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

    client
        .stdin
        .as_mut()
        .expect("client stdin")
        .write_all(b"ping\npong\n")
        .expect("write client stdin");

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

fn test_socket_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    PathBuf::from(format!(
        "/tmp/nmux-live-cli-{}-{nanos}.sock",
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
