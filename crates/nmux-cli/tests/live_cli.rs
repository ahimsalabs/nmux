use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

static NEXT_PATH_ID: AtomicU64 = AtomicU64::new(0);
const STDIN_BYTES_DETACH: u8 = 0x1d;
const DEFAULT_WORKSPACE_SUMMARY: &str =
    "session=local tab=tab-1 pane=pane-1 size=80x24 resize=fixed";

struct PtyCommandOutput {
    success: bool,
    output: String,
}

struct PtyCommand {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output_rx: mpsc::Receiver<Vec<u8>>,
    reader_thread: thread::JoinHandle<()>,
}

fn assert_default_workspace_attached(stdout: &str, context: &str) {
    assert!(
        stdout.contains(DEFAULT_WORKSPACE_SUMMARY),
        "{context}:\n{stdout}"
    );
}

fn assert_split_pty_writes_render_as_one_line(stdout: &str) {
    assert!(
        stdout.lines().any(|line| line == "abc"),
        "missing split-write line:\n{stdout}"
    );
    for fragment in ["a", "b", "c", "ab"] {
        assert!(
            !stdout.lines().any(|line| line == fragment),
            "split-write fragment {fragment:?} rendered as its own line:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("\na\nb\nc\n"),
        "split writes rendered as separate rows:\n{stdout}"
    );
}

fn spawn_nmux_client_in_pty(args: &[&str]) -> PtyCommand {
    spawn_nmux_client_in_pty_with_env(args, &[])
}

fn spawn_nmux_client_in_pty_with_env(args: &[&str], env: &[(&str, &str)]) -> PtyCommand {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open client pty");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_nmux"));
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    let child = pair
        .slave
        .spawn_command(command)
        .expect("spawn nmux in pty");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone pty reader");
    let writer = pair.master.take_writer().expect("take pty writer");
    let (output_tx, output_rx) = mpsc::channel();
    let reader_thread = thread::spawn(move || {
        let mut output = Vec::new();
        reader.read_to_end(&mut output).expect("read pty output");
        output_tx.send(output).ok();
    });

    PtyCommand {
        child,
        master: pair.master,
        writer,
        output_rx,
        reader_thread,
    }
}

fn daemon_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nmux"));
    command.arg("daemon");
    command
}

impl PtyCommand {
    fn detach(&mut self) {
        let _ = self.writer.write_all(&[STDIN_BYTES_DETACH]);
        let _ = self.writer.flush();
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize client pty");
    }

    fn kill(&mut self) {
        self.child.kill().expect("kill nmux in pty");
    }

    fn wait(mut self) -> PtyCommandOutput {
        let status = self.child.wait().expect("wait for nmux in pty");
        let output = self
            .output_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("collect pty output");
        self.reader_thread.join().expect("join pty reader");

        PtyCommandOutput {
            success: status.success(),
            output: String::from_utf8_lossy(&output).into_owned(),
        }
    }
}

#[test]
fn one_shot_cli_receives_nmux_pane_environment() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = "printf 'env:%s:%s:%s:%s:%s\\n' \"$NMUX\" \"$NMUX_SESSION_ID\" \"$NMUX_PANE_ID\" \"$NMUX_SOCKET\" \"$NMUX_ORIGIN\"; cat >/dev/null".to_string();

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--command",
            &command,
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args(["--socket", socket_path.to_str().expect("socket path")])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    let expected = format!(
        "env:1:local:pane-1:{}:local",
        socket_path.to_str().expect("socket path")
    );
    assert!(
        stdout.contains(&expected),
        "missing nmux pane environment {expected:?}:\n{stdout}"
    );
}

#[test]
fn one_shot_split_daemon_attaches_active_new_pane() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = "printf 'split-env:%s\\n' \"$NMUX_PANE_ID\"; cat >/dev/null";

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--split",
            "vertical",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn split daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--json",
        ])
        .output()
        .expect("run nmux --json");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --json failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\"pane_id\":\"pane-2\""),
        "split attach should report focused pane-2:\n{stdout}"
    );
    assert!(
        stdout.contains("split-env:pane-2"),
        "split attach should render pane-2 process output:\n{stdout}"
    );
}

#[test]
fn nmux_daemon_alias_serves_list_subcommands() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = "printf 'list-ready:%s\\n' \"$NMUX_PANE_ID\"; sleep 1";

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "daemon",
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--session",
            "work",
            "--live-clients",
            "3",
            "--split",
            "vertical",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn nmux daemon");

    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(150));

    let pane_list = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--session",
            "work",
            "pane",
            "ls",
            "--json",
        ])
        .output()
        .expect("run nmux pane ls");
    assert!(
        pane_list.status.success(),
        "nmux pane ls failed: {}",
        String::from_utf8_lossy(&pane_list.stderr)
    );
    let pane_stdout = String::from_utf8_lossy(&pane_list.stdout);
    assert!(
        pane_stdout.contains("\"pane_id\":\"pane-1\"")
            && pane_stdout.contains("\"pane_id\":\"pane-2\"")
            && pane_stdout.contains("\"active\":true"),
        "pane list should include both split panes and active marker:\n{pane_stdout}"
    );

    let tab_list = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--session",
            "work",
            "tab",
            "ls",
            "--json",
        ])
        .output()
        .expect("run nmux tab ls");
    assert!(
        tab_list.status.success(),
        "nmux tab ls failed: {}",
        String::from_utf8_lossy(&tab_list.stderr)
    );
    assert!(
        String::from_utf8_lossy(&tab_list.stdout).contains("\"tab_id\":\"tab-1\""),
        "tab list should include active tab:\n{}",
        String::from_utf8_lossy(&tab_list.stdout)
    );

    let session_list = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--session",
            "work",
            "--json",
            "ls",
        ])
        .output()
        .expect("run nmux ls");
    let server_status = server.wait().expect("wait for nmux daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        session_list.status.success(),
        "nmux ls failed: {}",
        String::from_utf8_lossy(&session_list.stderr)
    );
    assert!(
        String::from_utf8_lossy(&session_list.stdout).contains("\"session_id\":\"work\""),
        "session list should include named session:\n{}",
        String::from_utf8_lossy(&session_list.stdout)
    );
    assert!(
        server_status.success(),
        "nmux daemon failed: {server_status}"
    );
}

#[test]
fn nmux_kill_stops_named_daemon_and_rejects_wrong_session() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "daemon",
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--session",
            "work",
            "--live-forever",
            "--command",
            "printf 'kill-ready\\n'; sleep 30",
        ])
        .spawn()
        .expect("spawn nmux daemon");

    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(150));

    let wrong_session = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "kill",
            "other",
        ])
        .output()
        .expect("run nmux kill other");
    assert!(
        !wrong_session.status.success(),
        "wrong-session kill unexpectedly succeeded:\n{}",
        String::from_utf8_lossy(&wrong_session.stdout)
    );
    assert!(
        String::from_utf8_lossy(&wrong_session.stderr).contains("session not found: other"),
        "wrong-session kill should report server error:\n{}",
        String::from_utf8_lossy(&wrong_session.stderr)
    );
    assert!(
        server.try_wait().expect("poll daemon").is_none(),
        "wrong-session kill should leave daemon running"
    );

    let kill = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "kill",
            "work",
        ])
        .output()
        .expect("run nmux kill work");
    assert!(
        kill.status.success(),
        "nmux kill work failed: {}",
        String::from_utf8_lossy(&kill.stderr)
    );
    assert!(
        kill.stdout.is_empty(),
        "nmux kill should be quiet on success:\n{}",
        String::from_utf8_lossy(&kill.stdout)
    );

    let server_status =
        wait_for_child_exit(&mut server, Duration::from_secs(5)).expect("daemon exited after kill");
    let _ = fs::remove_file(&socket_path);

    assert!(
        server_status.success(),
        "nmux daemon should exit cleanly after kill: {server_status}"
    );
}

#[test]
fn live_redraw_split_daemon_renders_pane_layout() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = "printf 'split-env:%s\\n' \"$NMUX_PANE_ID\"; sleep 1";

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--split",
            "vertical",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn split daemon");

    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(150));

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--redraw",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux --live --redraw");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --live --redraw failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("[pane-1]") && stdout.contains("[pane-2 active]"),
        "split redraw should label both panes:\n{stdout}"
    );
    assert!(
        stdout.contains("split-env:pane-2"),
        "split redraw should render the active split pane surface:\n{stdout}"
    );
    assert!(
        stdout.contains("split-env:pane-1"),
        "split redraw should render the inactive split pane surface:\n{stdout}"
    );
    assert!(
        stdout.contains(" | "),
        "split redraw should render vertical pane separator:\n{stdout}"
    );
}

#[test]
fn live_redraw_split_reattach_restores_cached_inactive_pane() {
    let socket_path = test_socket_path();
    let state_path = test_state_path();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);
    let command = "printf 'split-env:%s\\n' \"$NMUX_PANE_ID\"; sleep 3";

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--split",
            "vertical",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn split daemon");

    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(150));

    for label in ["first", "second"] {
        let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
            .args([
                "--socket",
                socket_path.to_str().expect("socket path"),
                "--state",
                state_path.to_str().expect("state path"),
                "--live",
                "--redraw",
                "--no-scrollback",
                "--iterations",
                "1",
                "--interval-ms",
                "1000",
            ])
            .output()
            .unwrap_or_else(|err| panic!("run {label} nmux --live --redraw: {err}"));

        assert!(
            client.status.success(),
            "{label} nmux --live --redraw failed: {}",
            String::from_utf8_lossy(&client.stderr)
        );
        let stdout = String::from_utf8_lossy(&client.stdout);
        assert!(
            stdout.contains("[pane-1]") && stdout.contains("[pane-2 active]"),
            "{label} redraw should label both split panes:\n{stdout}"
        );
        assert!(
            stdout.contains("split-env:pane-1"),
            "{label} redraw should render cached inactive pane output:\n{stdout}"
        );
        assert!(
            stdout.contains("split-env:pane-2"),
            "{label} redraw should render active pane output:\n{stdout}"
        );
        if label == "second" {
            assert!(
                !stdout.contains("(surface not cached)"),
                "{label} redraw should not lose cached split surfaces:\n{stdout}"
            );
        }
    }

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(server_status.success(), "daemon failed: {server_status}");
}

#[test]
fn one_shot_split_daemon_can_attach_requested_pane() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = "printf 'split-env:%s\\n' \"$NMUX_PANE_ID\"; cat >/dev/null";

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--split",
            "vertical",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn split daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--pane",
            "pane-1",
            "--json",
        ])
        .output()
        .expect("run nmux --pane pane-1 --json");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --pane pane-1 --json failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\"pane_id\":\"pane-1\""),
        "requested pane attach should report pane-1:\n{stdout}"
    );
    assert!(
        stdout.contains("split-env:pane-1"),
        "requested pane attach should render pane-1 output:\n{stdout}"
    );
}

#[test]
fn one_shot_multi_tab_daemon_attaches_active_tab() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = "printf 'tab-env:%s:%s\\n' \"$NMUX_PANE_ID\" \"$NMUX_SOCKET\"; cat >/dev/null";

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--tabs",
            "2",
            "--active-tab",
            "tab-2",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn multi-tab daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--json",
        ])
        .output()
        .expect("run nmux --json");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --json failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\"tab_id\":\"tab-2\""),
        "multi-tab attach should report active tab-2:\n{stdout}"
    );
    assert!(
        stdout.contains("\"pane_id\":\"tab-2-pane-1\""),
        "multi-tab attach should report tab-2 pane:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "tab-env:tab-2-pane-1:{}",
            socket_path.to_str().expect("socket path")
        )),
        "multi-tab attach should render tab-2 process output:\n{stdout}"
    );
}

#[test]
fn one_shot_cli_can_switch_to_requested_tab() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = "printf 'tab-env:%s\\n' \"$NMUX_PANE_ID\"; cat >/dev/null";

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--tabs",
            "2",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn multi-tab daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--tab",
            "tab-2",
            "--json",
        ])
        .output()
        .expect("run nmux --tab tab-2 --json");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --tab tab-2 --json failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\"tab_id\":\"tab-2\""),
        "requested tab attach should switch workspace to tab-2:\n{stdout}"
    );
    assert!(
        stdout.contains("\"pane_id\":\"tab-2-pane-1\""),
        "requested tab attach should report tab-2 active pane:\n{stdout}"
    );
    assert!(
        stdout.contains("tab-env:tab-2-pane-1"),
        "requested tab attach should render tab-2 output:\n{stdout}"
    );
}

#[test]
fn one_shot_cli_can_print_attach_json() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = "printf 'json-output\\n'; cat >/dev/null";

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--json",
        ])
        .output()
        .expect("run nmux --json");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --json failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.starts_with("{\"workspace\":{"),
        "not JSON:\n{stdout}"
    );
    assert!(
        stdout.contains("\"session_id\":\"local\""),
        "missing workspace:\n{stdout}"
    );
    assert!(
        stdout.contains("\"resize_policy\":\"fixed\""),
        "missing policy:\n{stdout}"
    );
    assert!(stdout.contains("json-output"), "missing surface:\n{stdout}");
    assert!(
        stdout.contains("\"surface\":{"),
        "missing structured surface:\n{stdout}"
    );
    assert!(
        stdout.contains("\"row_updates\":["),
        "missing structured surface rows:\n{stdout}"
    );
    assert!(
        stdout.contains("\"scrollback\":{"),
        "missing scrollback:\n{stdout}"
    );
    assert!(
        stdout.contains("\"lines\":["),
        "missing structured scrollback lines:\n{stdout}"
    );
    assert!(
        stdout.contains("\"runs\":["),
        "missing structured scrollback runs:\n{stdout}"
    );
}

#[test]
fn pane_snapshot_subcommand_prints_json() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--command",
            "printf 'snapshot-ready\n'; cat >/dev/null",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "pane",
            "snapshot",
            "pane-1",
            "--json",
        ])
        .output()
        .expect("run nmux pane snapshot");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux pane snapshot failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\"pane_id\":\"pane-1\""),
        "missing pane:\n{stdout}"
    );
    assert!(
        stdout.contains("snapshot-ready"),
        "missing surface text:\n{stdout}"
    );
    assert!(
        stdout.contains("\"scrollback\":null"),
        "pane snapshot should skip scrollback fetch:\n{stdout}"
    );
}

