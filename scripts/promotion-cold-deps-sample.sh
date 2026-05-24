#!/usr/bin/env bash
set -euo pipefail
echo "clearing target/promotion-cold-deps for isolated Cargo dependency/source-fetch validation"
report_dir="target/promotion-cold-deps"
report="$report_dir/REPORT.txt"
log="$report_dir/RUN.log"
rm -rf "$report_dir"
mkdir -p "$report_dir"
report_dir_abs="$(cd "$report_dir" && pwd)"
cargo_home="$report_dir_abs/cargo-home"
cargo_target_dir="$report_dir_abs/target"
start_epoch="$(date -u '+%s')"
start_utc="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
{
    printf 'nmux isolated Cargo dependency/source-fetch sample\n'
    printf 'started_at_utc=%s\n' "$start_utc"
    printf 'sample_scope=%s\n' 'isolated repo-owned CARGO_HOME and CARGO_TARGET_DIR; Nix store, source checkout, and network state may still be warm'
    printf 'CARGO_HOME=%s\n' "$cargo_home"
    printf 'CARGO_TARGET_DIR=%s\n' "$cargo_target_dir"
    printf 'GHOSTTY_SOURCE_DIR=unset\n'
    printf 'GIT_CONFIG_GLOBAL=/dev/null\n'
    printf 'command=just check-all\n'
} > "$report"
if env -u GHOSTTY_SOURCE_DIR CARGO_HOME="$cargo_home" CARGO_TARGET_DIR="$cargo_target_dir" GIT_CONFIG_GLOBAL=/dev/null time -p just check-all > "$log" 2>&1; then
    result=passed
else
    status="$?"
    result=failed
fi
completed_utc="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
end_epoch="$(date -u '+%s')"
real_seconds="$(awk '/^real [0-9]+([.][0-9]+)?$/ { value = $2 } END { if (value != "") print value }' "$log")"
user_seconds="$(awk '/^user [0-9]+([.][0-9]+)?$/ { value = $2 } END { if (value != "") print value }' "$log")"
sys_seconds="$(awk '/^sys [0-9]+([.][0-9]+)?$/ { value = $2 } END { if (value != "") print value }' "$log")"
{
    printf 'completed_at_utc=%s\n' "$completed_utc"
    printf 'elapsed_seconds=%s\n' "$((end_epoch - start_epoch))"
    printf 'check_all_real_seconds=%s\n' "$real_seconds"
    printf 'check_all_user_seconds=%s\n' "$user_seconds"
    printf 'check_all_sys_seconds=%s\n' "$sys_seconds"
    printf 'result=%s\n' "$result"
    printf 'log=%s\n' "$log"
} >> "$report"
cat "$report"
if [ "$result" != passed ]; then
    cat "$log"
    exit "$status"
fi
just promotion-cold-deps-verify
printf 'promotion_cold_deps_sample=%s\n' "$report"
