#!/usr/bin/env bash
set -euo pipefail
echo "staging opt-in libghostty-vt package layout"
pkg_dir=target/packaging-libghostty-vt/package
lib_path="$(find target/packaging-libghostty-vt/release -path '*/ghostty-install/lib/libghostty-vt.*' -print -quit)"
if [ -z "$lib_path" ]; then
    echo "missing packaged libghostty-vt runtime library directory" >&2
    exit 1
fi
lib_dir="$(dirname "$lib_path")"
rm -rf "$pkg_dir"
mkdir -p "$pkg_dir/bin" "$pkg_dir/lib" "$pkg_dir/libexec"
cp target/packaging-libghostty-vt/release/nmux "$pkg_dir/libexec/nmux"
cp "$lib_dir"/libghostty-vt* "$pkg_dir/lib/"
{
    printf 'nmux opt-in native VT package metadata\n'
    printf 'generated_at_utc=%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    printf 'package_format=%s\n' 'local-tar-archive-layout'
    printf 'release_status=%s\n' 'local evidence artifact; not a signed, notarized, installed, or published release package'
    printf 'target_host=%s\n' "$(rustc -vV | awk '/^host: / { print $2 }')"
    printf 'terminal_engine=%s\n' 'libghostty-vt'
    printf 'terminal_engine_status=%s\n' 'opt-in'
    printf 'binaries=%s\n' 'nmux'
    printf 'runtime_library_strategy=%s\n' 'bundled dynamic libghostty-vt libraries loaded by wrapper-managed DYLD_LIBRARY_PATH/LD_LIBRARY_PATH'
    printf 'source_mode=%s\n' "$([ -n "${GHOSTTY_SOURCE_DIR:-}" ] && printf 'local' || printf 'pinned-fetch')"
    printf 'GHOSTTY_SOURCE_DIR=%s\n' "${GHOSTTY_SOURCE_DIR:-unset}"
} > "$pkg_dir/PACKAGE_METADATA.txt"
for bin in nmux; do
    {
        printf '%s\n' '#!/bin/sh'
        printf '%s\n' 'set -eu'
        printf '%s\n' 'bin_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)'
        printf '%s\n' 'lib_dir=$bin_dir/../lib'
        printf '%s\n' 'DYLD_LIBRARY_PATH=$lib_dir${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}'
        printf '%s\n' 'LD_LIBRARY_PATH=$lib_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}'
        printf '%s\n' 'export DYLD_LIBRARY_PATH LD_LIBRARY_PATH'
        printf 'exec "$bin_dir/../libexec/%s" "$@"\n' "$bin"
    } > "$pkg_dir/bin/$bin"
    chmod +x "$pkg_dir/bin/$bin"
done
printf 'package_layout=%s\n' "$pkg_dir"
find "$pkg_dir" -type f | sort
printf 'packaged libghostty-vt nmux version: '
"$pkg_dir/bin/nmux" --version
PACKAGING_LAYOUT="$pkg_dir" just packaging-layout-verify