#[test]
fn pane_send_subcommand_writes_to_target_pane() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'sent:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let reader = Command::new(env!("CARGO_BIN_EXE_nmux"))
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn reader nmux");

    thread::sleep(Duration::from_millis(150));

    let sender = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "pane",
            "send",
            "pane-1",
            "scripted\n",
        ])
        .output()
        .expect("run nmux pane send");

    assert!(
        sender.status.success(),
        "nmux pane send failed: {}",
        String::from_utf8_lossy(&sender.stderr)
    );
    assert!(
        sender.stdout.is_empty(),
        "pane send should be quiet on success:\n{}",
        String::from_utf8_lossy(&sender.stdout)
    );

    let reader = reader.wait_with_output().expect("wait for reader nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        reader.status.success(),
        "reader nmux failed: {}",
        String::from_utf8_lossy(&reader.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&reader.stdout);
    assert!(
        stdout.contains("sent:scripted"),
        "pane send did not reach target pane:\n{stdout}"
    );
}

#[test]
fn scriptable_cli_splits_panes_and_manages_tabs() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--command",
            "printf 'ready:%s\\n' \"$NMUX_PANE_ID\"; cat >/dev/null",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let split = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--json",
            "pane",
            "split",
            "vertical",
            "pane-1",
        ])
        .output()
        .expect("run nmux pane split");
    assert!(
        split.status.success(),
        "nmux pane split failed: {}",
        String::from_utf8_lossy(&split.stderr)
    );
    let split_stdout = String::from_utf8_lossy(&split.stdout);
    assert!(
        split_stdout.contains("\"pane_id\":\"pane-2\""),
        "split should focus the new pane:\n{split_stdout}"
    );

    let tab_new = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--json",
            "tab",
            "new",
            "tab-script",
            "--title",
            "Script",
        ])
        .output()
        .expect("run nmux tab new");
    assert!(
        tab_new.status.success(),
        "nmux tab new failed: {}",
        String::from_utf8_lossy(&tab_new.stderr)
    );
    let tab_new_stdout = String::from_utf8_lossy(&tab_new.stdout);
    assert!(
        tab_new_stdout.contains("\"tab_id\":\"tab-script\"")
            && tab_new_stdout.contains("\"pane_id\":\"tab-script-pane-1\""),
        "tab new should focus the new tab pane:\n{tab_new_stdout}"
    );

    let tab_switch = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--json",
            "tab",
            "switch",
            "tab-1",
        ])
        .output()
        .expect("run nmux tab switch");
    assert!(
        tab_switch.status.success(),
        "nmux tab switch failed: {}",
        String::from_utf8_lossy(&tab_switch.stderr)
    );
    let tab_switch_stdout = String::from_utf8_lossy(&tab_switch.stdout);
    assert!(
        tab_switch_stdout.contains("\"tab_id\":\"tab-1\"")
            && tab_switch_stdout.contains("\"pane_id\":\"pane-2\""),
        "tab switch should focus tab-1 pane:\n{tab_switch_stdout}"
    );

    let tab_close = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--json",
            "tab",
            "close",
            "tab-script",
        ])
        .output()
        .expect("run nmux tab close");
    assert!(
        tab_close.status.success(),
        "nmux tab close failed: {}",
        String::from_utf8_lossy(&tab_close.stderr)
    );
    let tab_close_stdout = String::from_utf8_lossy(&tab_close.stdout);
    assert!(
        tab_close_stdout.contains("\"tab_id\":\"tab-1\"")
            && tab_close_stdout.contains("\"pane_id\":\"pane-2\""),
        "tab close should return to tab-1 focused pane:\n{tab_close_stdout}"
    );

    let attach = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--json",
            "--pane",
            "pane-2",
        ])
        .output()
        .expect("run nmux attach pane-2");

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);

    assert!(
        attach.status.success(),
        "nmux attach pane-2 failed: {}",
        String::from_utf8_lossy(&attach.stderr)
    );
    let attach_stdout = String::from_utf8_lossy(&attach.stdout);
    assert!(
        attach_stdout.contains("\"pane_id\":\"pane-2\"") && attach_stdout.contains("ready:pane-2"),
        "attach should see the runtime split pane output:\n{attach_stdout}"
    );
}

#[test]
fn scriptable_cli_creates_and_attaches_named_session() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--command",
            "printf 'ready:%s:%s\\n' \"$NMUX_SESSION_ID\" \"$NMUX_PANE_ID\"; cat >/dev/null",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let session_new = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--json",
            "session",
            "new",
            "work",
            "--title",
            "Work",
        ])
        .output()
        .expect("run nmux session new");
    assert!(
        session_new.status.success(),
        "nmux session new failed: {}",
        String::from_utf8_lossy(&session_new.stderr)
    );
    let session_new_stdout = String::from_utf8_lossy(&session_new.stdout);
    assert!(
        session_new_stdout.contains("\"session_id\":\"work\"")
            && session_new_stdout.contains("\"pane_id\":\"pane-1\""),
        "session new should return the new workspace:\n{session_new_stdout}"
    );

    let session_list = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--json",
            "ls",
        ])
        .output()
        .expect("list sessions");
    assert!(
        session_list.status.success(),
        "nmux ls failed: {}",
        String::from_utf8_lossy(&session_list.stderr)
    );
    let session_list_stdout = String::from_utf8_lossy(&session_list.stdout);
    assert!(
        session_list_stdout.contains("\"session_id\":\"local\"")
            && session_list_stdout.contains("\"session_id\":\"work\"")
            && session_list_stdout.contains("\"title\":\"Work\""),
        "session list should include default and named sessions:\n{session_list_stdout}"
    );

    let attach_work = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--session",
            "work",
            "--json",
        ])
        .output()
        .expect("attach work session");
    assert!(
        attach_work.status.success(),
        "nmux attach work failed: {}",
        String::from_utf8_lossy(&attach_work.stderr)
    );
    let attach_work_stdout = String::from_utf8_lossy(&attach_work.stdout);
    assert!(
        attach_work_stdout.contains("\"session_id\":\"work\"")
            && attach_work_stdout.contains("ready:work:pane-1"),
        "attach should target the new named session:\n{attach_work_stdout}"
    );

    let attach_default = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--json",
        ])
        .output()
        .expect("attach default session");

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);

    assert!(
        attach_default.status.success(),
        "nmux attach default failed: {}",
        String::from_utf8_lossy(&attach_default.stderr)
    );
    let attach_default_stdout = String::from_utf8_lossy(&attach_default.stdout);
    assert!(
        attach_default_stdout.contains("\"session_id\":\"local\"")
            && attach_default_stdout.contains("ready:local:pane-1"),
        "untargeted attach should keep using the default session:\n{attach_default_stdout}"
    );
}

#[test]
fn scriptable_named_session_panes_inherit_session_environment() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--session",
            "work",
            "--command",
            "printf 'ready:%s:%s\\n' \"$NMUX_SESSION_ID\" \"$NMUX_PANE_ID\"; cat >/dev/null",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let split = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--session",
            "work",
            "--json",
            "pane",
            "split",
            "vertical",
            "pane-1",
        ])
        .output()
        .expect("run nmux pane split");
    assert!(
        split.status.success(),
        "nmux pane split failed: {}",
        String::from_utf8_lossy(&split.stderr)
    );

    let tab_new = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--session",
            "work",
            "--json",
            "tab",
            "new",
            "tab-script",
        ])
        .output()
        .expect("run nmux tab new");
    assert!(
        tab_new.status.success(),
        "nmux tab new failed: {}",
        String::from_utf8_lossy(&tab_new.stderr)
    );
    thread::sleep(Duration::from_millis(100));

    let attach = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--session",
            "work",
            "--json",
        ])
        .output()
        .expect("run nmux attach");

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);

    assert!(
        attach.status.success(),
        "nmux attach failed: {}",
        String::from_utf8_lossy(&attach.stderr)
    );
    let attach_stdout = String::from_utf8_lossy(&attach.stdout);
    assert!(
        attach_stdout.contains("\"session_id\":\"work\"")
            && attach_stdout.contains("\"pane_id\":\"tab-script-pane-1\"")
            && attach_stdout.contains("ready:work:tab-script-pane-1"),
        "runtime-created panes should inherit the named session:\n{attach_stdout}"
    );
}

#[test]
fn tcp_transport_can_attach_with_positional_remote_and_token_env() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve tcp port");
    let addr = listener.local_addr().expect("tcp addr").to_string();
    drop(listener);

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "daemon",
            "--listen",
            &addr,
            "--token",
            "test-token",
            "--one-shot",
            "--command",
            "printf 'tcp-ready\\n'",
        ])
        .spawn()
        .expect("spawn tcp daemon");

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([&addr, "--connect-timeout-ms", "2000", "--json"])
        .env("NMUX_TOKEN", "test-token")
        .output()
        .expect("run nmux over positional tcp");

    let server_status = server.wait().expect("wait for tcp daemon");

    assert!(
        client.status.success(),
        "nmux tcp attach failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");
    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\"pane_id\":\"pane-1\"") && stdout.contains("tcp-ready"),
        "tcp attach should render the remote daemon pane:\n{stdout}"
    );
}

#[test]
fn one_shot_json_reports_state_save_error() {
    let socket_path = test_socket_path();
    let blocking_parent = test_state_path();
    let state_path = blocking_parent.join("client.state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_dir_all(&blocking_parent);
    fs::create_dir(&blocking_parent).expect("create read-only parent");
    fs::set_permissions(&blocking_parent, fs::Permissions::from_mode(0o500))
        .expect("make parent read-only");

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--command",
            "printf 'save-error\n'",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--json",
        ])
        .output()
        .expect("run nmux --json");

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::set_permissions(&blocking_parent, fs::Permissions::from_mode(0o700));
    let _ = fs::remove_dir_all(&blocking_parent);

    assert!(
        !client.status.success(),
        "nmux unexpectedly succeeded:\n{}",
        String::from_utf8_lossy(&client.stdout)
    );
    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\"error\""),
        "missing JSON error object:\n{stdout}"
    );
    assert!(
        stdout.contains("\"message\":\"failed to save client state")
            && stdout.contains(state_path.to_str().expect("state path")),
        "missing state-save JSON error context:\n{stdout}"
    );
    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("nmux: failed to save client state"),
        "missing stderr state-save context:\n{stderr}"
    );
}

#[test]
fn live_json_reports_state_save_error() {
    let socket_path = test_socket_path();
    let blocking_parent = test_state_path();
    let state_path = blocking_parent.join("client.state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_dir_all(&blocking_parent);
    fs::create_dir(&blocking_parent).expect("create read-only parent");
    fs::set_permissions(&blocking_parent, fs::Permissions::from_mode(0o500))
        .expect("make parent read-only");

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'live-save-error\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--json",
            "--no-input",
            "--iterations",
            "1",
            "--interval-ms",
            "100",
        ])
        .output()
        .expect("run nmux --live --json");

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::set_permissions(&blocking_parent, fs::Permissions::from_mode(0o700));
    let _ = fs::remove_dir_all(&blocking_parent);

    assert!(
        !client.status.success(),
        "nmux unexpectedly succeeded:\n{}",
        String::from_utf8_lossy(&client.stdout)
    );
    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\"event\":\"attach\""),
        "missing initial live attach event:\n{stdout}"
    );
    assert!(
        stdout.contains("\"event\":\"error\""),
        "missing live JSON error event:\n{stdout}"
    );
    assert!(
        stdout.contains("\"message\":\"failed to save client state")
            && stdout.contains(state_path.to_str().expect("state path")),
        "missing state-save JSON error context:\n{stdout}"
    );
    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("nmux: failed to save client state"),
        "missing stderr state-save context:\n{stderr}"
    );
}

#[test]
fn one_shot_cli_can_request_scrollback_tail() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = "printf 'alpha\nbeta\ngamma\ndelta\n'; cat >/dev/null";

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--terminal-engine",
            "interim",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--scrollback-tail",
            "2",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("scrollback 6..7:"),
        "missing requested tail scrollback header:\n{stdout}"
    );
    assert!(stdout.contains("gamma"), "missing tail line:\n{stdout}");
    assert!(stdout.contains("delta"), "missing tail line:\n{stdout}");
}

#[test]
fn one_shot_cli_can_skip_scrollback_fetch() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = "printf 'current-only\nhistory-one\nhistory-two\n'; cat >/dev/null";

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--no-scrollback",
        ])
        .output()
        .expect("run nmux --no-scrollback");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --no-scrollback failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("current-only"),
        "missing visible surface output:\n{stdout}"
    );
    assert!(
        !stdout.contains("scrollback "),
        "unexpected scrollback block:\n{stdout}"
    );
}

#[test]
fn state_info_reports_persisted_cache_without_connecting() {
    let socket_path = test_socket_path();
    let state_path = test_state_path();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--command",
            "printf 'state-info\n'; cat >/dev/null",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let attach = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--no-input",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        attach.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&attach.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let info_json = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--state-info-json",
        ])
        .output()
        .expect("run nmux --state-info-json");
    assert!(
        info_json.status.success(),
        "state-info-json failed: {}",
        String::from_utf8_lossy(&info_json.stderr)
    );
    let stdout = String::from_utf8_lossy(&info_json.stdout);
    assert!(
        stdout.contains("\"exists\":true"),
        "missing exists:\n{stdout}"
    );
    assert!(
        stdout.contains("\"socket_exists\":false"),
        "missing socket existence:\n{stdout}"
    );
    assert!(
        stdout.contains("\"scope_matches_socket\":null"),
        "missing unknown socket scope match:\n{stdout}"
    );
    assert!(
        stdout.contains("\"scope\":{\"kind\":\"socket\""),
        "missing socket scope:\n{stdout}"
    );
    assert!(
        stdout.contains("\"surfaces\":[{\"pane_id\":\"pane-1\""),
        "missing surface summary:\n{stdout}"
    );
    assert!(
        stdout.contains("\"scrollbacks\":[{\"pane_id\":\"pane-1\""),
        "missing scrollback summary:\n{stdout}"
    );

    let info_text = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--state-info",
        ])
        .output()
        .expect("run nmux --state-info");
    assert!(
        info_text.status.success(),
        "state-info failed: {}",
        String::from_utf8_lossy(&info_text.stderr)
    );
    let stdout = String::from_utf8_lossy(&info_text.stdout);
    assert!(stdout.contains("exists=true"), "missing exists:\n{stdout}");
    assert!(
        stdout.contains("socket_exists=false"),
        "missing socket existence:\n{stdout}"
    );
    assert!(
        stdout.contains("scope_matches_socket=unknown"),
        "missing unknown socket scope match:\n{stdout}"
    );
    assert!(
        stdout.contains("surface pane=pane-1"),
        "missing surface summary:\n{stdout}"
    );

    let _ = fs::remove_file(&state_path);
}

