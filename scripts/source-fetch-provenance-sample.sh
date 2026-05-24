#!/usr/bin/env bash
set -euo pipefail
echo "writing source-fetch provenance report"
report_dir=target/source-fetch-provenance
report="$report_dir/SOURCE_FETCH.txt"
hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}
cargo_lock_record() {
    awk -v package="$1" '
        $0 == "[[package]]" { block = $0 ORS; in_block = 1; name = ""; next }
        in_block { block = block $0 ORS }
        in_block && $1 == "name" && $3 == "\"" package "\"" { name = package }
        in_block && $0 == "" { if (name == package) { printf "%s", block; found = 1 } in_block = 0; block = ""; name = "" }
        END { if (in_block && name == package) { printf "%s", block; found = 1 } if (!found) { exit 1 } }' Cargo.lock
}
rm -rf "$report_dir"
mkdir -p "$report_dir"
{
    printf 'nmux source-fetch provenance sample\n'
    printf 'generated_at_utc=%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    printf 'ghostty_source_mode=%s\n' "$([ -n "${GHOSTTY_SOURCE_DIR:-}" ] && printf 'local' || printf 'pinned-fetch')"
    printf 'GHOSTTY_SOURCE_DIR=%s\n' "${GHOSTTY_SOURCE_DIR:-unset}"
    printf 'GIT_CONFIG_GLOBAL=%s\n' "${GIT_CONFIG_GLOBAL:-unset}"
    printf 'Cargo.lock sha256=%s\n' "$(hash_file Cargo.lock)"
    printf '\n[toolchain]\n'
    just toolchain-info
    printf '\n[cargo_lock:libghostty-vt]\n'
    cargo_lock_record libghostty-vt
    printf '\n[cargo_lock:libghostty-vt-sys]\n'
    cargo_lock_record libghostty-vt-sys
    printf '\n[policy_note]\n'
    printf '%s\n' 'This report records local source-fetch inputs for evidence. It does not choose the default or packaged-build source policy.'
} > "$report"
just source-fetch-provenance-verify
printf 'source_fetch_provenance=%s\n' "$report"
