#!/usr/bin/env bash
set -euo pipefail
echo "verifying source-fetch provenance report"
report="$SOURCE_FETCH_REPORT"
require_file() {
    path="$1"
    if [ ! -s "$path" ]; then
        echo "missing or empty source-fetch provenance artifact: $path" >&2
        exit 1
    fi
}
require_line() {
    file="$1"
    pattern="$2"
    description="$3"
    if ! grep -Eq "$pattern" "$file"; then
        echo "missing source-fetch provenance record in $file: $description" >&2
        exit 1
    fi
}
require_exact() {
    file="$1"
    line="$2"
    description="$3"
    if ! grep -Fxq "$line" "$file"; then
        echo "missing source-fetch provenance record in $file: $description" >&2
        exit 1
    fi
}
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
require_lock_section() {
    package="$1"
    expected="$(mktemp)"
    section="$(mktemp)"
    if ! cargo_lock_record "$package" > "$expected"; then
        echo "missing Cargo.lock record for $package" >&2
        rm -f "$expected" "$section"
        exit 1
    fi
    awk -v header="[cargo_lock:$package]" '
        $0 == header { in_section = 1; next }
        in_section && $0 ~ /^\[[^[]/ { exit }
        in_section { print }' "$report" > "$section"
    status=0
    while IFS= read -r line; do
        if [ -n "$line" ] && ! grep -Fxq "$line" "$section"; then
            echo "missing source-fetch provenance Cargo.lock line for $package: $line" >&2
            status=1
            break
        fi
    done < "$expected"
    rm -f "$expected" "$section"
    if [ "$status" -ne 0 ]; then exit "$status"; fi
}
require_file "$report"
require_exact "$report" 'nmux source-fetch provenance sample' 'report title'
require_line "$report" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'generation timestamp'
require_line "$report" '^ghostty_source_mode=(pinned-fetch|local)$' 'Ghostty source mode'
require_line "$report" '^GHOSTTY_SOURCE_DIR=.+$' 'GHOSTTY_SOURCE_DIR field'
require_line "$report" '^GIT_CONFIG_GLOBAL=.+$' 'GIT_CONFIG_GLOBAL field'
require_exact "$report" "Cargo.lock sha256=$(hash_file Cargo.lock)" 'Cargo.lock hash'
require_exact "$report" '[toolchain]' 'toolchain section'
require_line "$report" '^cargo=cargo ' 'cargo version'
require_line "$report" '^rustc=rustc ' 'rustc version'
require_line "$report" '^flatc=flatc version 25\.12\.19$' 'flatc version'
require_line "$report" '^zig=0\.15\.' 'Zig 0.15 version'
require_line "$report" '^ghostty_source_dir_status=(unset|present|missing)$' 'Ghostty source dir status'
require_exact "$report" '[cargo_lock:libghostty-vt]' 'libghostty-vt section'
require_line "$report" '^name = "libghostty-vt"$' 'libghostty-vt package name'
require_line "$report" '^source = "git\+https://github\.com/uzaaft/libghostty-rs\.git\?rev=31d1f70004ff80727e36437cd540984f927333ce#31d1f70004ff80727e36437cd540984f927333ce"$' 'libghostty-vt pinned revision'
require_lock_section libghostty-vt
require_exact "$report" '[cargo_lock:libghostty-vt-sys]' 'libghostty-vt-sys section'
require_line "$report" '^name = "libghostty-vt-sys"$' 'libghostty-vt-sys package name'
require_lock_section libghostty-vt-sys
require_exact "$report" '[policy_note]' 'policy note section'
require_exact "$report" 'This report records local source-fetch inputs for evidence. It does not choose the default or packaged-build source policy.' 'policy note'
printf 'source_fetch_provenance_verified=%s\n' "$report"
