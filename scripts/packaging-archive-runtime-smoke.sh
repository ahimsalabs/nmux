#!/usr/bin/env bash
set -euo pipefail
echo "running packaged opt-in libghostty-vt archive runtime smoke"
archive=target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz
work_dir="$(mktemp -d "/tmp/nmuxpkg.XXXXXX")"
install_root="$work_dir/install"
mkdir -p "$install_root"
tar -C "$install_root" -xzf "$archive"
pkg_dir="$install_root/package"
socket="$work_dir/nmux.sock"
client_out="$work_dir/client.out"
client_err="$work_dir/client.err"
daemon_out="$work_dir/daemon.out"
daemon_err="$work_dir/daemon.err"
daemon_pid=""
trap 'status=$?; if [ -n "${daemon_pid:-}" ] && kill -0 "$daemon_pid" >/dev/null 2>&1; then kill "$daemon_pid" >/dev/null 2>&1 || true; wait "$daemon_pid" >/dev/null 2>&1 || true; fi; rm -rf "$work_dir"; exit "$status"' EXIT INT TERM
printf 'packaged_runtime_smoke_install_root=%s\n' "$install_root"
printf 'packaged_runtime_smoke_library_env=unset\n'
env -u DYLD_LIBRARY_PATH -u LD_LIBRARY_PATH "$pkg_dir/bin/nmux" daemon --socket "$socket" --one-shot --terminal-engine libghostty-vt --command "printf 'packaged-runtime-smoke\n'; cat >/dev/null" >"$daemon_out" 2>"$daemon_err" &
daemon_pid="$!"
if ! env -u DYLD_LIBRARY_PATH -u LD_LIBRARY_PATH "$pkg_dir/bin/nmux" --socket "$socket" --connect-timeout-ms 5000 --no-input --scrollback-start 1 --scrollback-count 5 >"$client_out" 2>"$client_err"; then
    echo "packaged runtime smoke client failed" >&2
    cat "$client_err" >&2
    exit 1
fi
if ! wait "$daemon_pid"; then
    daemon_pid=""
    echo "packaged runtime smoke daemon failed" >&2
    cat "$daemon_err" >&2
    exit 1
fi
daemon_pid=""
if ! grep -q 'packaged-runtime-smoke' "$client_out"; then
    echo "packaged runtime smoke output missing sentinel" >&2
    cat "$client_out" >&2
    cat "$client_err" >&2
    cat "$daemon_err" >&2
    exit 1
fi
printf 'packaged_runtime_smoke=passed\n'
printf 'packaged_runtime_smoke_output=%s\n' "$(grep -m1 'packaged-runtime-smoke' "$client_out")"
