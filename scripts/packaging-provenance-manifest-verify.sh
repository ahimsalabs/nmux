#!/usr/bin/env bash
set -euo pipefail
echo "verifying opt-in libghostty-vt package provenance manifest"
manifest="$PACKAGING_PROVENANCE_MANIFEST"
pkg_dir=target/packaging-libghostty-vt/package
require_line() {
    pattern="$1"
    description="$2"
    if ! grep -Eq "$pattern" "$manifest"; then
        echo "missing provenance record: $description" >&2
        exit 1
    fi
}
require_file_record() {
    path="$1"
    if ! grep -Eq "^$path bytes=[0-9]+ sha256=[0-9a-f]{64}$" "$manifest"; then
        echo "missing staged file hash record: $path" >&2
        exit 1
    fi
}
require_dynamic_dependency() {
    bin="$1"
    dependency="$2"
    description="$3"
    if ! awk -v bin="$bin" -v dependency="$dependency" '
        $0 == bin || $0 == bin ":" { in_bin = 1; next }
        in_bin && $0 ~ /^target\/packaging-libghostty-vt\/package\/libexec\/nmux:?$/ { exit }
        in_bin && index($0, dependency) { found = 1; exit }
        END { exit(found ? 0 : 1) }' "$manifest"; then
        echo "missing dynamic dependency record for $description" >&2
        exit 1
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
require_cargo_lock_record() {
    package="$1"
    expected_file="$(mktemp)"
    section_file="$(mktemp)"
    if ! cargo_lock_record "$package" > "$expected_file"; then
        echo "missing Cargo.lock record for $package" >&2
        rm -f "$expected_file" "$section_file"
        exit 1
    fi
    awk -v header="[cargo_lock:$package]" '
        $0 == header { in_section = 1; next }
        in_section && $0 ~ /^\[[^[]/ { exit }
        in_section { print }' "$manifest" > "$section_file"
    status=0
    while IFS= read -r line; do
        if [ -n "$line" ] && ! grep -Fxq "$line" "$section_file"; then
            echo "missing Cargo.lock record line for $package: $line" >&2
            exit 1
        fi
    done < "$expected_file" || status="$?"
    rm -f "$expected_file" "$section_file"
    if [ "$status" -ne 0 ]; then exit "$status"; fi
}
test -s "$manifest" || { echo "missing provenance manifest: $manifest" >&2; exit 1; }
require_line '^ghostty_source_mode=(pinned-fetch|local)$' 'ghostty source mode'
require_line '^GHOSTTY_SOURCE_DIR=' 'GHOSTTY_SOURCE_DIR'
require_line '^GIT_CONFIG_GLOBAL=' 'GIT_CONFIG_GLOBAL'
require_line '^\[toolchain\]$' 'toolchain section'
require_line '^cargo=cargo ' 'cargo version'
require_line '^rustc=rustc ' 'rustc version'
require_line '^flatc=flatc version 25\.12\.19$' 'flatc version'
require_line '^zig=0\.15\.' 'Zig 0.15 version'
require_line '^\[cargo_lock\]$' 'Cargo.lock section'
require_line '^Cargo\.lock sha256=[0-9a-f]{64}$' 'Cargo.lock hash'
require_line '^\[cargo_lock:libghostty-vt\]$' 'locked libghostty-vt package section'
require_cargo_lock_record libghostty-vt
require_line '^\[cargo_lock:libghostty-vt-sys\]$' 'locked libghostty-vt-sys package section'
require_cargo_lock_record libghostty-vt-sys
require_line '^\[package_metadata\]$' 'package metadata section'
require_line '^package_format=local-tar-archive-layout$' 'package metadata format'
require_line '^target_host=.+$' 'package metadata target host'
require_line '^terminal_engine=libghostty-vt$' 'package metadata terminal engine'
require_line '^terminal_engine_status=opt-in$' 'package metadata terminal engine status'
require_line '^runtime_library_strategy=bundled dynamic libghostty-vt libraries loaded by wrapper-managed DYLD_LIBRARY_PATH/LD_LIBRARY_PATH$' 'package metadata runtime-library strategy'
require_line '^source_mode=(pinned-fetch|local)$' 'package metadata source mode'
require_line '^\[staged_files\]$' 'staged file section'
require_file_record "$pkg_dir/bin/nmux"
require_file_record "$pkg_dir/PACKAGE_METADATA.txt"
require_file_record "$pkg_dir/libexec/nmux"
require_line "^$pkg_dir/lib/libghostty-vt.* bytes=[0-9]+ sha256=[0-9a-f]{64}$" 'libghostty-vt runtime library hash'
require_line '^\[native_runtime_libraries\]$' 'native runtime library section'
require_line "^$pkg_dir/lib/libghostty-vt" 'native runtime library path'
require_line '^\[dynamic_dependencies\]$' 'dynamic dependencies section'
require_line "^$pkg_dir/libexec/nmux$" 'nmux dynamic dependency heading'
require_dynamic_dependency "$pkg_dir/libexec/nmux" 'libghostty-vt' 'nmux libghostty-vt runtime library'
require_line '^\[cargo_tree\]$' 'cargo tree section'
require_line '^nmux-cli v' 'nmux-cli cargo tree root'
printf 'provenance_manifest_verified=%s\n' "$manifest"
