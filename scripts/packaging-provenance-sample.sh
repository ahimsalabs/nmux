#!/usr/bin/env bash
set -euo pipefail
echo "writing opt-in libghostty-vt package provenance manifest"
pkg_dir=target/packaging-libghostty-vt/package
manifest="$pkg_dir/PROVENANCE.txt"
tree_file="$pkg_dir/CARGO_TREE.txt"
cargo tree --locked -p nmux-cli --features libghostty-vt > "$tree_file"
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
{
    printf 'nmux packaging provenance sample\n'
    printf 'generated_at_utc=%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    printf 'package_layout=%s\n' "$pkg_dir"
    printf 'ghostty_source_mode=%s\n' "$([ -n "${GHOSTTY_SOURCE_DIR:-}" ] && printf 'local' || printf 'pinned-fetch')"
    printf 'GHOSTTY_SOURCE_DIR=%s\n' "${GHOSTTY_SOURCE_DIR:-unset}"
    printf 'GIT_CONFIG_GLOBAL=%s\n' "${GIT_CONFIG_GLOBAL:-unset}"
    printf '\n[toolchain]\n'
    just toolchain-info
    printf '\n[cargo_lock]\n'
    printf 'Cargo.lock sha256=%s\n' "$(hash_file Cargo.lock)"
    printf '\n[cargo_lock:libghostty-vt]\n'
    cargo_lock_record libghostty-vt
    printf '\n[cargo_lock:libghostty-vt-sys]\n'
    cargo_lock_record libghostty-vt-sys
    printf '\n[package_metadata]\n'
    cat "$pkg_dir/PACKAGE_METADATA.txt"
    printf '\n[staged_files]\n'
    find "$pkg_dir" -type f ! -name PROVENANCE.txt | sort | while read -r file; do
        printf '%s bytes=%s sha256=%s\n' "$file" "$(wc -c < "$file" | tr -d ' ')" "$(hash_file "$file")"
    done
    printf '\n[native_runtime_libraries]\n'
    find "$pkg_dir/lib" -type f | sort
    printf '\n[dynamic_dependencies]\n'
    if command -v otool >/dev/null 2>&1; then
        for bin in "$pkg_dir/libexec/nmux"; do
            printf '%s\n' "$bin"
            otool -L "$bin"
        done
    elif command -v ldd >/dev/null 2>&1; then
        for bin in "$pkg_dir/libexec/nmux"; do
            printf '%s\n' "$bin"
            ldd "$bin"
        done
    else
        printf 'dynamic dependency inspector unavailable\n'
    fi
    printf '\n[cargo_tree]\n'
    cat "$tree_file"
} > "$manifest"
printf 'provenance_manifest=%s\n' "$manifest"