#[test]
fn state_info_reports_matching_live_socket_scope() {
    let socket_path = test_socket_path();
    let state_path = test_state_path();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-forever",
            "--command",
            "printf 'ready\n'; cat >/dev/null",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let attach = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--interval-ms",
            "100",
        ])
        .output()
        .expect("run nmux");
    assert!(
        attach.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&attach.stderr)
    );

    let info_json = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--state-info-json",
        ])
        .output()
        .expect("run nmux --state-info-json");

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        info_json.status.success(),
        "state-info-json failed: {}",
        String::from_utf8_lossy(&info_json.stderr)
    );
    let stdout = String::from_utf8_lossy(&info_json.stdout);
    assert!(
        stdout.contains("\"socket_exists\":true"),
        "missing socket existence:\n{stdout}"
    );
    assert!(
        stdout.contains("\"scope_matches_socket\":true"),
        "missing matching socket scope:\n{stdout}"
    );
}

#[test]
fn daemon_ready_json_reports_bound_socket_before_clients() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--ready-json",
            "--live-forever",
            "--command",
            "printf 'ready\n'; cat >/dev/null",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn daemon");

    let stdout = server.stdout.take().expect("server stdout");
    let mut lines = BufReader::new(stdout).lines();
    let ready = lines
        .next()
        .expect("ready json line")
        .expect("read ready json");
    let socket_was_ready = socket_path.exists();

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);

    assert!(
        ready.contains("\"event\":\"ready\""),
        "missing ready event:\n{ready}"
    );
    assert!(
        ready.contains(&format!(
            "\"NMUX_SOCKET\":\"{}\"",
            socket_path.to_str().expect("socket path")
        )),
        "missing socket path:\n{ready}"
    );
    assert!(
        ready.contains("\"source\":\"--socket\""),
        "missing socket source:\n{ready}"
    );
    assert!(
        ready.contains("\"mode\":\"live-forever\""),
        "missing daemon mode:\n{ready}"
    );
    #[cfg(feature = "libghostty-vt")]
    let expected_terminal_engine = "\"terminal_engine\":\"libghostty-vt\"";
    #[cfg(not(feature = "libghostty-vt"))]
    let expected_terminal_engine = "\"terminal_engine\":\"interim\"";
    assert!(
        ready.contains(expected_terminal_engine),
        "missing terminal engine:\n{ready}"
    );
    assert!(
        ready.contains("\"resize_policy\":\"fixed\""),
        "missing resize policy:\n{ready}"
    );
    assert!(socket_was_ready, "ready emitted before socket existed");
}

#[test]
fn one_shot_daemon_can_set_command_cwd_and_env() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let cwd_path = std::env::temp_dir().join(format!(
        "nmux-cwd-{}-{}",
        std::process::id(),
        socket_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("test")
    ));
    fs::create_dir_all(&cwd_path).expect("create cwd");
    let expected_cwd = fs::canonicalize(&cwd_path).expect("canonical cwd");

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--cols",
            "200",
            "--rows",
            "24",
            "--cwd",
            cwd_path.to_str().expect("cwd path"),
            "--env",
            "NMUX_TEST_VALUE=one=two",
            "--command",
            "printf 'cwd:%s env:%s\\n' \"$(pwd -P)\" \"$NMUX_TEST_VALUE\"; cat >/dev/null",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args(["--socket", socket_path.to_str().expect("socket path")])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_dir_all(&cwd_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains(&format!("cwd:{}", expected_cwd.display())),
        "missing cwd in output:\n{stdout}"
    );
    assert!(
        stdout.contains("env:one=two"),
        "missing env in output:\n{stdout}"
    );
}

#[test]
fn one_shot_json_cli_reports_protocol_error_object() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = "printf 'ready\\n'; cat >/dev/null";

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--json",
            "--focus",
            "gained",
        ])
        .output()
        .expect("run nmux --json --focus gained");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        !client.status.success(),
        "nmux unexpectedly succeeded:\n{}",
        String::from_utf8_lossy(&client.stdout)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.starts_with("{\"error\":{"),
        "missing JSON error object:\n{stdout}"
    );
    assert!(
        stdout.contains("\"code\":\"permission-denied\"")
            && stdout.contains("\"pane_id\":\"pane-1\"")
            && stdout.contains("\"input_seq\":1"),
        "missing structured JSON error attribution:\n{stdout}"
    );

    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("nmux: server error: input rejected: focus reporting is disabled"),
        "missing stderr error:\n{stderr}"
    );
}

#[test]
fn one_shot_cli_appends_inherited_nmux_origin() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = "printf 'origin:%s\\n' \"$NMUX_ORIGIN\"; cat >/dev/null";

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--command",
            command,
        ])
        .env("NMUX", "1")
        .env("NMUX_ORIGIN", "outer")
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args(["--socket", socket_path.to_str().expect("socket path")])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("origin:outer>local"),
        "missing chained origin:\n{stdout}"
    );
}

#[test]
fn one_shot_cli_can_print_nested_nmux_context() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = format!(
        "{} --print-context; cat >/dev/null",
        shell_quote(env!("CARGO_BIN_EXE_nmux"))
    );

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--command",
            &command,
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args(["--socket", socket_path.to_str().expect("socket path")])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(stdout.contains("NMUX=1"), "missing nmux flag:\n{stdout}");
    assert!(
        stdout.contains("NMUX_SESSION_ID=local"),
        "missing session id:\n{stdout}"
    );
    assert!(
        stdout.contains("NMUX_PANE_ID=pane-1"),
        "missing pane id:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("NMUX_SOCKET={}", socket_path.display())),
        "missing socket path:\n{stdout}"
    );
    assert!(
        stdout.contains("NMUX_ORIGIN=local"),
        "missing origin:\n{stdout}"
    );
}

#[test]
fn live_cli_streams_command_output_and_committed_resize() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(stdout.contains("session=local tab=tab-1 pane=pane-1 size=80x24 resize=fixed"));
    assert!(stdout.contains("session=local tab=tab-1 pane=pane-1 size=100x30 resize=fixed"));
    assert!(
        stdout.contains("echo:ping"),
        "missing streamed echo output:\n{stdout}"
    );
}

#[test]
fn managed_start_live_cli_runs_private_daemon() {
    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--start",
            "--live",
            "--iterations",
            "2",
            "--key",
            "managed\n",
            "--interval-ms",
            "100",
            "--command",
            "printf 'managed-ready\n'; while IFS= read -r line; do printf 'managed:%s\n' \"$line\"; done",
        ])
        .output()
        .expect("run nmux --start --live");

    assert!(
        client.status.success(),
        "nmux --start --live failed: {}\n{}",
        String::from_utf8_lossy(&client.stderr),
        String::from_utf8_lossy(&client.stdout)
    );

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("managed-ready"),
        "missing managed daemon output:\n{stdout}"
    );
    assert!(
        stdout.contains("managed:managed"),
        "missing managed input echo:\n{stdout}"
    );
}

#[test]
fn managed_start_live_cli_can_stream_json_events() {
    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--start",
            "--live",
            "--json",
            "--iterations",
            "2",
            "--key",
            "managed-json\n",
            "--interval-ms",
            "100",
            "--command",
            "printf 'managed-live-json-ready\n'; while IFS= read -r line; do printf 'managed-json:%s\n' \"$line\"; done",
        ])
        .output()
        .expect("run nmux --start --live --json");

    assert!(
        client.status.success(),
        "nmux --start --live --json failed: {}\n{}",
        String::from_utf8_lossy(&client.stderr),
        String::from_utf8_lossy(&client.stdout)
    );

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("{\"event\":\"attach\"")),
        "missing live attach JSON event:\n{stdout}"
    );
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("{\"event\":\"surface\"")),
        "missing live surface JSON event:\n{stdout}"
    );
    assert!(
        stdout.contains("managed-live-json-ready"),
        "missing managed live initial JSON output:\n{stdout}"
    );
    assert!(
        stdout.contains("managed-json:managed-json"),
        "missing managed live input JSON output:\n{stdout}"
    );
    assert!(
        stdout.contains("{\"event\":\"detach\",\"reason\":\"iteration-limit\"}"),
        "missing live detach JSON event:\n{stdout}"
    );
}

#[test]
fn managed_shell_cli_runs_private_live_daemon() {
    let mut client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--shell",
            "--iterations",
            "4",
            "--interval-ms",
            "250",
            "--command",
            "printf 'shell-ready\n'; while IFS= read -r line; do sleep 0.02; printf 'shell:%s\n' \"$line\"; done",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux --shell");
    client
        .stdin
        .as_mut()
        .expect("client stdin")
        .write_all(b"shell\n")
        .expect("write shell stdin");
    drop(client.stdin.take());
    let output = client.wait_with_output().expect("wait for nmux --shell");

    assert!(
        output.status.success(),
        "nmux --shell failed: {}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("shell-ready"),
        "missing shell daemon output:\n{stdout}"
    );
    assert!(
        stdout.contains("shell:shell"),
        "missing shell input echo:\n{stdout}"
    );
}

#[test]
fn bare_tty_nmux_starts_shared_default_session_and_can_reattach() {
    let socket_path = test_socket_path();
    let shell_path = socket_path.with_extension("shell");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&shell_path);
    fs::write(
        &shell_path,
        "#!/bin/sh\nprintf 'test-shell-ready\\n'\nwhile :; do sleep 1; done\n",
    )
    .expect("write test shell");
    fs::set_permissions(&shell_path, fs::Permissions::from_mode(0o755)).expect("chmod test shell");
    let socket = socket_path.to_str().expect("socket path");
    let shell = shell_path.to_str().expect("shell path");

    let mut client =
        spawn_nmux_client_in_pty_with_env(&[], &[("NMUX_SOCKET", socket), ("SHELL", shell)]);
    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(200));
    client.kill();
    let _ = client.wait();
    thread::sleep(Duration::from_millis(200));

    let reattach = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .env("NMUX_SOCKET", socket)
        .args([
            "--live",
            "--no-input",
            "--no-scrollback",
            "--iterations",
            "1",
            "--interval-ms",
            "100",
        ])
        .output()
        .expect("reattach to bare nmux daemon");

    assert!(
        reattach.status.success(),
        "reattach failed: {}\n{}",
        String::from_utf8_lossy(&reattach.stderr),
        String::from_utf8_lossy(&reattach.stdout)
    );
    assert!(
        String::from_utf8_lossy(&reattach.stdout).contains("session=local"),
        "reattach did not see shared session:\n{}",
        String::from_utf8_lossy(&reattach.stdout)
    );

    let kill = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .env("NMUX_SOCKET", socket)
        .arg("kill")
        .output()
        .expect("kill bare nmux daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&shell_path);

    assert!(
        kill.status.success(),
        "kill failed: {}\n{}",
        String::from_utf8_lossy(&kill.stderr),
        String::from_utf8_lossy(&kill.stdout)
    );
}

#[test]
fn bare_tty_nmux_starts_shell_at_client_terminal_size() {
    let socket_path = test_socket_path();
    let shell_path = socket_path.with_extension("shell");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&shell_path);
    fs::write(
        &shell_path,
        "#!/bin/sh\ncols=$(stty size 2>/dev/null | awk '{print $2}')\nprintf 'startup-cols:%s\\n' \"$cols\"\nwhile :; do sleep 1; done\n",
    )
    .expect("write test shell");
    fs::set_permissions(&shell_path, fs::Permissions::from_mode(0o755)).expect("chmod test shell");
    let socket = socket_path.to_str().expect("socket path");
    let shell = shell_path.to_str().expect("shell path");

    let mut client =
        spawn_nmux_client_in_pty_with_env(&[], &[("NMUX_SOCKET", socket), ("SHELL", shell)]);
    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(500));
    client.detach();
    let output = client.wait();

    let kill = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .env("NMUX_SOCKET", socket)
        .arg("kill")
        .output()
        .expect("kill bare nmux daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&shell_path);

    assert!(
        output.success,
        "detaching client failed:\n{}",
        output.output
    );
    assert!(
        output.output.contains("startup-cols:100"),
        "bare nmux did not start shell at the client PTY width:\n{}",
        output.output
    );
    assert!(
        kill.status.success(),
        "kill failed: {}\n{}",
        String::from_utf8_lossy(&kill.stderr),
        String::from_utf8_lossy(&kill.stdout)
    );
}

#[test]
fn bare_tty_nmux_detach_keeps_shared_default_daemon_running() {
    let socket_path = test_socket_path();
    let shell_path = socket_path.with_extension("shell");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&shell_path);
    fs::write(
        &shell_path,
        "#!/bin/sh\nprintf 'detach-ready\\n'\nwhile :; do sleep 1; done\n",
    )
    .expect("write test shell");
    fs::set_permissions(&shell_path, fs::Permissions::from_mode(0o755)).expect("chmod test shell");
    let socket = socket_path.to_str().expect("socket path");
    let shell = shell_path.to_str().expect("shell path");

    let mut client =
        spawn_nmux_client_in_pty_with_env(&[], &[("NMUX_SOCKET", socket), ("SHELL", shell)]);
    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(500));
    client.detach();
    let output = client.wait();

    assert!(
        output.success,
        "detaching client failed:\n{}",
        output.output
    );
    assert!(
        output.output.contains("detach-ready"),
        "detaching client did not render daemon output:\n{}",
        output.output
    );
    assert!(
        !output.output.contains("scrollback "),
        "bare redraw should not duplicate the active surface with an initial scrollback block:\n{}",
        output.output
    );
    assert!(
        socket_path.exists(),
        "detach removed shared daemon socket: {}",
        socket_path.display()
    );

    let reattach = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .env("NMUX_SOCKET", socket)
        .args([
            "--live",
            "--no-input",
            "--no-scrollback",
            "--iterations",
            "1",
            "--connect-timeout-ms",
            "5000",
        ])
        .output()
        .expect("reattach after local detach");

    let kill = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .env("NMUX_SOCKET", socket)
        .arg("kill")
        .output()
        .expect("kill bare nmux daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&shell_path);

    assert!(
        reattach.status.success(),
        "reattach after detach failed: {}\n{}",
        String::from_utf8_lossy(&reattach.stderr),
        String::from_utf8_lossy(&reattach.stdout)
    );
    assert!(
        String::from_utf8_lossy(&reattach.stdout).contains("session=local"),
        "reattach did not see shared session:\n{}",
        String::from_utf8_lossy(&reattach.stdout)
    );
    assert!(
        kill.status.success(),
        "kill failed: {}\n{}",
        String::from_utf8_lossy(&kill.stderr),
        String::from_utf8_lossy(&kill.stdout)
    );
}

