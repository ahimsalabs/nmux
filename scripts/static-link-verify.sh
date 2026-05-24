#!/usr/bin/env bash
set -euo pipefail
echo "building libghostty-vt release binary for static-link verification"
GIT_CONFIG_GLOBAL=/dev/null CARGO_TARGET_DIR=target/static-link-verify cargo build -p nmux-cli --release --bin nmux
bin=target/static-link-verify/release/nmux
deps=target/static-link-verify/DYNAMIC_DEPS.txt
if command -v otool >/dev/null 2>&1; then
    otool -L "$bin" > "$deps"
elif command -v ldd >/dev/null 2>&1; then
    ldd "$bin" > "$deps"
else
    echo "dynamic dependency inspector unavailable" >&2
    exit 1
fi
if grep -Ei 'libghostty-vt|ghostty-vt' "$deps"; then
    echo "release nmux dynamically depends on libghostty-vt; static linking is required" >&2
    exit 1
fi
printf 'static_link_verified=%s\n' "$bin"
printf 'dynamic_dependency_report=%s\n' "$deps"
