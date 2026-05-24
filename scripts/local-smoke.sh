#!/usr/bin/env bash
set -euo pipefail
echo "running local nmux daemon/client smoke"
tmp_dir="$(mktemp -d "/tmp/nmux-local-smoke.XXXXXX")"
repo_dir="$(pwd)"
cargo_target_dir="${CARGO_TARGET_DIR:-target}"
case "$cargo_target_dir" in
    /*) nmux_bin="$cargo_target_dir/debug/nmux" ;;
    *) nmux_bin="$repo_dir/$cargo_target_dir/debug/nmux" ;;
esac
socket="$tmp_dir/nmux.sock"
state="$tmp_dir/state.json"
daemon_out="$tmp_dir/daemon.out"
daemon_err="$tmp_dir/daemon.err"
client1_out="$tmp_dir/client1.out"
client1_err="$tmp_dir/client1.err"
client2_out="$tmp_dir/client2.out"
client2_err="$tmp_dir/client2.err"
client3_out="$tmp_dir/client3.out"
client3_err="$tmp_dir/client3.err"
client4_out="$tmp_dir/client4.out"
client4_err="$tmp_dir/client4.err"
client5_out="$tmp_dir/client5.out"
client5_err="$tmp_dir/client5.err"
client6_out="$tmp_dir/client6.out"
client6_err="$tmp_dir/client6.err"
info_nmux_version_json="$tmp_dir/info-nmux-version.json"
info_daemon_version_json="$tmp_dir/info-daemon-version.json"
info_nmux_socket_json="$tmp_dir/info-nmux-socket.json"
info_daemon_socket_json="$tmp_dir/info-daemon-socket.json"
cleanup() {
    status="$?"
    if [ -n "${daemon_pid:-}" ] && kill -0 "$daemon_pid" >/dev/null 2>&1; then
        kill "$daemon_pid" >/dev/null 2>&1 || true
        wait "$daemon_pid" >/dev/null 2>&1 || true
    fi
    rm -rf "$tmp_dir"
    exit "$status"
}
trap cleanup EXIT INT TERM
cargo run --quiet --bin nmux -- --version-json >"$info_nmux_version_json"
cargo run --quiet --bin nmux -- daemon --version-json >"$info_daemon_version_json"
cargo run --quiet --bin nmux -- --socket "$socket" --print-socket-json >"$info_nmux_socket_json"
cargo run --quiet --bin nmux -- daemon --socket "$socket" --print-socket-json >"$info_daemon_socket_json"
command_text="printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
cargo run --quiet --bin nmux -- daemon --socket "$socket" --ready-json --live-clients 2 --command "$command_text" >"$daemon_out" 2>"$daemon_err" &
daemon_pid="$!"
if ! printf 'ping\n' | cargo run --quiet --bin nmux -- --socket "$socket" --connect-timeout-ms 5000 --state "$state" --live --iterations 1 --stdin --scrollback-start 1 --scrollback-count 8 >"$client1_out" 2>"$client1_err"; then
    cat "$daemon_err" "$client1_err" >&2
    exit 1
fi
if ! cargo run --quiet --bin nmux -- --socket "$socket" --connect-timeout-ms 5000 --state "$state" --live --no-input --iterations 1 --scrollback-start 1 --scrollback-count 8 >"$client2_out" 2>"$client2_err"; then
    cat "$daemon_err" "$client2_err" >&2
    exit 1
fi
if ! wait "$daemon_pid"; then
    daemon_pid=""
    cat "$daemon_err" >&2
    exit 1
fi
daemon_pid=""
if ! grep -Fq '"event":"ready"' "$daemon_out" || ! grep -Fq "\"NMUX_SOCKET\":\"$socket\"" "$daemon_out" || ! grep -Fq '"mode":"live-clients"' "$daemon_out"; then
    echo "missing local smoke ready-json output" >&2
    cat "$daemon_out" "$daemon_err" >&2
    exit 1
fi
: >"$daemon_out"
: >"$daemon_err"
command_text="printf 'fresh daemon\n'; sleep 1"
cargo run --quiet --bin nmux -- daemon --socket "$socket" --live-clients 1 --command "$command_text" >"$daemon_out" 2>"$daemon_err" &
daemon_pid="$!"
if ! cargo run --quiet --bin nmux -- --socket "$socket" --connect-timeout-ms 5000 --state "$state" --live --no-input --iterations 1 --scrollback-start 1 --scrollback-count 8 >"$client3_out" 2>"$client3_err"; then
    cat "$daemon_err" "$client3_err" >&2
    exit 1
fi
if ! wait "$daemon_pid"; then
    daemon_pid=""
    cat "$daemon_err" >&2
    exit 1
fi
daemon_pid=""
: >"$daemon_out"
: >"$daemon_err"
command_text="\"$nmux_bin\" --print-context; cat >/dev/null"
cargo run --quiet --bin nmux -- daemon --socket "$socket" --one-shot --cols 200 --rows 24 --command "$command_text" >"$daemon_out" 2>"$daemon_err" &
daemon_pid="$!"
if ! cargo run --quiet --bin nmux -- --socket "$socket" --connect-timeout-ms 5000 --scrollback-start 1 --scrollback-count 24 >"$client4_out" 2>"$client4_err"; then
    cat "$daemon_err" "$client4_err" >&2
    exit 1
fi
if ! wait "$daemon_pid"; then
    daemon_pid=""
    cat "$daemon_err" >&2
    exit 1
fi
daemon_pid=""
: >"$daemon_out"
: >"$daemon_err"
command_text="\"$nmux_bin\" --print-context-json; cat >/dev/null"
cargo run --quiet --bin nmux -- daemon --socket "$socket" --one-shot --cols 200 --rows 24 --command "$command_text" >"$daemon_out" 2>"$daemon_err" &
daemon_pid="$!"
if ! cargo run --quiet --bin nmux -- --socket "$socket" --connect-timeout-ms 5000 --scrollback-start 1 --scrollback-count 24 >"$client5_out" 2>"$client5_err"; then
    cat "$daemon_err" "$client5_err" >&2
    exit 1
fi
if ! wait "$daemon_pid"; then
    daemon_pid=""
    cat "$daemon_err" >&2
    exit 1
fi
daemon_pid=""
if ! cargo run --quiet --bin nmux -- --start --json --startup-timeout-ms 5000 --command "printf 'managed-json-ready\n'; cat >/dev/null" >"$client6_out" 2>"$client6_err"; then
    cat "$client6_err" >&2
    exit 1
fi
require_output() {
    file="$1"
    text="$2"
    description="$3"
    if ! grep -Fq "$text" "$file"; then
        echo "missing local smoke output: $description" >&2
        cat "$daemon_err" "$client1_err" "$client2_err" "$client3_err" "$client4_err" "$client5_err" "$client6_err" >&2
        echo "--- $file ---" >&2
        cat "$file" >&2
        exit 1
    fi
}
reject_output() {
    file="$1"
    text="$2"
    description="$3"
    if grep -Fq "$text" "$file"; then
        echo "unexpected local smoke output: $description" >&2
        cat "$daemon_err" "$client1_err" "$client2_err" "$client3_err" "$client4_err" "$client5_err" "$client6_err" >&2
        echo "--- $file ---" >&2
        cat "$file" >&2
        exit 1
    fi
}
require_output "$client1_out" 'ready' 'initial daemon output'
require_output "$client1_out" 'echo:ping' 'read-write live input response'
require_output "$client2_out" 'echo:ping' 'sequential read-only reattach sees prior output'
require_output "$client3_out" 'fresh daemon' 'recreated socket path forces fresh daemon surface'
reject_output "$client3_out" 'echo:ping' 'stale cached surface after socket recreation'
require_output "$client4_out" 'NMUX=1' 'nested print-context nmux flag'
require_output "$client4_out" 'NMUX_SESSION_ID=local' 'nested print-context session id'
require_output "$client4_out" 'NMUX_PANE_ID=pane-1' 'nested print-context pane id'
require_output "$client4_out" "NMUX_SOCKET=$socket" 'nested print-context socket path'
require_output "$client4_out" 'NMUX_ORIGIN=local' 'nested print-context origin'
require_output "$client5_out" '"NMUX":"1"' 'nested print-context-json nmux flag'
require_output "$client5_out" '"NMUX_SESSION_ID":"local"' 'nested print-context-json session id'
require_output "$client5_out" '"NMUX_PANE_ID":"pane-1"' 'nested print-context-json pane id'
require_output "$client5_out" "\"NMUX_SOCKET\":\"$socket\"" 'nested print-context-json socket path'
require_output "$client5_out" '"NMUX_ORIGIN":"local"' 'nested print-context-json origin'
require_output "$client6_out" '{"workspace":{' 'managed start JSON workspace'
require_output "$client6_out" 'managed-json-ready' 'managed start JSON command output'
require_output "$info_nmux_version_json" '"binary":"nmux"' 'nmux version-json binary'
require_output "$info_nmux_version_json" '"version":"' 'nmux version-json version'
require_output "$info_nmux_version_json" '"channel":"' 'nmux version-json channel'
require_output "$info_nmux_version_json" '"commit":"' 'nmux version-json commit'
require_output "$info_nmux_version_json" '"build_date":"' 'nmux version-json build date'
require_output "$info_daemon_version_json" '"binary":"nmux"' 'daemon version-json binary'
require_output "$info_daemon_version_json" '"version":"' 'daemon version-json version'
require_output "$info_daemon_version_json" '"channel":"' 'daemon version-json channel'
require_output "$info_daemon_version_json" '"commit":"' 'daemon version-json commit'
require_output "$info_daemon_version_json" '"build_date":"' 'daemon version-json build date'
require_output "$info_nmux_socket_json" "\"NMUX_SOCKET\":\"$socket\"" 'nmux print-socket-json socket path'
require_output "$info_nmux_socket_json" '"source":"--socket"' 'nmux print-socket-json source'
require_output "$info_daemon_socket_json" "\"NMUX_SOCKET\":\"$socket\"" 'daemon print-socket-json socket path'
require_output "$info_daemon_socket_json" '"source":"--socket"' 'daemon print-socket-json source'
test -s "$state" || { echo "missing local smoke state file: $state" >&2; exit 1; }
printf 'local_smoke_reattach=passed\n'
printf 'local_smoke_socket_recreation=passed\n'
printf 'local_smoke_print_context=passed\n'
printf 'local_smoke_json_info=passed\n'
printf 'local_smoke_ready_json=passed\n'
printf 'local_smoke_managed_start=passed\n'
printf 'local_smoke=passed\n'