#[test]
fn bare_tty_nmux_replaces_default_daemon_after_pane_exit() {
    let socket_path = test_socket_path();
    let stale_shell_path = socket_path.with_extension("stale-shell");
    let fresh_shell_path = socket_path.with_extension("fresh-shell");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&stale_shell_path);
    let _ = fs::remove_file(&fresh_shell_path);
    fs::write(
        &stale_shell_path,
        "#!/bin/sh\nprintf 'stale-ready\\n'\nexit 0\n",
    )
    .expect("write stale test shell");
    fs::write(
        &fresh_shell_path,
        "#!/bin/sh\nprintf 'fresh-ready\\n'\nwhile :; do sleep 1; done\n",
    )
    .expect("write fresh test shell");
    fs::set_permissions(&stale_shell_path, fs::Permissions::from_mode(0o755))
        .expect("chmod stale test shell");
    fs::set_permissions(&fresh_shell_path, fs::Permissions::from_mode(0o755))
        .expect("chmod fresh test shell");
    let socket = socket_path.to_str().expect("socket path");
    let stale_shell = stale_shell_path.to_str().expect("stale shell path");
    let fresh_shell = fresh_shell_path.to_str().expect("fresh shell path");

    let mut stale_daemon = daemon_command()
        .args([
            "--socket",
            socket,
            "--live-forever",
            "--command",
            stale_shell,
        ])
        .spawn()
        .expect("spawn stale daemon");
    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(500));

    let mut client =
        spawn_nmux_client_in_pty_with_env(&[], &[("NMUX_SOCKET", socket), ("SHELL", fresh_shell)]);
    thread::sleep(Duration::from_millis(500));
    client.detach();
    let output = client.wait();

    let kill = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .env("NMUX_SOCKET", socket)
        .arg("kill")
        .output()
        .expect("kill replacement daemon");
    let _ = stale_daemon.kill();
    let _ = stale_daemon.wait();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&stale_shell_path);
    let _ = fs::remove_file(&fresh_shell_path);

    assert!(
        output.success,
        "reattach through stale default daemon failed:\n{}",
        output.output
    );
    assert!(
        output.output.contains("fresh-ready"),
        "client did not attach to replacement daemon:\n{}",
        output.output
    );
    assert!(
        !output.output.contains("pane process is not running"),
        "client saw stale pane failure instead of replacing daemon:\n{}",
        output.output
    );
    assert!(
        kill.status.success(),
        "kill failed: {}\n{}",
        String::from_utf8_lossy(&kill.stderr),
        String::from_utf8_lossy(&kill.stdout)
    );
}

#[test]
fn bare_tty_nmux_health_probe_does_not_resize_existing_daemon() {
    let socket_path = test_socket_path();
    let stale_shell_path = socket_path.with_extension("stale-shell");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&stale_shell_path);
    fs::write(
        &stale_shell_path,
        "#!/bin/sh\nprintf 'resize-exit-ready\\n'\ninitial=$(stty size 2>/dev/null || true)\nwhile :; do current=$(stty size 2>/dev/null || true); [ -n \"$initial\" ] && [ \"$current\" != \"$initial\" ] && exit 0; sleep 0.05; done\n",
    )
    .expect("write stale test shell");
    fs::set_permissions(&stale_shell_path, fs::Permissions::from_mode(0o755))
        .expect("chmod stale test shell");
    let socket = socket_path.to_str().expect("socket path");
    let stale_shell = stale_shell_path.to_str().expect("stale shell path");

    let mut daemon = daemon_command()
        .args([
            "--socket",
            socket,
            "--live-forever",
            "--command",
            stale_shell,
        ])
        .spawn()
        .expect("spawn stale daemon");
    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(200));

    let mut client = spawn_nmux_client_in_pty_with_env(&[], &[("NMUX_SOCKET", socket)]);
    thread::sleep(Duration::from_millis(700));
    client.detach();
    let output = client.wait();

    let kill = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .env("NMUX_SOCKET", socket)
        .arg("kill")
        .output()
        .expect("kill daemon");
    let _ = daemon.kill();
    let _ = daemon.wait();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&stale_shell_path);

    assert!(
        output.success,
        "bare attach failed after health probe:\n{}",
        output.output
    );
    assert!(
        output.output.contains("resize-exit-ready"),
        "client did not attach to existing daemon:\n{}",
        output.output
    );
    assert!(
        !output.output.contains("fresh-after-resize-ready"),
        "health probe unexpectedly replaced the existing daemon:\n{}",
        output.output
    );
    assert!(
        kill.status.success(),
        "kill failed: {}\n{}",
        String::from_utf8_lossy(&kill.stderr),
        String::from_utf8_lossy(&kill.stdout)
    );
}

#[test]
fn bare_tty_nmux_allows_two_shared_default_attachers() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let socket = socket_path.to_str().expect("socket path");
    let mut server = daemon_command()
        .args([
            "--socket",
            socket,
            "--live-forever",
            "--command",
            "printf 'two-client-ready\n'; while :; do sleep 1; done",
        ])
        .spawn()
        .expect("spawn daemon");
    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(200));

    let mut first = spawn_nmux_client_in_pty_with_env(&[], &[("NMUX_SOCKET", socket)]);
    thread::sleep(Duration::from_millis(200));

    let mut second = spawn_nmux_client_in_pty_with_env(&[], &[("NMUX_SOCKET", socket)]);
    thread::sleep(Duration::from_millis(500));

    first.detach();
    second.detach();
    let first_output = first.wait();
    let second_output = second.wait();

    let _ = server.kill();
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        first_output.success,
        "first client failed:\n{}",
        first_output.output
    );
    assert!(
        second_output.success,
        "second client failed:\n{}",
        second_output.output
    );
    assert!(
        first_output.output.contains("two-client-ready"),
        "first client did not render shared shell output:\n{}",
        first_output.output
    );
    assert!(
        second_output.output.contains("two-client-ready"),
        "second client did not render shared shell output:\n{}",
        second_output.output
    );
    assert!(
        !first_output.output.contains("wire I/O failed"),
        "first client hit wire error:\n{}",
        first_output.output
    );
    assert!(
        !second_output.output.contains("wire I/O failed"),
        "second client hit wire error:\n{}",
        second_output.output
    );
    assert!(!server_status.success(), "daemon should be killed by test");
}

#[test]
fn live_tty_client_resize_updates_daemon_pane_size() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let socket = socket_path.to_str().expect("socket path");
    let mut server = daemon_command()
        .args([
            "--socket",
            socket,
            "--live-forever",
            "--command",
            "printf 'resize-ready\n'; while :; do sleep 1; done",
        ])
        .spawn()
        .expect("spawn daemon");
    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(200));

    let mut client = spawn_nmux_client_in_pty_with_env(
        &["--live", "--stdin-bytes", "--redraw"],
        &[("NMUX_SOCKET", socket)],
    );
    thread::sleep(Duration::from_millis(500));
    client.resize(72, 19);
    thread::sleep(Duration::from_millis(500));
    client.detach();
    let client_output = client.wait();

    let observe = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .env("NMUX_SOCKET", socket)
        .args([
            "--live",
            "--no-input",
            "--no-scrollback",
            "--iterations",
            "1",
            "--connect-timeout-ms",
            "5000",
        ])
        .output()
        .expect("observe resized daemon");

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);

    assert!(
        client_output.success,
        "resizing client failed:\n{}",
        client_output.output
    );
    assert!(
        observe.status.success(),
        "observe failed: {}\n{}",
        String::from_utf8_lossy(&observe.stderr),
        String::from_utf8_lossy(&observe.stdout)
    );
    let stdout = String::from_utf8_lossy(&observe.stdout);
    assert!(
        stdout.contains("session=local tab=tab-1 pane=pane-1 size=72x19 resize=fixed"),
        "daemon did not commit tty resize:\n{stdout}"
    );
}

#[test]
fn live_tty_attach_reflows_existing_wrapped_output_to_client_width() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let socket = socket_path.to_str().expect("socket path");
    let long_line = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let command = format!("printf '{long_line}\\n'; while :; do sleep 1; done");
    let mut server = daemon_command()
        .args([
            "--socket",
            socket,
            "--live-forever",
            "--cols",
            "20",
            "--rows",
            "8",
            "--command",
            &command,
        ])
        .spawn()
        .expect("spawn daemon");
    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(500));

    let mut client = spawn_nmux_client_in_pty_with_env(
        &[
            "--live",
            "--stdin-bytes",
            "--redraw",
            "--connect-timeout-ms",
            "5000",
        ],
        &[("NMUX_SOCKET", socket)],
    );
    thread::sleep(Duration::from_millis(700));
    client.detach();
    let client_output = client.wait();

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);

    assert!(
        client_output.success,
        "wide attach client failed:\n{}",
        client_output.output
    );
    assert!(
        client_output
            .output
            .lines()
            .any(|line| line.contains(long_line)),
        "wide attach did not reflow existing wrapped output:\n{}",
        client_output.output
    );
}

#[test]
fn live_tty_client_records_resize_without_followup_input() {
    let socket_path = test_socket_path();
    let record_path = socket_path.with_extension("record.jsonl");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&record_path);
    let socket = socket_path.to_str().expect("socket path");
    let record = record_path.to_str().expect("record path");
    let mut server = daemon_command()
        .args([
            "--socket",
            socket,
            "--live-forever",
            "--command",
            "printf 'resize-record-ready\n'; while :; do sleep 1; done",
        ])
        .spawn()
        .expect("spawn daemon");
    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(200));

    let mut client = spawn_nmux_client_in_pty_with_env(
        &["--live", "--stdin-bytes", "--redraw", "--record", record],
        &[("NMUX_SOCKET", socket)],
    );
    thread::sleep(Duration::from_millis(200));
    client.resize(72, 19);

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut record_contents = String::new();
    while Instant::now() < deadline {
        record_contents = fs::read_to_string(&record_path).unwrap_or_default();
        if record_contents.contains(
            "\"event\":\"workspace\",\"workspace\":{\"session_id\":\"local\",\"tab_id\":\"tab-1\",\"pane_id\":\"pane-1\",\"cols\":72,\"rows\":19,\"resize_policy\":\"fixed\"}",
        ) {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    client.detach();
    let client_output = client.wait();

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&record_path);

    assert!(
        client_output.success,
        "resizing client failed:\n{}",
        client_output.output
    );
    assert!(
        record_contents.contains(
            "\"event\":\"workspace\",\"workspace\":{\"session_id\":\"local\",\"tab_id\":\"tab-1\",\"pane_id\":\"pane-1\",\"cols\":72,\"rows\":19,\"resize_policy\":\"fixed\"}"
        ),
        "client did not record resize before follow-up input:\nrecord:\n{record_contents}\noutput:\n{}",
        client_output.output
    );
}

#[test]
fn live_tty_client_keeps_rendering_after_resize_without_input() {
    let socket_path = test_socket_path();
    let record_path = socket_path.with_extension("record.jsonl");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&record_path);
    let socket = socket_path.to_str().expect("socket path");
    let record = record_path.to_str().expect("record path");
    let mut server = daemon_command()
        .args([
            "--socket",
            socket,
            "--live-forever",
            "--command",
            "printf 'pre-resize-ready\n'; sleep 0.2; printf 'post-resize-tick\n'; while :; do sleep 1; done",
        ])
        .spawn()
        .expect("spawn daemon");
    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(200));

    let mut client = spawn_nmux_client_in_pty_with_env(
        &["--live", "--stdin-bytes", "--redraw", "--record", record],
        &[("NMUX_SOCKET", socket)],
    );
    thread::sleep(Duration::from_millis(100));
    client.resize(72, 19);

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut record = String::new();
    while Instant::now() < deadline {
        record = fs::read_to_string(&record_path).unwrap_or_default();
        if record.contains("post-resize-tick") {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    client.detach();
    let client_output = client.wait();

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&record_path);

    assert!(
        client_output.success,
        "resizing client failed:\n{}",
        client_output.output
    );
    assert!(
        record.contains("post-resize-tick"),
        "client stopped rendering after resize without input:\nrecord:\n{record}\noutput:\n{}",
        client_output.output
    );
    assert!(
        client_output.output.contains("post-resize-tick"),
        "client recorded but did not render output after resize without input:\nrecord:\n{record}\noutput:\n{}",
        client_output.output
    );
}

#[test]
fn live_redraw_tty_clears_stale_rows_after_resize() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let socket = socket_path.to_str().expect("socket path");
    let mut server = daemon_command()
        .args([
            "--socket",
            socket,
            "--live-forever",
            "--command",
            "printf 'resize-clear-ready\n'; while :; do sleep 1; done",
        ])
        .spawn()
        .expect("spawn daemon");
    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(200));

    let mut client = spawn_nmux_client_in_pty_with_env(
        &["--live", "--stdin-bytes", "--redraw"],
        &[("NMUX_SOCKET", socket)],
    );
    thread::sleep(Duration::from_millis(300));
    client.resize(72, 19);
    thread::sleep(Duration::from_millis(500));
    client.detach();
    let output = client.wait();

    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);

    assert!(output.success, "nmux failed:\n{}", output.output);
    assert!(
        output.output.contains("resize-clear-ready"),
        "missing daemon output:\n{}",
        output.output
    );
    let clear_count = output.output.matches("\x1b[2J").count();
    assert!(
        clear_count >= 2,
        "redraw resize should clear stale terminal rows with a full repaint; clear_count={clear_count}\n{}",
        output.output
    );
}

#[test]
fn live_tty_client_initial_size_renders_before_input() {
    let socket_path = test_socket_path();
    let record_path = socket_path.with_extension("record.jsonl");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&record_path);
    let socket = socket_path.to_str().expect("socket path");
    let record = record_path.to_str().expect("record path");
    let mut server = daemon_command()
        .args([
            "--socket",
            socket,
            "--live-forever",
            "--command",
            "printf 'initial-size-ready\n'; while :; do sleep 1; done",
        ])
        .spawn()
        .expect("spawn daemon");
    wait_for_socket(&socket_path);
    thread::sleep(Duration::from_millis(200));

    let mut client = spawn_nmux_client_in_pty_with_env(
        &["--live", "--stdin-bytes", "--redraw", "--record", record],
        &[("NMUX_SOCKET", socket)],
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut record = String::new();
    while Instant::now() < deadline {
        record = fs::read_to_string(&record_path).unwrap_or_default();
        if record.contains(
            "\"event\":\"workspace\",\"workspace\":{\"session_id\":\"local\",\"tab_id\":\"tab-1\",\"pane_id\":\"pane-1\",\"cols\":100,\"rows\":24,\"resize_policy\":\"fixed\"}",
        ) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    client.kill();
    let client_output = client.wait();
    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&record_path);

    assert!(
        record.contains(
            "\"event\":\"workspace\",\"workspace\":{\"session_id\":\"local\",\"tab_id\":\"tab-1\",\"pane_id\":\"pane-1\",\"cols\":100,\"rows\":24,\"resize_policy\":\"fixed\"}"
        ),
        "client did not render initial tty resize before input:\nrecord:\n{record}\noutput:\n{}",
        client_output.output
    );
}

