#!/usr/bin/env bash
set -euo pipefail
echo "running cache-present source-fetch offline probe"
report_dir=target/source-fetch-offline
report="$report_dir/OFFLINE_PROBE.txt"
log="$report_dir/OFFLINE_PROBE.log"
start_epoch="$(date -u '+%s')"
start_utc="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
rm -rf "$report_dir"
mkdir -p "$report_dir"
status=0
env CARGO_NET_OFFLINE=true GIT_CONFIG_GLOBAL=/dev/null CARGO_TARGET_DIR=target/source-fetch-offline cargo test -p nmux-core --features libghostty-vt --no-run > "$log" 2>&1 || status="$?"
end_epoch="$(date -u '+%s')"
completed_utc="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
{
    printf 'nmux source-fetch offline probe\n'
    printf 'generated_at_utc=%s\n' "$completed_utc"
    printf 'started_at_utc=%s\n' "$start_utc"
    printf 'completed_at_utc=%s\n' "$completed_utc"
    printf 'elapsed_seconds=%s\n' "$((end_epoch - start_epoch))"
    printf 'probe_scope=%s\n' 'cache-present opt-in native VT build only; not cold checkout, CI cache miss, network-failure, or default/package source policy evidence'
    printf 'ghostty_source_mode=%s\n' "$([ -n "${GHOSTTY_SOURCE_DIR:-}" ] && printf 'local' || printf 'pinned-fetch')"
    printf 'GHOSTTY_SOURCE_DIR=%s\n' "${GHOSTTY_SOURCE_DIR:-unset}"
    printf 'CARGO_NET_OFFLINE=%s\n' 'true'
    printf 'GIT_CONFIG_GLOBAL=%s\n' '/dev/null'
    printf 'CARGO_TARGET_DIR=%s\n' 'target/source-fetch-offline'
    printf 'package=%s\n' 'nmux-core'
    printf 'features=%s\n' 'libghostty-vt'
    printf 'command=%s\n' 'cargo test -p nmux-core --features libghostty-vt --no-run'
    printf 'log=%s\n' "$log"
    if [ "$status" -eq 0 ]; then
        printf 'source_fetch_offline_probe=passed\n'
    else
        printf 'source_fetch_offline_probe=failed\n'
        printf 'exit_status=%s\n' "$status"
    fi
} > "$report"
cat "$log"
if [ "$status" -ne 0 ]; then
    echo "source-fetch offline probe failed; see $report and $log" >&2
    exit "$status"
fi
just source-fetch-offline-probe-verify
printf 'source_fetch_offline_probe=%s\n' "$report"
