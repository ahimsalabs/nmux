use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
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
    let mut lines = read_until_line(&lines_rx, "echo:ping");

    stdin.write_all(b"pong\n").expect("write pong");
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
fn live_stdin_requires_iterations_while_bounded() {
    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args(["--socket", "/tmp/nmux-missing.sock", "--live", "--stdin"])
        .output()
        .expect("run nmux");

    assert!(!client.status.success());
    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("--stdin requires --iterations"),
        "unexpected stderr:\n{stderr}"
    );
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