#[test]
fn managed_start_one_shot_cli_runs_private_daemon() {
    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--start",
            "--startup-timeout-ms",
            "10000",
            "--command",
            "printf 'managed-one-ready\n'; cat >/dev/null",
        ])
        .output()
        .expect("run nmux --start");

    assert!(
        client.status.success(),
        "nmux --start failed: {}\n{}",
        String::from_utf8_lossy(&client.stderr),
        String::from_utf8_lossy(&client.stdout)
    );

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("managed-one-ready"),
        "missing managed one-shot daemon output:\n{stdout}"
    );
}

#[test]
fn managed_start_one_shot_cli_can_print_attach_json() {
    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--start",
            "--json",
            "--command",
            "printf 'managed-json-ready\n'; cat >/dev/null",
        ])
        .output()
        .expect("run nmux --start --json");

    assert!(
        client.status.success(),
        "nmux --start --json failed: {}\n{}",
        String::from_utf8_lossy(&client.stderr),
        String::from_utf8_lossy(&client.stdout)
    );

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.starts_with("{\"workspace\":{"),
        "not JSON attach output:\n{stdout}"
    );
    assert!(
        stdout.contains("managed-json-ready"),
        "missing managed one-shot JSON output:\n{stdout}"
    );
}

#[test]
fn managed_start_json_reports_ready_error_without_nested_json() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    fs::write(&socket_path, b"not a socket").expect("create blocking socket path");

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--start",
            "--json",
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--command",
            "printf 'unreachable\n'",
        ])
        .output()
        .expect("run nmux --start --json with blocked socket path");

    let _ = fs::remove_file(&socket_path);

    assert!(
        !client.status.success(),
        "nmux unexpectedly succeeded:\n{}",
        String::from_utf8_lossy(&client.stdout)
    );
    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.starts_with(
            "{\"error\":{\"message\":\"managed daemon startup failed: socket path already exists:"
        ),
        "missing clean managed startup JSON error:\n{stdout}"
    );
    assert!(
        !stdout.contains("\\\"event\\\":\\\"error\\\""),
        "managed startup error nested daemon JSON instead of extracting message:\n{stdout}"
    );
    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("nmux: managed daemon startup failed: socket path already exists:"),
        "missing stderr managed startup context:\n{stderr}"
    );
}

#[test]
fn managed_start_passes_cwd_and_env_to_private_daemon() {
    let id = NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed);
    let cwd = PathBuf::from(format!("/tmp/nmux-managed-cwd-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&cwd);
    fs::create_dir(&cwd).expect("create managed cwd");
    let expected_cwd = fs::canonicalize(&cwd).expect("canonical cwd");

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--start",
            "--cwd",
            cwd.to_str().expect("cwd path"),
            "--env",
            "NMUX_MANAGED_TEST=visible",
            "--command",
            "printf 'cwd:%s env:%s\n' \"$PWD\" \"$NMUX_MANAGED_TEST\"; cat >/dev/null",
        ])
        .output()
        .expect("run nmux --start with cwd/env");

    let _ = fs::remove_dir_all(&cwd);

    assert!(
        client.status.success(),
        "nmux --start --cwd --env failed: {}\n{}",
        String::from_utf8_lossy(&client.stderr),
        String::from_utf8_lossy(&client.stdout)
    );

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains(&format!("cwd:{} env:visible", expected_cwd.display())),
        "missing managed cwd/env output:\n{stdout}"
    );
}

#[test]
fn live_cli_can_stream_json_events() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'json-ready\n'; while IFS= read -r line; do printf 'json:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--json",
            "--iterations",
            "2",
            "--key",
            "ping\n",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux --live --json");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --live --json failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    let mut lines = stdout.lines();
    let first = lines.next().expect("initial json event");
    assert!(
        first.starts_with("{\"event\":\"attach\""),
        "missing attach event:\n{stdout}"
    );
    assert!(
        stdout.contains("\"event\":\"surface\""),
        "missing surface event:\n{stdout}"
    );
    assert!(
        stdout.contains("\"rows\":["),
        "missing structured row updates:\n{stdout}"
    );
    assert!(
        stdout.contains("\"runs\":["),
        "missing structured row runs:\n{stdout}"
    );
    assert!(
        stdout.contains("json:ping"),
        "missing streamed output:\n{stdout}"
    );
    assert!(
        stdout
            .lines()
            .last()
            .is_some_and(|line| line == "{\"event\":\"detach\",\"reason\":\"iteration-limit\"}"),
        "missing final detach event:\n{stdout}"
    );
}

#[test]
fn live_cli_can_record_timestamped_json_events() {
    let socket_path = test_socket_path();
    let record_path = test_state_path();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&record_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "1",
            "--command",
            "printf 'record-ready\n'; sleep 0.05; printf 'record-update\n'; sleep 0.2",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--record",
            record_path.to_str().expect("record path"),
            "--iterations",
            "3",
            "--interval-ms",
            "50",
            "--no-scrollback",
        ])
        .output()
        .expect("run nmux --live --record");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --live --record failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let record = fs::read_to_string(&record_path).expect("read record file");
    assert!(
        record
            .lines()
            .all(|line| line.starts_with("{\"elapsed_ms\":")),
        "record events should include elapsed timestamps:\n{record}"
    );
    assert!(
        record.contains("\"event\":\"attach\""),
        "missing attach event:\n{record}"
    );
    assert!(
        record.contains("\"event\":\"presence\""),
        "missing presence event:\n{record}"
    );
    assert!(
        record.contains("record-update"),
        "missing recorded surface text:\n{record}"
    );
    assert!(
        record.contains("\"event\":\"detach\""),
        "missing detach event:\n{record}"
    );

    let replay = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args(["replay", record_path.to_str().expect("record path")])
        .output()
        .expect("run nmux replay");
    assert!(
        replay.status.success(),
        "nmux replay failed: {}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let replay_stdout = String::from_utf8_lossy(&replay.stdout);
    assert!(
        replay_stdout.contains("record-ready") && replay_stdout.contains("record-update"),
        "replay should print recorded surface text:\n{replay_stdout}"
    );
    let _ = fs::remove_file(&record_path);
}

#[test]
fn live_json_reports_server_closed_detach() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "sleep 0.3; printf 'closing-soon\n'; sleep 0.05",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--json",
            "--interval-ms",
            "1000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux --live --json");

    let client = wait_for_command_output(client, "nmux --live --json", Duration::from_secs(10));
    let server_status = wait_for_child_exit(&mut server, Duration::from_secs(10));
    if server_status.is_none() {
        let _ = server.kill();
    }
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --live --json failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    if let Some(server_status) = server_status {
        assert!(server_status.success(), "daemon failed: {server_status}");
    }

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout
            .lines()
            .last()
            .is_some_and(|line| line == "{\"event\":\"detach\",\"reason\":\"server-closed\"}"),
        "missing server-closed detach event:\n{stdout}"
    );
}

#[test]
fn live_json_reports_stdin_eof_detach() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'json-stdin-ready\n'; while IFS= read -r line; do printf 'json-stdin:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let mut client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--json",
            "--stdin",
            "--interval-ms",
            "1000",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux --live --json --stdin");

    let mut stdin = client.stdin.take().expect("client stdin");
    stdin.write_all(b"ping\n").expect("write stdin");
    drop(stdin);

    let client = client.wait_with_output().expect("wait for nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --live --json --stdin failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("json-stdin:ping"),
        "missing stdin echo:\n{stdout}"
    );
    assert!(
        stdout
            .lines()
            .last()
            .is_some_and(|line| line == "{\"event\":\"detach\",\"reason\":\"stdin-eof\"}"),
        "missing stdin-eof detach event:\n{stdout}"
    );
}

#[test]
fn live_cli_commits_explicit_resize_without_pane_input() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "2",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "2",
            "--cols",
            "100",
            "--rows",
            "30",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(stdout.contains("session=local tab=tab-1 pane=pane-1 size=80x24 resize=fixed"));
    assert!(
        stdout.contains("session=local tab=tab-1 pane=pane-1 size=100x30 resize=fixed"),
        "missing committed resize without pane input:\n{stdout}"
    );
    assert!(
        !stdout.contains("echo:"),
        "resize-only client should not send pane input:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_streams_command_output_and_committed_resize() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf '\\033[31mready\\033[0m\n'; while IFS= read -r line; do printf '\\033[32mecho:%s\\033[0m\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(stdout.contains("session=local tab=tab-1 pane=pane-1 size=80x24 resize=fixed"));
    assert!(stdout.contains("session=local tab=tab-1 pane=pane-1 size=100x30 resize=fixed"));
    assert!(
        stdout.contains("echo:ping"),
        "missing streamed echo output:\n{stdout}"
    );
    assert!(
        !stdout.contains("[31m") && !stdout.contains("[32m") && !stdout.contains("[0m"),
        "ANSI control sequences leaked into resized libghostty-vt output:\n{stdout}"
    );
}

#[test]
fn live_cli_forwards_paste_input() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'paste:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--paste",
            "clip\n",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("paste:clip"),
        "missing pasted output:\n{stdout}"
    );
}

#[test]
fn live_cli_forwards_named_keypad_enter_in_normal_mode() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; IFS= read -r line; printf 'key:%s\n' \"$line\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--key-name",
            "keypad-enter",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("key:"),
        "missing keypad enter output:\n{stdout}"
    );
}

#[test]
fn live_cli_forwards_named_delete_key() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "stty -icanon -echo min 4 time 20; printf 'ready\n'; bytes=$(dd bs=4 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'delete:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--key-name",
            "delete",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("delete:1b5b337e"),
        "missing delete key bytes:\n{stdout}"
    );
}

#[test]
fn live_cli_forwards_repeated_named_keys_in_order() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "stty -icanon -echo min 5 time 20; printf 'ready\n'; bytes=$(dd bs=5 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'keys:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--key-name",
            "esc",
            "--key-name",
            "delete",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("keys:1b1b5b337e"),
        "missing repeated key bytes:\n{stdout}"
    );
}

#[test]
fn live_cli_displays_daemon_resize_policy() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
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
        .expect("spawn daemon");

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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

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

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

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

    let server_status = server.wait().expect("wait for daemon");

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");
    assert!(
        !socket_path.exists(),
        "daemon left socket after bounded exit: {}",
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

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    let client = client.wait_with_output().expect("wait for nmux");
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(server_status.success(), "daemon failed: {server_status}");
    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    let stdout = String::from_utf8_lossy(&client.stdout);
    assert_default_workspace_attached(&stdout, "client did not attach after waiting for socket");
}

#[test]
fn one_shot_cli_can_wait_for_daemon_socket() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--connect-timeout-ms",
            "2000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux");

    thread::sleep(Duration::from_millis(100));

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    let client = client.wait_with_output().expect("wait for nmux");
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(server_status.success(), "daemon failed: {server_status}");
    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    let stdout = String::from_utf8_lossy(&client.stdout);
    assert_default_workspace_attached(
        &stdout,
        "one-shot client did not attach after waiting for socket",
    );
}

#[test]
fn follow_cli_can_wait_for_daemon_socket() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--connect-timeout-ms",
            "2000",
            "--follow",
            "--iterations",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux");

    thread::sleep(Duration::from_millis(100));

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    let client = client.wait_with_output().expect("wait for nmux");
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(server_status.success(), "daemon failed: {server_status}");
    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    let stdout = String::from_utf8_lossy(&client.stdout);
    assert_default_workspace_attached(
        &stdout,
        "follow client did not attach after waiting for socket",
    );
}

#[test]
fn follow_json_cli_can_wait_for_daemon_socket() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--connect-timeout-ms",
            "2000",
            "--follow",
            "--json",
            "--iterations",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux");

    thread::sleep(Duration::from_millis(100));

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    let client = client.wait_with_output().expect("wait for nmux");
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(server_status.success(), "daemon failed: {server_status}");
    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\"workspace\""),
        "follow JSON client did not print attach JSON:\n{stdout}"
    );
    assert!(
        stdout.contains("\"surface_text\":"),
        "follow JSON client did not include rendered surface text field:\n{stdout}"
    );
}

#[test]
fn live_cli_renders_initial_scrollback_range() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--terminal-engine",
            "interim",
            "--command",
            "printf 'one\ntwo\nthree\nfour\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

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
            "4",
            "--scrollback-count",
            "2",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("scrollback 4..5 of 7:"),
        "missing requested live scrollback header:\n{stdout}"
    );
    assert!(
        stdout.contains("one"),
        "missing live scrollback command output:\n{stdout}"
    );
    assert!(
        stdout.contains("two"),
        "missing live scrollback command output:\n{stdout}"
    );
}

#[test]
fn live_cli_can_skip_initial_scrollback_fetch() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'visible-live\nhistory-live\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--no-scrollback",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux live --no-scrollback");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux live --no-scrollback failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("visible-live"),
        "missing live visible output:\n{stdout}"
    );
    assert!(
        !stdout.contains("scrollback "),
        "unexpected live scrollback block:\n{stdout}"
    );
}

