#!/usr/bin/env bash
set -euo pipefail
echo "verifying existing opt-in libghostty-vt package archive"
archive="$PACKAGING_ARCHIVE"
archive_sha_file="$PACKAGING_ARCHIVE_SHA256"
work_dir="$(mktemp -d "/tmp/nmuxpkg-verify.XXXXXX")"
trap 'status=$?; rm -rf "$work_dir"; exit "$status"' EXIT INT TERM
hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}
require_file() {
    path="$1"
    if [ ! -s "$path" ]; then
        echo "missing or empty archive artifact: $path" >&2
        exit 1
    fi
}
require_line() {
    file="$1"
    pattern="$2"
    description="$3"
    if ! grep -Eq "$pattern" "$file"; then
        echo "missing archive record in $file: $description" >&2
        exit 1
    fi
}
require_file_record() {
    rel="$1"
    file="$pkg_dir/$rel"
    staged="target/packaging-libghostty-vt/package/$rel"
    require_file "$file"
    bytes="$(wc -c < "$file" | tr -d ' ')"
    sha="$(hash_file "$file")"
    if ! grep -Fxq "$staged bytes=$bytes sha256=$sha" "$provenance"; then
        echo "missing or mismatched staged file hash record: $staged" >&2
        exit 1
    fi
}
require_file "$archive"
require_file "$archive_sha_file"
expected_sha="$(awk 'NR == 1 { print $1 }' "$archive_sha_file")"
if ! printf '%s\n' "$expected_sha" | grep -Eq '^[0-9a-f]{64}$'; then
    echo "invalid archive SHA-256 record: $archive_sha_file" >&2
    exit 1
fi
actual_sha="$(hash_file "$archive")"
if [ "$actual_sha" != "$expected_sha" ]; then
    echo "archive SHA-256 mismatch: $archive" >&2
    echo "expected $expected_sha" >&2
    echo "actual   $actual_sha" >&2
    exit 1
fi
if tar -tzf "$archive" | awk '$0 ~ /^\// || $0 ~ /(^|\/)\.\.(\/|$)/ { bad = 1 } END { exit bad ? 0 : 1 }'; then
    echo "archive contains unsafe absolute or parent-relative paths: $archive" >&2
    exit 1
fi
tar -C "$work_dir" -xzf "$archive"
pkg_dir="$work_dir/package"
metadata="$pkg_dir/PACKAGE_METADATA.txt"
provenance="$pkg_dir/PROVENANCE.txt"
cargo_tree="$pkg_dir/CARGO_TREE.txt"
require_file "$metadata"
require_file "$provenance"
require_file "$cargo_tree"
require_file_record "bin/nmux"
require_file_record "PACKAGE_METADATA.txt"
require_file_record "CARGO_TREE.txt"
require_file_record "libexec/nmux"
found_runtime_library=0
for lib in "$pkg_dir"/lib/libghostty-vt*; do
    if [ -f "$lib" ]; then
        found_runtime_library=1
        require_file_record "lib/$(basename "$lib")"
    fi
done
if [ "$found_runtime_library" -ne 1 ]; then
    echo "missing libghostty-vt runtime library in archive: $archive" >&2
    exit 1
fi
require_line "$metadata" '^package_format=local-tar-archive-layout$' 'package metadata format'
require_line "$metadata" '^terminal_engine=libghostty-vt$' 'package metadata terminal engine'
require_line "$metadata" '^terminal_engine_status=opt-in$' 'package metadata terminal engine status'
require_line "$metadata" '^runtime_library_strategy=bundled dynamic libghostty-vt libraries loaded by wrapper-managed DYLD_LIBRARY_PATH/LD_LIBRARY_PATH$' 'package metadata runtime-library strategy'
require_line "$provenance" '^\[package_metadata\]$' 'provenance package metadata section'
require_line "$provenance" '^\[staged_files\]$' 'provenance staged file section'
require_line "$provenance" '^\[native_runtime_libraries\]$' 'provenance runtime library section'
require_line "$provenance" '^\[dynamic_dependencies\]$' 'provenance dynamic dependencies section'
require_line "$provenance" '^\[cargo_tree\]$' 'provenance cargo tree section'
require_line "$provenance" '^target/packaging-libghostty-vt/package/libexec/nmux$' 'nmux dynamic dependency heading'
if ! awk '
    $0 == "target/packaging-libghostty-vt/package/libexec/nmux" { in_nmux = 1; next }
    in_nmux && index($0, "libghostty-vt") { found_nmux = 1 }
    END { exit(found_nmux ? 0 : 1) }' "$provenance"; then
    echo "missing libghostty-vt dynamic dependency records in archive provenance" >&2
    exit 1
fi
require_line "$cargo_tree" '^nmux-cli v' 'cargo tree root'
PACKAGING_LAYOUT="$pkg_dir" just packaging-layout-verify
printf 'packaging_archive_verified=%s\n' "$archive"
