#!/usr/bin/env bash
set -euo pipefail
echo "verifying isolated Cargo dependency/source-fetch sample"
report_dir="target/promotion-cold-deps"
report="$report_dir/REPORT.txt"
require_file() {
    path="$1"
    if [ ! -s "$path" ]; then
        echo "missing or empty cold-deps artifact: $path" >&2
        exit 1
    fi
}
require_line() {
    file="$1"
    pattern="$2"
    description="$3"
    if ! grep -Eq "$pattern" "$file"; then
        echo "missing cold-deps record in $file: $description" >&2
        exit 1
    fi
}
require_exact() {
    file="$1"
    line="$2"
    description="$3"
    if ! grep -Fxq "$line" "$file"; then
        echo "missing cold-deps record in $file: $description" >&2
        exit 1
    fi
}
require_file "$report"
log="$(awk -F= '/^log=/{print $2; exit}' "$report")"
require_file "$log"
require_exact "$report" 'nmux isolated Cargo dependency/source-fetch sample' 'report title'
require_line "$report" '^started_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'start timestamp'
require_line "$report" '^completed_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'completion timestamp'
require_exact "$report" 'sample_scope=isolated repo-owned CARGO_HOME and CARGO_TARGET_DIR; Nix store, source checkout, and network state may still be warm' 'sample scope'
require_line "$report" '^CARGO_HOME=.*/target/promotion-cold-deps/cargo-home$' 'isolated Cargo home'
require_line "$report" '^CARGO_TARGET_DIR=.*/target/promotion-cold-deps/target$' 'isolated target dir'
require_exact "$report" 'GHOSTTY_SOURCE_DIR=unset' 'Ghostty source env'
require_exact "$report" 'GIT_CONFIG_GLOBAL=/dev/null' 'Git config isolation'
require_exact "$report" 'command=just check-all' 'sample command'
require_line "$report" '^elapsed_seconds=[0-9]+$' 'elapsed seconds'
require_line "$report" '^check_all_real_seconds=[0-9]+([.][0-9]+)?$' 'real timing'
require_line "$report" '^check_all_user_seconds=[0-9]+([.][0-9]+)?$' 'user timing'
require_line "$report" '^check_all_sys_seconds=[0-9]+([.][0-9]+)?$' 'sys timing'
require_exact "$report" 'result=passed' 'sample result'
require_line "$report" '^log=target/promotion-cold-deps/RUN\.log$' 'run log path'
log_real="$(awk '/^real [0-9]+([.][0-9]+)?$/ { value = $2 } END { if (value != "") print value }' "$log")"
log_user="$(awk '/^user [0-9]+([.][0-9]+)?$/ { value = $2 } END { if (value != "") print value }' "$log")"
log_sys="$(awk '/^sys [0-9]+([.][0-9]+)?$/ { value = $2 } END { if (value != "") print value }' "$log")"
if [ -z "$log_real" ] || [ -z "$log_user" ] || [ -z "$log_sys" ]; then
    echo "missing time -p result in $log" >&2
    exit 1
fi
require_exact "$report" "check_all_real_seconds=$log_real" 'real timing matches log'
require_exact "$report" "check_all_user_seconds=$log_user" 'user timing matches log'
require_exact "$report" "check_all_sys_seconds=$log_sys" 'sys timing matches log'
require_line "$log" '^flatc --json --strict-json --no-warnings -o /tmp schema/nmux\.fbs$' 'schema check ran'
require_line "$log" '^RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null cargo test --workspace$' 'default Ghostty workspace tests ran'
require_line "$log" '^cargo test -p nmux-core --no-default-features$' 'no-default-features core tests ran'
require_line "$log" '^cargo test -p nmux-cli --no-default-features$' 'no-default-features cli tests ran'
printf 'promotion_cold_deps_verified=%s\n' "$report"
