#!/usr/bin/env bash
set -euo pipefail
echo "building default release binaries"
CARGO_TARGET_DIR=target/packaging-default cargo build -p nmux-cli --release --bins
echo "default release artifacts"
for bin in target/packaging-default/release/nmux; do
    printf '%s bytes=%s\n' "$bin" "$(wc -c < "$bin" | tr -d ' ')"
done
printf 'default nmux version: '
if ! target/packaging-default/release/nmux --version; then
    echo "default nmux version check failed" >&2
    exit 1
fi
echo "building default libghostty-vt release binaries"
GIT_CONFIG_GLOBAL=/dev/null CARGO_TARGET_DIR=target/packaging-libghostty-vt cargo build -p nmux-cli --release --bins
echo "default libghostty-vt release artifacts"
for bin in target/packaging-libghostty-vt/release/nmux; do
    printf '%s bytes=%s\n' "$bin" "$(wc -c < "$bin" | tr -d ' ')"
done
echo "default libghostty-vt dynamic library artifacts"
find target/packaging-libghostty-vt/release -name 'libghostty-vt*.dylib' -o -name 'libghostty-vt*.so' -o -name 'libghostty-vt*.dll'
status=0
lib_path="$(find target/packaging-libghostty-vt/release -path '*/ghostty-install/lib/libghostty-vt.*' -print -quit)"
if [ -z "$lib_path" ]; then
    echo "missing packaged libghostty-vt runtime library directory" >&2
    exit 1
fi
lib_dir="$(dirname "$lib_path")"
printf 'libghostty-vt_runtime_library_dir=%s\n' "$lib_dir"
printf 'libghostty-vt nmux version: '
if ! DYLD_LIBRARY_PATH="$lib_dir" LD_LIBRARY_PATH="$lib_dir" target/packaging-libghostty-vt/release/nmux --version; then
    echo "libghostty-vt nmux version check failed" >&2
    status=1
fi
if [ "$status" -ne 0 ]; then
    echo "one or more default libghostty-vt binary version checks failed; record this as packaging evidence" >&2
    exit "$status"
fi