#[test]
fn live_cli_renders_initial_scrollback_tail() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--terminal-engine",
            "interim",
            "--command",
            "printf 'one\ntwo\nthree\nfour\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--scrollback-tail",
            "2",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("scrollback 6..7:"),
        "missing requested live scrollback tail header:\n{stdout}"
    );
    assert!(
        stdout.contains("three"),
        "missing live scrollback tail output:\n{stdout}"
    );
    assert!(
        stdout.contains("four"),
        "missing live scrollback tail output:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_cli_can_use_libghostty_vt_terminal_engine() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
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
        .expect("spawn daemon");

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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

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

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_replies_to_terminal_query() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "6",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty raw -echo; printf '\\033[?7$p'; reply=$(dd bs=1 count=8 2>/dev/null | od -An -tx1 | tr -d ' \\n'); printf '\\r\\nreply:%s\\r\\n' \"$reply\"; stty sane; sleep 0.2",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-input",
            "--iterations",
            "6",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("reply:1b5b3f373b312479"),
        "missing DECRQM terminal query reply:\n{stdout}"
    );
    assert!(
        !stdout.contains("\x1b[?7$p") && !stdout.contains("\x1b[?7;1$y"),
        "terminal query/reply controls leaked into rendered output:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_omits_alternate_screen_from_scrollback() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf 'main-before\n\\033[?1049h\\033[Halt-only\n\\033[?1049lmain-after\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

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
            "10",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("main-before"),
        "missing main-screen history before alternate screen:\n{stdout}"
    );
    assert!(
        stdout.contains("main-after"),
        "missing restored main-screen output:\n{stdout}"
    );
    assert!(
        !stdout.contains("alt-only"),
        "alternate-screen output leaked into libghostty-vt scrollback:\n{stdout}"
    );
    assert!(
        !stdout.contains("[?1049h") && !stdout.contains("[?1049l"),
        "alternate-screen controls leaked into libghostty-vt output:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_prints_terminal_metadata() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf '\\033]2;nmux live title\\033\\\\\\033]7;file://localhost/tmp/nmux\\007ready\\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("title=nmux live title"),
        "missing title metadata:\n{stdout}"
    );
    assert!(
        stdout.contains("working-directory=file://localhost/tmp/nmux"),
        "missing working-directory metadata:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_prints_metadata_only_update_without_reprinting_rows() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "4",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf 'ready\\n'; sleep 0.3; printf '\\033]2;metadata only\\033\\\\'; sleep 0.3",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-input",
            "--iterations",
            "4",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("title=metadata only"),
        "missing metadata-only title update:\n{stdout}"
    );
    let metadata_update = stdout
        .find("title=metadata only")
        .expect("metadata-only title update");
    assert!(
        !stdout[metadata_update..].contains("ready"),
        "metadata-only update reprinted unchanged row text:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_persists_metadata_only_update_to_state() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf 'ready\\n'; sleep 0.3; printf '\\033]2;patched metadata\\033\\\\\\033]7;file://localhost/tmp/patched\\007'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "4",
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
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("ready"),
        "reattached client did not render cached surface text:\n{stdout}"
    );
    assert!(
        stdout.contains("title=patched metadata"),
        "reattached client did not render persisted metadata-only title:\n{stdout}"
    );
    assert!(
        stdout.contains("working-directory=file://localhost/tmp/patched"),
        "reattached client did not render persisted metadata-only working directory:\n{stdout}"
    );
    assert!(
        !stdout.contains("\x1b]2;") && !stdout.contains("\x1b]7;"),
        "metadata control sequences leaked after state reattach:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_persists_cursor_only_update_to_state() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf 'ready\\n'; sleep 0.3; printf '\\033[2;5H'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "4",
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

    let first_stdout = String::from_utf8_lossy(&first_client.stdout);
    assert!(
        first_stdout.contains("ready"),
        "first client did not render initial row text:\n{first_stdout}"
    );
    assert!(
        !first_stdout.contains("[2;5H"),
        "cursor-only control leaked into first render:\n{first_stdout}"
    );

    let first_state = fs::read_to_string(&state_path).expect("read first state");
    assert!(
        first_state.contains("cursor 1 4 1 0 0\n"),
        "first state did not persist cursor-only update:\n{first_state}"
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
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let second_stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        second_stdout.contains("ready"),
        "reattached client did not render cached row text:\n{second_stdout}"
    );
    assert!(
        !second_stdout.contains("[2;5H"),
        "cursor-only control leaked after state reattach:\n{second_stdout}"
    );

    let second_state = fs::read_to_string(&state_path).expect("read second state");
    assert!(
        second_state.contains("cursor 1 4 1 0 0\n"),
        "second state did not preserve cursor-only update:\n{second_state}"
    );

    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_persists_color_only_update_to_state() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf 'ready\\n'; sleep 0.3; printf '\\033[?2004h\\033]12;#ff00ff\\033\\\\\\033]4;1;#112233\\033\\\\'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "4",
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

    let first_stdout = String::from_utf8_lossy(&first_client.stdout);
    assert!(
        first_stdout.contains("ready"),
        "first client did not render initial row text:\n{first_stdout}"
    );
    assert_eq!(
        first_stdout.matches("ready").count(),
        2,
        "color-only update reprinted unchanged row text:\n{first_stdout}"
    );
    assert!(
        !first_stdout.contains("[?2004h")
            && !first_stdout.contains("\x1b]12;")
            && !first_stdout.contains("\x1b]4;"),
        "color controls leaked into first render:\n{first_stdout}"
    );

    let first_state = fs::read_to_string(&state_path).expect("read first state");
    assert!(
        first_state.contains("4278255615 1 "),
        "first state did not persist explicit cursor color:\n{first_state}"
    );
    assert!(
        first_state.contains("112233ff"),
        "first state did not persist palette-diff materialized color:\n{first_state}"
    );
    assert!(
        first_state.contains("modes 1 0 0 0 0 0 1 0 0\n"),
        "first state did not persist mode payload from color-only update:\n{first_state}"
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
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let second_stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        second_stdout.contains("ready"),
        "reattached client did not render cached row text:\n{second_stdout}"
    );
    assert!(
        !second_stdout.contains("[?2004h")
            && !second_stdout.contains("\x1b]12;")
            && !second_stdout.contains("\x1b]4;"),
        "color controls leaked after state reattach:\n{second_stdout}"
    );

    let second_state = fs::read_to_string(&state_path).expect("read second state");
    assert!(
        second_state.contains("4278255615 1 "),
        "second state did not preserve explicit cursor color:\n{second_state}"
    );
    assert!(
        second_state.contains("112233ff"),
        "second state did not preserve palette-diff materialized color:\n{second_state}"
    );
    assert!(
        second_state.contains("modes 1 0 0 0 0 0 1 0 0\n"),
        "second state did not preserve mode payload from color-only update:\n{second_state}"
    );

    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_persists_mode_only_update_to_state() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf 'ready\\n'; sleep 0.3; printf '\\033[?2004h\\033[?1004h'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "4",
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

    let first_stdout = String::from_utf8_lossy(&first_client.stdout);
    assert!(
        first_stdout.contains("ready"),
        "first client did not render initial row text:\n{first_stdout}"
    );
    assert_eq!(
        first_stdout.matches("ready").count(),
        2,
        "mode-only update reprinted unchanged row text:\n{first_stdout}"
    );
    assert!(
        !first_stdout.contains("[?2004h") && !first_stdout.contains("[?1004h"),
        "mode controls leaked into first render:\n{first_stdout}"
    );

    let first_state = fs::read_to_string(&state_path).expect("read first state");
    assert!(
        first_state.contains("modes 1 0 1 0 0 0 1 0 0\n"),
        "first state did not persist mode-only update:\n{first_state}"
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
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let second_stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        second_stdout.contains("ready"),
        "reattached client did not render cached row text:\n{second_stdout}"
    );
    assert!(
        !second_stdout.contains("[?2004h") && !second_stdout.contains("[?1004h"),
        "mode controls leaked after state reattach:\n{second_stdout}"
    );

    let second_state = fs::read_to_string(&state_path).expect("read second state");
    assert!(
        second_state.contains("modes 1 0 1 0 0 0 1 0 0\n"),
        "second state did not preserve mode-only update:\n{second_state}"
    );

    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_persists_styled_wide_runs_to_state() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf '\\033[31mred\\033[0m plain\\nwide:\\344\\270\\255'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
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

    let first_stdout = String::from_utf8_lossy(&first_client.stdout);
    assert!(
        first_stdout.contains("red plain") && first_stdout.contains("wide:中"),
        "first client did not render styled/wide rows:\n{first_stdout}"
    );
    assert!(
        !first_stdout.contains("[31m") && !first_stdout.contains("[0m"),
        "SGR controls leaked into first render:\n{first_stdout}"
    );

    let first_state = fs::read_to_string(&state_path).expect("read first state");
    assert_styled_wide_state(&first_state, "first");

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
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let second_stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        second_stdout.contains("red plain") && second_stdout.contains("wide:中"),
        "reattached client did not render cached styled/wide rows:\n{second_stdout}"
    );
    assert!(
        !second_stdout.contains("[31m") && !second_stdout.contains("[0m"),
        "SGR controls leaked after state reattach:\n{second_stdout}"
    );

    let second_state = fs::read_to_string(&state_path).expect("read second state");
    assert_styled_wide_state(&second_state, "second");

    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_persists_replace_rows_metadata_to_state() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf 'ready\\n'; sleep 0.3; printf '\\033]133;A\\033\\\\prompt \\033]133;B\\033\\\\input\\033]133;C\\033\\\\output\\n\\033]8;;https://example.com\\033\\\\linked\\033]8;;\\033\\\\ text'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "4",
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

    let first_stdout = String::from_utf8_lossy(&first_client.stdout);
    assert!(
        first_stdout.contains("ready")
            && first_stdout.contains("prompt inputoutput")
            && first_stdout.contains("linked text"),
        "first client did not render replace-rows output:\n{first_stdout}"
    );
    assert!(
        !first_stdout.contains("]133;") && !first_stdout.contains("]8;;"),
        "metadata controls leaked into first render:\n{first_stdout}"
    );

    let first_state = fs::read_to_string(&state_path).expect("read first state");
    assert_replace_rows_metadata_state(&first_state, "first");

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
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let second_stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        second_stdout.contains("ready")
            && second_stdout.contains("prompt inputoutput")
            && second_stdout.contains("linked text"),
        "reattached client did not render cached replace-rows output:\n{second_stdout}"
    );
    assert!(
        !second_stdout.contains("]133;") && !second_stdout.contains("]8;;"),
        "metadata controls leaked after state reattach:\n{second_stdout}"
    );

    let second_state = fs::read_to_string(&state_path).expect("read second state");
    assert_replace_rows_metadata_state(&second_state, "second");

    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_redraw_prints_terminal_metadata() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf '\\033]2;redraw title\\033\\\\\\033]7;file://localhost/tmp/redraw\\007ready\\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

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
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\x1b[2J\x1b[H"),
        "missing redraw clear/home prefix:\n{stdout:?}"
    );
    assert!(
        stdout.contains(DEFAULT_WORKSPACE_SUMMARY),
        "missing redraw workspace summary:\n{stdout:?}"
    );
    assert!(
        stdout.contains("title=redraw title"),
        "missing redraw title metadata:\n{stdout:?}"
    );
    assert!(
        stdout.contains("working-directory=file://localhost/tmp/redraw"),
        "missing redraw working-directory metadata:\n{stdout:?}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_forwards_focus_when_reporting_is_enabled() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 3 time 20; printf '\\033[?1004hready\n'; bytes=$(dd bs=3 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'focus:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--focus",
            "gained",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("focus:1b5b49"),
        "missing focus gained bytes:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_forwards_application_keypad_enter() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 3 time 20; printf '\\033=ready\n'; bytes=$(dd bs=3 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'keypad:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--key-name",
            "keypad-enter",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("keypad:1b4f4d"),
        "missing application keypad enter bytes:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_forwards_application_cursor_arrow() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 3 time 20; printf '\\033[?1hready\n'; bytes=$(dd bs=3 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'cursor:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--key-name",
            "arrow-up",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("cursor:1b4f41"),
        "missing application cursor arrow bytes:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_forwards_modified_named_key() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 6 time 20; printf 'ready\n'; bytes=$(dd bs=6 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'modified:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--key-name",
            "arrow-up",
            "--key-modifiers",
            "ctrl",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("modified:1b5b313b3541"),
        "missing modified arrow bytes:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_forwards_sgr_mouse_press() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 9 time 20; printf '\\033[?1000h\\033[?1006hready\n'; bytes=$(dd bs=9 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'mouse:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--mouse",
            "press:left:1:1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("mouse:1b5b3c303b313b314d"),
        "missing SGR mouse press bytes:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_forwards_sgr_mouse_modifiers() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 10 time 20; printf '\\033[?1000h\\033[?1006hready\n'; bytes=$(dd bs=10 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'mouse:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--mouse",
            "press:left:1:1",
            "--mouse-modifiers",
            "ctrl",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("mouse:1b5b3c31363b313b314d"),
        "missing modified SGR mouse press bytes:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_forwards_sgr_pixel_mouse_coordinates() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 0 time 20; printf '\\033[?1000h\\033[?1006h\\033[?1016hready\n'; bytes=$(dd bs=32 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'mouse:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--mouse",
            "press:left:1:1",
            "--mouse-pixels",
            "1000:2000",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("mouse:1b5b3c303b313030303b323030304d"),
        "missing SGR-pixels mouse press bytes:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_blocks_motion_in_normal_mouse_mode() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 0 time 5; printf '\\033[?1000h\\033[?1006hready\n'; bytes=$(dd bs=32 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'mouse:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--mouse",
            "motion:left:1:1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        !client.status.success(),
        "nmux unexpectedly succeeded:\n{}",
        String::from_utf8_lossy(&client.stdout)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains(
            "nmux: live server error: input rejected: normal mouse tracking accepts press and release events only"
        ),
        "missing mouse rejection error:\n{stderr}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_cli_reports_mouse_out_of_bounds() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 0 time 5; printf '\\033[?1000h\\033[?1006hready\n'; bytes=$(dd bs=32 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'mouse:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--mouse",
            "press:left:25:1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        !client.status.success(),
        "nmux unexpectedly succeeded:\n{}",
        String::from_utf8_lossy(&client.stdout)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains(
            "nmux: live server error: input rejected: mouse coordinates are outside pane bounds"
        ),
        "missing mouse bounds error:\n{stderr}"
    );
}

#[test]
fn live_cli_redraw_includes_initial_scrollback_range() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--terminal-engine",
            "interim",
            "--command",
            "printf 'ready\nhistory\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

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
    let socket_path = runtime_dir.join("nmux").join("nmux.sock");
    let _ = fs::remove_dir_all(&runtime_dir);
    fs::create_dir_all(&runtime_dir).expect("create runtime dir");

    let mut server = daemon_command()
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .env_remove("NMUX_SOCKET")
        .args([
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .env_remove("NMUX_SOCKET")
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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_dir_all(&runtime_dir);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert_default_workspace_attached(
        &stdout,
        "default-socket live attach did not use XDG_RUNTIME_DIR socket",
    );
}

#[test]
fn live_cli_uses_shared_default_socket_from_env() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .env("NMUX_SOCKET", &socket_path)
        .args([
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .env("NMUX_SOCKET", &socket_path)
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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert_default_workspace_attached(&stdout, "env-socket live attach did not use NMUX_SOCKET");
}

fn test_runtime_dir() -> PathBuf {
    let id = NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(format!("/tmp/nmuxrt{}-{id}", std::process::id()))
}

#[test]
fn live_cli_commits_explicit_resize_with_manual_policy() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
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
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        !stderr.contains("resize request ignored by manual resize policy"),
        "unexpected resize policy warning:\n{stderr}"
    );
    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("session=local tab=tab-1 pane=pane-1 size=80x24 resize=manual"),
        "missing manual resize policy summary:\n{stdout}"
    );
    assert!(
        stdout.contains("session=local tab=tab-1 pane=pane-1 size=100x30 resize=manual"),
        "manual policy should allow explicit user-command resize:\n{stdout}"
    );
}

#[test]
fn live_cli_forwards_modified_named_keys() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--key-name",
            "arrow-up",
            "--key-modifiers",
            "ctrl",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed:\n{}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    let expected_sequence = "^[[1;5A";
    assert!(
        stdout.contains(expected_sequence),
        "missing forwarded Ctrl+ArrowUp bytes:\n{stdout:?}"
    );
}

