#![cfg(feature = "libghostty-vt")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_PATH_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn renderer_equivalence_smoke_captures_structured_nmux_state() {
    let socket_path = test_socket_path();
    let _ = fs::remove_file(&socket_path);
    let command = concat!(
        "printf '\\033]0;renderer fixture\\007'; ",
        "printf '\\033]7;file://localhost/tmp/nmux-renderer\\033\\\\'; ",
        "printf '\\033[31mred\\033[0m plain'; printf '\\n'; ",
        "printf 'wide:中'; printf '\\n'; ",
        "printf '\\033]8;;https://example.invalid\\033\\\\link\\033]8;;\\033\\\\'; printf '\\n'; ",
        "printf 'main-scroll'; printf '\\n'; ",
        "printf '\\033[?1049hALT-ONLY\\033[?1049l'; ",
        "cat >/dev/null"
    );

    let mut server = Command::new(env!("CARGO_BIN_EXE_nmuxd"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--one-shot",
            "--terminal-engine",
            "libghostty-vt",
            "--command",
            command,
        ])
        .spawn()
        .expect("spawn nmuxd");

    wait_for_socket(&socket_path);

    let client = Command::new(env!("CARGO_BIN_EXE_nmux"))
        .args([
            "--socket",
            socket_path.to_str().expect("socket path"),
            "--connect-timeout-ms",
            "5000",
            "--json",
            "--scrollback-start",
            "1",
            "--scrollback-count",
            "20",
        ])
        .output()
        .expect("run nmux --json");

    let server_status = server.wait().expect("wait for nmuxd");
    let _ = fs::remove_file(&socket_path);

    assert!(
        client.status.success(),
        "nmux --json failed: {}",
        String::from_utf8_lossy(&client.stderr)
    );
    assert!(server_status.success(), "nmuxd failed: {server_status}");

    let stdout = String::from_utf8(client.stdout).expect("json stdout is utf-8");
    write_artifact_if_requested(&stdout);

    assert_contains(&stdout, "\"title\":\"renderer fixture\"");
    assert_contains(
        &stdout,
        "\"working_directory\":\"file://localhost/tmp/nmux-renderer\"",
    );
    assert_contains(&stdout, "\"surface_kind\":\"main\"");
    assert_contains(&stdout, "\"text\":\"red plain\"");
    assert_contains(
        &stdout,
        "\"text\":\"red\",\"cell_widths\":[1,1,1],\"style_id\":1",
    );
    assert_contains(
        &stdout,
        "\"text\":\" plain\",\"cell_widths\":[1,1,1,1,1,1],\"style_id\":0",
    );
    assert_contains(&stdout, "\"text\":\"wide:中\"");
    assert_contains(
        &stdout,
        "\"text\":\"wide:中\",\"cell_widths\":[1,1,1,1,1,2]",
    );
    assert_contains(&stdout, "\"text\":\"link\"");
    assert_contains(
        &stdout,
        "\"text\":\"link\",\"cell_widths\":[1,1,1,1],\"style_id\":0,\"flags\":1",
    );
    assert_contains(&stdout, "\"text\":\"main-scroll\"");
    assert_contains(
        &stdout,
        "\"surface_text\":\"red plain\\nwide:中\\nlink\\nmain-scroll\"",
    );
    assert_absent(&stdout, "\u{1b}[31m");
    assert_absent(&stdout, "ALT-ONLY");
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

fn write_artifact_if_requested(json: &str) {
    let Some(dir) = std::env::var_os("NMUX_RENDERER_EQUIVALENCE_ARTIFACT_DIR") else {
        return;
    };
    let dir = workspace_path(PathBuf::from(dir));
    fs::create_dir_all(&dir).expect("create renderer-equivalence artifact dir");
    fs::write(dir.join("libghostty-vt-smoke.json"), json)
        .expect("write renderer-equivalence artifact");
}

fn workspace_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn assert_contains(haystack: &str, needle: &str) {
    assert!(haystack.contains(needle), "missing {needle:?}:\n{haystack}");
}

fn assert_absent(haystack: &str, needle: &str) {
    assert!(
        !haystack.contains(needle),
        "unexpected {needle:?}:\n{haystack}"
    );
}
