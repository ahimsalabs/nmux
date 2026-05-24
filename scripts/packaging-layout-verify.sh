#!/usr/bin/env bash
set -euo pipefail
echo "verifying existing opt-in libghostty-vt package layout"
pkg_dir="$PACKAGING_LAYOUT"
metadata="$pkg_dir/PACKAGE_METADATA.txt"
require_file() {
    path="$1"
    if [ ! -s "$path" ]; then
        echo "missing or empty package layout artifact: $path" >&2
        exit 1
    fi
}
require_executable() {
    path="$1"
    require_file "$path"
    if [ ! -x "$path" ]; then
        echo "package layout artifact is not executable: $path" >&2
        exit 1
    fi
}
require_line() {
    file="$1"
    pattern="$2"
    description="$3"
    if ! grep -Eq "$pattern" "$file"; then
        echo "missing package layout record in $file: $description" >&2
        exit 1
    fi
}
require_wrapper_line() {
    wrapper="$1"
    line="$2"
    description="$3"
    if ! grep -Fxq "$line" "$wrapper"; then
        echo "missing wrapper record in $wrapper: $description" >&2
        exit 1
    fi
}
require_executable "$pkg_dir/bin/nmux"
require_executable "$pkg_dir/libexec/nmux"
require_file "$metadata"
found_runtime_library=0
for lib in "$pkg_dir"/lib/libghostty-vt*; do
    if [ -f "$lib" ]; then
        require_file "$lib"
        found_runtime_library=1
    fi
done
if [ "$found_runtime_library" -ne 1 ]; then
    echo "missing libghostty-vt runtime library in package layout: $pkg_dir/lib" >&2
    exit 1
fi
require_line "$metadata" '^nmux opt-in native VT package metadata$' 'metadata title'
require_line "$metadata" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'metadata timestamp'
require_line "$metadata" '^package_format=local-tar-archive-layout$' 'package format'
require_line "$metadata" '^release_status=local evidence artifact; not a signed, notarized, installed, or published release package$' 'release status'
require_line "$metadata" '^target_host=.+$' 'target host'
require_line "$metadata" '^terminal_engine=libghostty-vt$' 'terminal engine'
require_line "$metadata" '^terminal_engine_status=opt-in$' 'terminal engine status'
require_line "$metadata" '^binaries=nmux$' 'binary list'
require_line "$metadata" '^runtime_library_strategy=staged libghostty-vt native library artifacts for packaging evidence; nmux must not dynamically depend on libghostty-vt$' 'runtime-library strategy'
require_line "$metadata" '^source_mode=(pinned-fetch|local)$' 'source mode'
require_line "$metadata" '^GHOSTTY_SOURCE_DIR=.+$' 'GHOSTTY_SOURCE_DIR'
for bin in nmux; do
    wrapper="$pkg_dir/bin/$bin"
    require_wrapper_line "$wrapper" '#!/bin/sh' 'shell shebang'
    require_wrapper_line "$wrapper" 'set -eu' 'strict shell mode'
    require_wrapper_line "$wrapper" 'bin_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)' 'relative wrapper directory'
    require_wrapper_line "$wrapper" 'lib_dir=$bin_dir/../lib' 'relative library directory'
    require_wrapper_line "$wrapper" 'DYLD_LIBRARY_PATH=$lib_dir${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}' 'DYLD library path'
    require_wrapper_line "$wrapper" 'LD_LIBRARY_PATH=$lib_dir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}' 'LD library path'
    require_wrapper_line "$wrapper" 'export DYLD_LIBRARY_PATH LD_LIBRARY_PATH' 'library path export'
    require_wrapper_line "$wrapper" "exec \"\$bin_dir/../libexec/$bin\" \"\$@\"" 'relative libexec handoff'
done
printf 'packaged libghostty-vt nmux version: '
env -u DYLD_LIBRARY_PATH -u LD_LIBRARY_PATH "$pkg_dir/bin/nmux" --version
printf 'packaging_layout_verified=%s\n' "$pkg_dir"