#[test]
fn live_cli_reports_mouse_tracking_rejections() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--mouse",
            "press:left:1:1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        !client.status.success(),
        "nmux unexpectedly succeeded:\n{}",
        String::from_utf8_lossy(&client.stdout)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("nmux: live server error: input rejected: mouse tracking is disabled"),
        "missing mouse tracking error:\n{stderr}"
    );
}

#[test]
fn live_cli_reports_focus_reporting_rejections() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--focus",
            "gained",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        !client.status.success(),
        "nmux unexpectedly succeeded:\n{}",
        String::from_utf8_lossy(&client.stdout)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("nmux: live server error: input rejected: focus reporting is disabled"),
        "missing focus reporting error:\n{stderr}"
    );
    assert!(
        stderr.contains("code=PermissionDenied")
            && stderr.contains("pane_id=pane-1")
            && stderr.contains("input_seq=1"),
        "missing structured server error attribution:\n{stderr}"
    );
}

#[test]
fn live_json_cli_reports_protocol_error_event() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--json",
            "--iterations",
            "1",
            "--focus",
            "gained",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux --live --json");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        !client.status.success(),
        "nmux unexpectedly succeeded:\n{}",
        String::from_utf8_lossy(&client.stdout)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\"event\":\"error\""),
        "missing JSON error event:\n{stdout}"
    );
    assert!(
        stdout.contains("\"code\":\"permission-denied\"")
            && stdout.contains("\"pane_id\":\"pane-1\"")
            && stdout.contains("\"input_seq\":1"),
        "missing structured JSON error attribution:\n{stdout}"
    );

    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        stderr.contains("nmux: live server error: input rejected: focus reporting is disabled"),
        "missing stderr error:\n{stderr}"
    );
}

#[test]
fn live_cli_redraw_repaints_surface_in_place() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

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
fn live_cli_speculative_echo_repaints_before_server_confirmation() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "stty -echo; printf 'ready'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--redraw",
            "--speculative-echo",
            "--iterations",
            "1",
            "--key",
            "x",
            "--interval-ms",
            "100",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("\x1b[2J\x1b[H"),
        "missing redraw sequence:\n{stdout:?}"
    );
    assert!(
        stdout.contains("ready\x1b[4mx\x1b[24m"),
        "missing underlined speculative echo repaint before server confirmation:\n{stdout:?}"
    );
}

#[test]
fn live_cli_stdin_bytes_speculative_echo_repaints_before_server_confirmation() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "stty -echo; printf 'ready'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let mut client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--redraw",
            "--stdin-bytes",
            "--iterations",
            "1",
            "--interval-ms",
            "100",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run nmux");
    client
        .stdin
        .as_mut()
        .expect("client stdin")
        .write_all(b"x")
        .expect("write stdin byte");

    let client = client.wait_with_output().expect("wait nmux");
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("ready\x1b[4mx\x1b[24m"),
        "missing stdin-byte speculative echo repaint before server confirmation:\n{stdout:?}"
    );
}

#[test]
fn live_clients_can_reattach_to_persisted_workspace_state() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("echo:reattach"),
        "reattached client did not render cached current live surface:\n{stdout}"
    );

    let state = fs::read_to_string(&state_path).unwrap_or_default();
    let _ = fs::remove_file(&state_path);
    assert!(
        !state.lines().any(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            fields.first() == Some(&"scrollback")
                && (fields.get(3) == Some(&"999") || fields.get(4) == Some(&"0"))
        }),
        "out-of-range empty scrollback chunk should not persist a range:\n{state}"
    );
}

#[test]
fn live_clients_can_observe_shared_input_concurrently() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let read_only_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn read-only nmux");

    thread::sleep(Duration::from_millis(150));

    let writer_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "1",
            "--key",
            "shared\n",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run writer nmux");

    let read_only_output = read_only_client
        .wait_with_output()
        .expect("wait for read-only nmux");
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        writer_client.status.success(),
        "writer nmux failed: {}",
        String::from_utf8_lossy(&writer_client.stderr)
    );
    assert!(
        read_only_output.status.success(),
        "read-only nmux failed: {}",
        String::from_utf8_lossy(&read_only_output.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let writer_stdout = String::from_utf8_lossy(&writer_client.stdout);
    assert!(
        writer_stdout.contains("echo:shared"),
        "writer did not render its committed input:\n{writer_stdout}"
    );
    let read_only_stdout = String::from_utf8_lossy(&read_only_output.stdout);
    assert!(
        read_only_stdout.contains("echo:shared"),
        "read-only concurrent client did not receive broadcast input state:\n{read_only_stdout}"
    );
}

#[test]
fn live_json_clients_exchange_presence_identity() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let reader_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--json",
            "--no-input",
            "--no-scrollback",
            "--actor-id",
            "reader",
            "--user-id",
            "reader-user",
            "--display-name",
            "Reader",
            "--iterations",
            "3",
            "--interval-ms",
            "1000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn reader nmux");

    thread::sleep(Duration::from_millis(150));

    let writer_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--json",
            "--no-input",
            "--no-scrollback",
            "--actor-id",
            "writer",
            "--user-id",
            "writer-user",
            "--display-name",
            "Writer",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn writer nmux");

    let writer_client =
        wait_for_command_output(writer_client, "writer nmux", Duration::from_secs(20));
    let reader_output =
        wait_for_command_output(reader_client, "reader nmux", Duration::from_secs(20));
    let server_status =
        wait_for_child_exit(&mut server, Duration::from_secs(20)).unwrap_or_else(|| {
            let _ = server.kill();
            panic!("daemon did not exit after presence clients detached");
        });
    let _ = fs::remove_file(&socket_path);

    assert!(
        writer_client.status.success(),
        "writer nmux failed: {}",
        String::from_utf8_lossy(&writer_client.stderr)
    );
    assert!(
        reader_output.status.success(),
        "reader nmux failed: {}",
        String::from_utf8_lossy(&reader_output.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let reader_stdout = String::from_utf8_lossy(&reader_output.stdout);
    assert!(
        reader_stdout.contains("\"event\":\"presence\"")
            && reader_stdout.contains("\"actor_id\":\"writer\"")
            && reader_stdout.contains("\"user_id\":\"writer-user\"")
            && reader_stdout.contains("\"display_name\":\"Writer\"")
            && reader_stdout.contains("\"mode\":\"read-only\"")
            && reader_stdout.contains("\"focused_pane_id\":\"pane-1\""),
        "reader did not receive writer presence:\n{reader_stdout}"
    );

    let writer_stdout = String::from_utf8_lossy(&writer_client.stdout);
    assert!(
        writer_stdout.contains("\"event\":\"presence\"")
            && writer_stdout.contains("\"actor_id\":\"reader\"")
            && writer_stdout.contains("\"user_id\":\"reader-user\"")
            && writer_stdout.contains("\"display_name\":\"Reader\"")
            && writer_stdout.contains("\"mode\":\"read-only\"")
            && writer_stdout.contains("\"focused_pane_id\":\"pane-1\""),
        "writer did not receive reader presence:\n{writer_stdout}"
    );
}

#[test]
fn live_current_surface_reattach_reports_focus_rejection() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--command",
            "printf 'ready\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
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
            "--focus",
            "gained",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        !second_client.status.success(),
        "second nmux unexpectedly succeeded"
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stderr = String::from_utf8_lossy(&second_client.stderr);
    assert!(
        stderr.contains("nmux: live server error: input rejected: focus reporting is disabled"),
        "missing current-surface focus rejection:\n{stderr}"
    );
}

#[test]
fn live_current_surface_reattach_forwards_paste_input() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'paste:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
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
            "--paste",
            "current-paste\n",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("paste:current-paste"),
        "current-surface reattach did not forward paste input:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_current_surface_reattach_forwards_application_cursor_arrow() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 3 time 20; printf '\\033[?1hready\n'; bytes=$(dd bs=3 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'cursor:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
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
            "--key-name",
            "arrow-up",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("cursor:1b4f41"),
        "current-surface reattach did not forward app-cursor arrow bytes:\n{stdout}"
    );
    assert!(
        !stdout.contains("[?1h"),
        "application-cursor mode control leaked through libghostty-vt output:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_current_surface_reattach_forwards_application_keypad_enter() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 3 time 20; printf '\\033=ready\n'; bytes=$(dd bs=3 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'keypad:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
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
            "--key-name",
            "keypad-enter",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("keypad:1b4f4d"),
        "current-surface reattach did not forward application-keypad Enter bytes:\n{stdout}"
    );
    assert!(
        !stdout.contains("\x1b="),
        "application-keypad mode control leaked through libghostty-vt output:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_current_surface_reattach_wraps_bracketed_paste() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 16 time 20; printf '\\033[?2004hready\n'; bytes=$(dd bs=16 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'paste:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
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
            "--paste",
            "clip",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("paste:1b5b3230307e636c69701b5b3230317e"),
        "current-surface reattach did not use daemon-owned bracketed paste mode:\n{stdout}"
    );
    assert!(
        !stdout.contains("[?2004h"),
        "bracketed-paste mode control leaked through libghostty-vt output:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_current_surface_reattach_forwards_sgr_mouse_press() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 9 time 20; printf '\\033[?1000h\\033[?1006hready\n'; bytes=$(dd bs=9 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'mouse:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
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
            "--mouse",
            "press:left:1:1",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("mouse:1b5b3c303b313b314d"),
        "current-surface reattach did not forward SGR mouse press bytes:\n{stdout}"
    );
    assert!(
        !stdout.contains("[?1000h") && !stdout.contains("[?1006h"),
        "mouse mode controls leaked through libghostty-vt output:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_current_surface_reattach_forwards_sgr_pixel_mouse_press() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "stty -icanon -echo min 1 time 20; printf '\\033[?1000h\\033[?1006h\\033[?1016hready\n'; bytes=$(dd bs=32 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'mouse:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
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
            "--mouse",
            "press:left:1:1",
            "--mouse-pixels",
            "1000:2000",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("mouse:1b5b3c303b313030303b323030304d"),
        "current-surface reattach did not forward SGR-pixels mouse press bytes:\n{stdout}"
    );
    assert!(
        !stdout.contains("[?1000h") && !stdout.contains("[?1006h") && !stdout.contains("[?1016h"),
        "mouse mode controls leaked through libghostty-vt output:\n{stdout}"
    );
}

#[test]
fn live_state_file_is_scoped_to_socket_identity() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut first_server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'first daemon\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn first daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run first nmux");
    let first_status = first_server.wait().expect("wait for first daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        first_client.status.success(),
        "first nmux failed: {}",
        String::from_utf8_lossy(&first_client.stderr)
    );
    assert!(
        first_status.success(),
        "first daemon failed: {first_status}"
    );

    let mut second_server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'second daemon\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn second daemon");

    wait_for_socket(&socket_path);

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
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");
    let second_status = second_server.wait().expect("wait for second daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(
        second_status.success(),
        "second daemon failed: {second_status}"
    );

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("second daemon"),
        "new daemon surface was not rendered:\n{stdout}"
    );
    assert!(
        !stdout.contains("first daemon"),
        "stale cached surface leaked across socket recreation:\n{stdout}"
    );
}

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_clients_can_reattach_to_persisted_workspace_state() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf '\\033]2;cached title\\033\\\\\\033]7;file://localhost/tmp/cached\\007ready\\n'; while IFS= read -r line; do printf '\\033[32mecho:%s\\033[0m\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("echo:reattach"),
        "reattached libghostty-vt client did not render cached current live surface:\n{stdout}"
    );
    assert!(
        stdout.contains("title=cached title"),
        "reattached libghostty-vt client did not render cached title metadata:\n{stdout}"
    );
    assert!(
        stdout.contains("working-directory=file://localhost/tmp/cached"),
        "reattached libghostty-vt client did not render cached working-directory metadata:\n{stdout}"
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

#[cfg(feature = "libghostty-vt")]
#[test]
fn live_libghostty_vt_reattach_recovers_style_table_full_refresh() {
    let socket_path = test_socket_path();
    let state_path = socket_path.with_extension("state");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "2",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf '\\033[31mstyled:%s\\033[0m\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let first_client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--state",
            state_path.to_str().expect("state path"),
            "--live",
            "--no-input",
            "--iterations",
            "1",
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
            "--iterations",
            "1",
            "--key",
            "full-refresh\n",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run second nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&state_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&second_client.stdout);
    assert!(
        stdout.contains("styled:full-refresh"),
        "reattached libghostty-vt client did not render styled update:\n{stdout}"
    );
    assert!(
        !stdout.contains("[31m") && !stdout.contains("[0m"),
        "ANSI control sequences leaked after full-refresh reattach:\n{stdout}"
    );
}

#[test]
fn live_forever_can_serve_sequential_reattach_clients() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-forever",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        second_client.status.success(),
        "second nmux failed: {}",
        String::from_utf8_lossy(&second_client.stderr)
    );
    assert!(
        !server_status.success(),
        "daemon should have been terminated after test clients"
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

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(stdout.contains("echo:ping"), "missing ping echo:\n{stdout}");
    assert!(stdout.contains("echo:pong"), "missing pong echo:\n{stdout}");
}

#[test]
fn live_cli_can_drive_input_chunks_from_stdin_bytes() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");
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
fn live_cli_stdin_bytes_forwards_bracketed_paste_as_paste_input() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'paste:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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

    let mut stdin = client.stdin.take().expect("client stdin");
    stdin
        .write_all(b"\x1b[200~clip\n\x1b[201~")
        .expect("write bracketed paste");
    stdin.flush().expect("flush bracketed paste");
    drop(stdin);

    let client = client.wait_with_output().expect("wait for nmux");
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("paste:clip"),
        "missing decoded paste output:\n{stdout}"
    );
    assert!(
        !stdout.contains("[200~clip"),
        "bracketed paste control bytes leaked to pane:\n{stdout}"
    );
}

#[test]
fn live_stdin_bytes_keeps_polling_before_input_arrives() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; sleep 0.05; printf 'tick-before-input\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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
    let server_status = server.wait().expect("wait for daemon");
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
    assert!(server_status.success(), "daemon failed: {server_status}");
    assert!(
        lines.iter().any(|line| line.contains("tick-before-input")),
        "missing delayed output:\n{}",
        lines.join("\n")
    );
}

#[test]
fn live_stdin_lines_keep_polling_before_input_arrives() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; sleep 0.05; printf 'tick-before-input\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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
    let server_status = server.wait().expect("wait for daemon");
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
    assert!(server_status.success(), "daemon failed: {server_status}");
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

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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
    let server_status = server.wait().expect("wait for daemon");
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
    assert!(server_status.success(), "daemon failed: {server_status}");
    assert!(
        lines.iter().any(|line| line.contains("echo:ping")),
        "missing ping echo:\n{}",
        lines.join("\n")
    );
}

