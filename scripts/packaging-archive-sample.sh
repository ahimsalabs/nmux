#!/usr/bin/env bash
set -euo pipefail
echo "writing and verifying opt-in libghostty-vt package archive"
pkg_dir=target/packaging-libghostty-vt/package
archive_dir=target/packaging-libghostty-vt/archive
archive="$archive_dir/nmux-libghostty-vt-package.tar.gz"
check_dir="$archive_dir/check"
hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}
rm -rf "$archive_dir"
mkdir -p "$archive_dir" "$check_dir"
tar -C "$pkg_dir/.." -czf "$archive" package
printf '%s  %s\n' "$(hash_file "$archive")" "$archive" > "$archive.sha256"
tar -C "$check_dir" -xzf "$archive"
printf 'archive=%s\n' "$archive"
printf 'archive_sha256=%s\n' "$(cat "$archive.sha256")"
find "$check_dir/package" -type f | sort
printf 'archived libghostty-vt nmux version: '
"$check_dir/package/bin/nmux" --version
PACKAGING_ARCHIVE="$archive" PACKAGING_ARCHIVE_SHA256="$archive.sha256" just packaging-archive-verify
