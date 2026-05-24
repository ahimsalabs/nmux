#!/usr/bin/env bash
set -euo pipefail
echo "verifying source-fetch offline probe report"
report="$SOURCE_FETCH_OFFLINE_PROBE_REPORT"
override_log="$SOURCE_FETCH_OFFLINE_PROBE_LOG"
require_file() {
    path="$1"
    if [ ! -s "$path" ]; then
        echo "missing or empty source-fetch offline probe artifact: $path" >&2
        exit 1
    fi
}
require_line() {
    file="$1"
    pattern="$2"
    description="$3"
    if ! grep -Eq "$pattern" "$file"; then
        echo "missing source-fetch offline probe record in $file: $description" >&2
        exit 1
    fi
}
require_exact() {
    file="$1"
    line="$2"
    description="$3"
    if ! grep -Fxq "$line" "$file"; then
        echo "missing source-fetch offline probe record in $file: $description" >&2
        exit 1
    fi
}
require_file "$report"
if [ -n "$override_log" ]; then
    log="$override_log"
else
    log="$(awk -F= '/^log=/{print $2; exit}' "$report")"
fi
require_file "$log"
require_exact "$report" 'nmux source-fetch offline probe' 'report title'
require_line "$report" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'generation timestamp'
require_line "$report" '^started_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'start timestamp'
require_line "$report" '^completed_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'completion timestamp'
require_line "$report" '^elapsed_seconds=[0-9]+$' 'elapsed seconds'
require_exact "$report" 'probe_scope=cache-present opt-in native VT build only; not cold checkout, CI cache miss, network-failure, or default/package source policy evidence' 'probe scope'
require_line "$report" '^ghostty_source_mode=(pinned-fetch|local)$' 'Ghostty source mode'
require_line "$report" '^GHOSTTY_SOURCE_DIR=.+$' 'GHOSTTY_SOURCE_DIR field'
require_exact "$report" 'CARGO_NET_OFFLINE=true' 'Cargo offline mode'
require_exact "$report" 'GIT_CONFIG_GLOBAL=/dev/null' 'Git config isolation'
require_exact "$report" 'CARGO_TARGET_DIR=target/source-fetch-offline' 'target dir'
require_exact "$report" 'package=nmux-core' 'package'
require_exact "$report" 'features=libghostty-vt' 'features'
require_exact "$report" 'command=cargo test -p nmux-core --features libghostty-vt --no-run' 'command'
require_exact "$report" 'log=target/source-fetch-offline/OFFLINE_PROBE.log' 'log path'
require_exact "$report" 'source_fetch_offline_probe=passed' 'probe result'
require_line "$log" '^   Compiling libghostty-vt-sys v0\.1\.1$|^    Checking libghostty-vt-sys v0\.1\.1$|^    Finished `test` profile ' 'opt-in native VT build activity'
require_line "$log" '^  Executable unittests src/lib\.rs \(target/source-fetch-offline/debug/deps/nmux_core-.+\)$' 'nmux-core test binary'
printf 'source_fetch_offline_probe_verified=%s\n' "$report"