#[test]
fn live_stdin_bytes_can_pass_ctrl_right_bracket_through() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "stty -icanon -echo min 6 time 20; printf 'ready\n'; bytes=$(dd bs=6 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'); printf 'bytes:%s\n' \"$bytes\"",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let mut client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--stdin-bytes",
            "--detach-key",
            "none",
            "--iterations",
            "2",
            "--interval-ms",
            "100",
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
        .write_all(b"ping\n\x1d")
        .expect("write input");
    drop(client.stdin.take());

    let client = client.wait_with_output().expect("wait for nmux");
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        !stderr.contains("detached by local Ctrl-]"),
        "unexpected local detach status:\n{stderr}"
    );
    assert!(server_status.success(), "daemon failed: {server_status}");
    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("bytes:70696e670a1d"),
        "missing forwarded Ctrl-] byte:\n{stdout}"
    );
}

#[test]
fn live_stdin_bytes_ctrl_d_exits_pane_without_later_input_error() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "stty -icanon -echo min 1 time 0; printf 'ready\n'; dd bs=1 count=1 >/dev/null 2>/dev/null",
        ])
        .spawn()
        .expect("spawn daemon");

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

    let lines = read_until_line(&lines_rx, "ready");
    let mut stdin = client.stdin.take().expect("client stdin");
    stdin.write_all(b"\x04").expect("write ctrl-d");
    stdin.flush().expect("flush ctrl-d");
    thread::sleep(Duration::from_millis(200));
    if let Err(error) = stdin.write_all(b"\n") {
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }
    drop(stdin);

    let client = client.wait_with_output().expect("wait for nmux");
    stdout_reader.join().expect("stdout reader");
    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    let stderr = String::from_utf8_lossy(&client.stderr);
    assert!(
        !stderr.contains("input forwarding failed"),
        "pane exit should not report input forwarding failure:\n{stderr}"
    );
    assert!(server_status.success(), "daemon failed: {server_status}");
    assert!(
        lines.iter().any(|line| line.contains("ready")),
        "missing ready output:\n{}",
        lines.join("\n")
    );
}

#[test]
fn live_read_only_cli_observes_output_without_input() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; printf 'tick-one\n'; printf 'tick-two\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--iterations",
            "3",
            "--interval-ms",
            "1000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux");

    let client = wait_for_command_output(client, "read-only live client", Duration::from_secs(10));
    let server_status =
        wait_for_child_exit(&mut server, Duration::from_secs(10)).unwrap_or_else(|| {
            let _ = server.kill();
            panic!("daemon did not exit after read-only live client");
        });
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

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
fn live_cli_renders_split_pty_writes_as_one_logical_line() {
    let socket_path = test_socket_path();
    let ready_path = socket_path.with_extension("ready");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&ready_path);
    let command = format!(
        "printf a; sleep 0.05; printf b; sleep 0.05; printf c; printf '\\n'; touch {}; sleep 1",
        shell_quote(ready_path.to_str().expect("ready path"))
    );

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            &command,
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);
    wait_for_path(&ready_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-scrollback",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux");

    let client =
        wait_for_command_output(client, "split-write live client", Duration::from_secs(10));
    let server_status =
        wait_for_child_exit(&mut server, Duration::from_secs(10)).unwrap_or_else(|| {
            let _ = server.kill();
            panic!("daemon did not exit after split-write live client");
        });
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&ready_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert_split_pty_writes_render_as_one_line(&stdout);
}

#[test]
fn live_redraw_cli_renders_split_pty_writes_as_one_logical_line() {
    let socket_path = test_socket_path();
    let ready_path = socket_path.with_extension("ready");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&ready_path);
    let command = format!(
        "printf a; sleep 0.05; printf b; sleep 0.05; printf c; printf '\\n'; touch {}; sleep 1",
        shell_quote(ready_path.to_str().expect("ready path"))
    );

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            &command,
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);
    wait_for_path(&ready_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--redraw",
            "--iterations",
            "1",
            "--interval-ms",
            "1000",
        ])
        .output()
        .expect("run nmux");

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&ready_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert_split_pty_writes_render_as_one_line(&stdout);
}

#[test]
fn live_redraw_tty_uses_alternate_screen_and_logical_lines() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-clients",
            "1",
            "--command",
            "printf 'ready\n'; sleep 0.05; printf a; sleep 0.01; printf b; sleep 0.01; printf c; printf '\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let output = spawn_nmux_client_in_pty(&[
        "--socket",
        socket_path.to_str().expect("socket path"),
        "--live",
        "--redraw",
        "--iterations",
        "30",
        "--interval-ms",
        "50",
    ])
    .wait();

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(output.success, "nmux failed:\n{}", output.output);
    assert!(server_status.success(), "daemon failed: {server_status}");
    assert!(
        output.output.contains("\x1b[?1049h\x1b[?25l"),
        "missing alternate-screen entry:\n{}",
        output.output
    );
    assert!(
        output.output.contains("\x1b[?25h\x1b[?1049l"),
        "missing alternate-screen exit:\n{}",
        output.output
    );
    assert!(
        output.output.contains("\x1b[2J\x1b[1;1H"),
        "missing initial redraw clear/home:\n{}",
        output.output
    );
    assert!(
        output.output.contains("\x1b[7m") && output.output.contains(" rows "),
        "missing redraw stats overlay:\n{}",
        output.output
    );
    let ready_output = output
        .output
        .split_once("ready")
        .map(|(_, trailing)| trailing)
        .unwrap_or_else(|| panic!("missing initial split-write line:\n{}", output.output));
    // Ratatui may paint later cells as cursor-addressed diffs, so the raw PTY
    // stream is not required to contain the complete logical line contiguously.
    for fragment in ["a", "b", "c"] {
        assert!(
            ready_output.contains(fragment),
            "missing split-write fragment {fragment:?} after initial line:\n{}",
            output.output
        );
    }
    assert!(
        !output.output.contains("\na\nb\nc\n") && !output.output.contains("\r\na\r\nb\r\nc\r\n"),
        "split writes rendered as separate rows:\n{}",
        output.output
    );
}

#[test]
fn live_read_only_cli_without_iterations_runs_until_server_closes() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; printf 'tick-one\n'; printf 'tick-two\n'; sleep 1",
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-scrollback",
            "--interval-ms",
            "1000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux");

    let client = wait_for_command_output(
        client,
        "read-only live client without iterations",
        Duration::from_secs(10),
    );
    let server_status =
        wait_for_child_exit(&mut server, Duration::from_secs(10)).unwrap_or_else(|| {
            let _ = server.kill();
            panic!("daemon did not exit after read-only live client without iterations");
        });
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
    assert!(server_status.success(), "daemon failed: {server_status}");

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

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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
    let server_status = server.wait().expect("wait for daemon");
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
    assert!(server_status.success(), "daemon failed: {server_status}");

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

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done",
        ])
        .spawn()
        .expect("spawn daemon");

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

    let server_status = server.wait().expect("wait for daemon");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let state = fs::read_to_string(&state_path).expect("read state");
    let _ = fs::remove_file(&state_path);
    let pane_surface_version = persisted_pane_surface_version(&state, "70616e652d31")
        .unwrap_or_else(|| panic!("expected pane-1 persisted surface in state:\n{state}"));
    assert!(
        pane_surface_version >= 4,
        "expected pane-1 surface version at least 4 in state:\n{state}"
    );
    assert!(
        state.contains("6563686f3a70657273697374"),
        "expected rendered live output in state:\n{state}"
    );
    assert!(
        state.contains("scrollback 70616e652d31 "),
        "expected pane-1 scrollback metadata in state:\n{state}"
    );
}

fn persisted_pane_surface_version(state: &str, pane_id_hex: &str) -> Option<u64> {
    state.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        if fields.next()? != "surface" || fields.next()? != pane_id_hex {
            return None;
        }
        fields.next()?.parse().ok()
    })
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

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
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

fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("path did not appear: {}", path.display());
}

fn wait_for_child_exit(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll child") {
            return Some(status);
        }
        thread::sleep(Duration::from_millis(20));
    }
    None
}

fn wait_for_command_output(mut child: Child, label: &str, timeout: Duration) -> Output {
    if wait_for_child_exit(&mut child, timeout).is_none() {
        let _ = child.kill();
        panic!("{label} did not exit within {timeout:?}");
    }
    child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("wait for {label}: {error}"))
}

#[cfg(feature = "libghostty-vt")]
fn assert_styled_wide_state(state: &str, label: &str) {
    let style_count = state
        .lines()
        .filter(|line| line.starts_with("style "))
        .count();
    assert!(
        style_count > 1,
        "{label} state missing non-default style table entry:\n{state}"
    );

    let styled_row = state_row_index(state, "72656420706c61696e")
        .unwrap_or_else(|| panic!("{label} state missing styled row:\n{state}"));
    assert!(
        state_contains_run_with_style_and_width(
            state,
            styled_row,
            "726564",
            "010101",
            StyleExpectation::Styled,
        ) && state_contains_run_with_style_and_width(
            state,
            styled_row,
            "20706c61696e",
            "010101010101",
            StyleExpectation::Default,
        ),
        "{label} state missing styled/default run split:\n{state}"
    );

    let wide_row = state_row_index(state, "776964653ae4b8ad")
        .unwrap_or_else(|| panic!("{label} state missing wide row:\n{state}"));
    assert!(
        state_contains_run_with_style_and_width(
            state,
            wide_row,
            "776964653ae4b8ad",
            "010101010102",
            StyleExpectation::Default,
        ),
        "{label} state missing wide-cell width bytes:\n{state}"
    );
}

#[cfg(feature = "libghostty-vt")]
fn assert_replace_rows_metadata_state(state: &str, label: &str) {
    let prompt_row = state_row_index(state, "70726f6d707420696e7075746f7574707574")
        .unwrap_or_else(|| panic!("{label} state missing OSC 133 prompt row:\n{state}"));
    let prompt_rowmeta = format!("rowmeta {prompt_row} 1 1 0 ");
    assert!(
        state.contains(&prompt_rowmeta),
        "{label} state missing prompt row metadata:\n{state}"
    );
    assert!(
        state_contains_run(state, prompt_row, "70726f6d707420", 0, 0, 0, 2)
            && state_contains_run(state, prompt_row, "696e707574", 0, 0, 0, 1)
            && state_contains_run(state, prompt_row, "6f7574707574", 0, 0, 0, 0),
        "{label} state missing OSC 133 semantic content runs:\n{state}"
    );

    let link_row = state_row_index(state, "6c696e6b65642074657874")
        .unwrap_or_else(|| panic!("{label} state missing OSC 8 hyperlink row:\n{state}"));
    assert!(
        state_contains_run(state, link_row, "6c696e6b6564", 0, 1, 0, 0)
            && state_contains_run(state, link_row, "2074657874", 0, 0, 0, 0),
        "{label} state missing OSC 8 hyperlink run flags:\n{state}"
    );
}

#[cfg(feature = "libghostty-vt")]
fn state_row_index<'a>(state: &'a str, text_hex: &str) -> Option<&'a str> {
    state.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some("row"), Some(row), Some(text), None) if text == text_hex => Some(row),
            _ => None,
        }
    })
}

#[cfg(feature = "libghostty-vt")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StyleExpectation {
    Styled,
    Default,
}

#[cfg(feature = "libghostty-vt")]
fn state_contains_run_with_style_and_width(
    state: &str,
    row: &str,
    text_hex: &str,
    width_hex: &str,
    style: StyleExpectation,
) -> bool {
    state.lines().any(|line| {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 8
            || parts[0] != "run"
            || parts[1] != row
            || parts[2] != text_hex
            || parts[3] != width_hex
        {
            return false;
        }
        let Ok(style_id) = parts[4].parse::<u32>() else {
            return false;
        };
        match style {
            StyleExpectation::Styled => style_id != 0,
            StyleExpectation::Default => style_id == 0,
        }
    })
}

#[cfg(feature = "libghostty-vt")]
fn state_contains_run(
    state: &str,
    row: &str,
    text_hex: &str,
    style_id: u32,
    flags: u32,
    hyperlink_id: u32,
    semantic_content: i8,
) -> bool {
    state.lines().any(|line| {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        parts.len() == 8
            && parts[0] == "run"
            && parts[1] == row
            && parts[2] == text_hex
            && parts[4] == style_id.to_string()
            && parts[5] == flags.to_string()
            && parts[6] == hyperlink_id.to_string()
            && parts[7] == semantic_content.to_string()
    })
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

/// Regression test: bounded post-input coalescing must hold the first changed
/// frame long enough for delayed PTY output to arrive. The scheduled output
/// avoids depending on platform-specific terminal input line discipline while
/// still reproducing the old one-cycle attach behavior: the `typed:` frame
/// arrived first, consumed the only iteration, and the delayed `echo:` frame
/// was lost.
#[test]
fn live_cycles_coalesces_delayed_echo_after_input() {
    let socket_path = test_socket_path();
    let ready_path = socket_path.with_extension("ready");
    let trigger_path = socket_path.with_extension("trigger");
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&ready_path);
    let _ = fs::remove_file(&trigger_path);
    let command = format!(
        "touch {}; while [ ! -f {} ]; do sleep 0.01; done; printf 'echo:coalesce!\\n'; sleep 1",
        shell_quote(ready_path.to_str().expect("ready path")),
        shell_quote(trigger_path.to_str().expect("trigger path")),
    );

    let mut server = daemon_command()
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live-cycles",
            "1",
            "--command",
            &command,
        ])
        .spawn()
        .expect("spawn daemon");

    wait_for_socket(&socket_path);
    wait_for_path(&ready_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--live",
            "--no-scrollback",
            "--iterations",
            "1",
            "--key",
            "coalesce!",
            "--interval-ms",
            "2000",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn nmux --live");
    thread::sleep(Duration::from_millis(50));
    fs::write(&trigger_path, b"trigger").expect("write trigger path");

    let client = wait_for_command_output(client, "coalescing live client", Duration::from_secs(10));
    let server_status =
        wait_for_child_exit(&mut server, Duration::from_secs(5)).unwrap_or_else(|| {
            let _ = server.kill();
            panic!("daemon did not exit after bounded live cycle");
        });
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&ready_path);
    let _ = fs::remove_file(&trigger_path);

    assert!(
        client.status.success(),
        "nmux failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "daemon failed: {server_status}");

    let stdout = String::from_utf8_lossy(&client.stdout);
    assert!(
        stdout.contains("echo:coalesce!"),
        "daemon exited before delayed echo arrived — post-input coalescing is broken:\n{stdout}"
    );
}
