# nmux justfile

SCHEMA := "schema/nmux.fbs"
GEN_DIR := "crates/nmux-proto/src/generated"
FLATC_VERSION := "25.12.19"
ZIG_VERSION_PREFIX := "0.15."
RENDERER_EQUIVALENCE_ARTIFACT_DIR := "target/renderer-equivalence"
RENDERER_EQUIVALENCE_ORACLE_DIR := ""

export PROMOTION_EVIDENCE_DIR := env_var_or_default("PROMOTION_EVIDENCE_DIR", "target/promotion-evidence")
export SOURCE_FETCH_REPORT := env_var_or_default("SOURCE_FETCH_REPORT", "target/source-fetch-provenance/SOURCE_FETCH.txt")
export SOURCE_FETCH_OFFLINE_PROBE_REPORT := env_var_or_default("SOURCE_FETCH_OFFLINE_PROBE_REPORT", "target/source-fetch-offline/OFFLINE_PROBE.txt")
export SOURCE_FETCH_OFFLINE_PROBE_LOG := env_var_or_default("SOURCE_FETCH_OFFLINE_PROBE_LOG", "")
export PACKAGING_LAYOUT := env_var_or_default("PACKAGING_LAYOUT", "target/packaging-libghostty-vt/package")
export PACKAGING_PROVENANCE_MANIFEST := env_var_or_default("PACKAGING_PROVENANCE_MANIFEST", "target/packaging-libghostty-vt/package/PROVENANCE.txt")
export PACKAGING_ARCHIVE := env_var_or_default("PACKAGING_ARCHIVE", "target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz")
export PACKAGING_ARCHIVE_SHA256 := env_var_or_default("PACKAGING_ARCHIVE_SHA256", PACKAGING_ARCHIVE + ".sha256")

# --- Core ---

check: check-vt-toolchain check-schema rust-test

check-all: check check-interim

check-ghostty-vt: check

check-interim: check-toolchain check-schema
    cargo test -p nmux-core --no-default-features
    cargo test -p nmux-cli --no-default-features

rust-test: require-cargo
    RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null cargo test --workspace

# --- Schema ---

check-schema: require-flatc
    flatc --json --strict-json --no-warnings -o /tmp {{SCHEMA}}

generate-schema: require-flatc
    rm -rf {{GEN_DIR}}
    mkdir -p {{GEN_DIR}}
    flatc --rust -o {{GEN_DIR}} {{SCHEMA}}

# --- Toolchain ---

check-toolchain: require-flatc require-cargo

check-vt-toolchain: check-toolchain require-zig require-ghostty-source

[private]
require-cargo:
    #!/usr/bin/env bash
    set -eu
    command -v cargo >/dev/null 2>&1 || { echo "missing cargo; use 'nix develop . -c just check' or install a Rust toolchain compatible with this workspace" >&2; exit 127; }

[private]
require-flatc:
    #!/usr/bin/env bash
    set -eu
    command -v flatc >/dev/null 2>&1 || { echo "missing flatc; use 'nix develop . -c just check' or install FlatBuffers compatible with the pinned Rust flatbuffers crate" >&2; exit 127; }
    version="$(flatc --version | awk '{print $NF}')"
    if [ "$version" != "{{FLATC_VERSION}}" ]; then
        echo "unsupported flatc $version; expected {{FLATC_VERSION}}. Use 'nix develop . -c just check' or install matching FlatBuffers" >&2
        exit 1
    fi

[private]
require-zig:
    #!/usr/bin/env bash
    set -eu
    command -v zig >/dev/null 2>&1 || { echo "missing zig; use 'nix develop . -c just check' or install Zig 0.15 for the default libghostty-vt build" >&2; exit 127; }
    version="$(zig version)"
    case "$version" in
        {{ZIG_VERSION_PREFIX}}*) ;;
        *) echo "unsupported zig $version; expected {{ZIG_VERSION_PREFIX}}x. Use 'nix develop . -c just check' or install Zig 0.15" >&2; exit 1 ;;
    esac

[private]
require-ghostty-source:
    #!/usr/bin/env bash
    set -eu
    if [ -n "${GHOSTTY_SOURCE_DIR:-}" ] && [ ! -d "${GHOSTTY_SOURCE_DIR}" ]; then
        echo "invalid GHOSTTY_SOURCE_DIR=${GHOSTTY_SOURCE_DIR}; expected a readable Ghostty source directory or unset it to use the pinned libghostty-vt-sys fetch" >&2
        exit 1
    fi

toolchain-info:
    #!/usr/bin/env bash
    set -eu
    printf 'cargo=%s\n' "$(command -v cargo >/dev/null 2>&1 && cargo --version || printf 'missing')"
    printf 'rustc=%s\n' "$(command -v rustc >/dev/null 2>&1 && rustc --version || printf 'missing')"
    printf 'flatc=%s\n' "$(command -v flatc >/dev/null 2>&1 && flatc --version || printf 'missing')"
    printf 'zig=%s\n' "$(command -v zig >/dev/null 2>&1 && zig version || printf 'missing')"
    printf 'GHOSTTY_SOURCE_DIR=%s\n' "${GHOSTTY_SOURCE_DIR:-unset}"
    printf 'ghostty_source_mode=%s\n' "$([ -n "${GHOSTTY_SOURCE_DIR:-}" ] && printf 'local' || printf 'pinned-fetch')"
    printf 'ghostty_source_dir_status=%s\n' "$([ -z "${GHOSTTY_SOURCE_DIR:-}" ] && printf 'unset' || { [ -d "${GHOSTTY_SOURCE_DIR}" ] && printf 'present' || printf 'missing'; })"
    printf 'GIT_CONFIG_GLOBAL=%s\n' "${GIT_CONFIG_GLOBAL:-unset}"

# --- Source audit ---

source-audit:
    #!/usr/bin/env bash
    set -euo pipefail
    system="$(nix eval --impure --raw --expr builtins.currentSystem)"
    out="$(nix build ".#checks.$system.source-audit" --no-link --print-out-paths)"
    cat "$out/source-audit.txt"

# --- Smoke & verification ---

local-smoke: check-toolchain
    @scripts/local-smoke.sh

static-link-verify: check-vt-toolchain
    cargo run -p xtask -- static-link-verify

# --- Renderer equivalence ---

renderer-equivalence-smoke: check-vt-toolchain
    RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-core --features libghostty-vt renderer_equivalence
    RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-cli --features libghostty-vt --test renderer_equivalence

renderer-equivalence-artifacts: check-vt-toolchain
    rm -rf "{{RENDERER_EQUIVALENCE_ARTIFACT_DIR}}"
    RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null NMUX_RENDERER_EQUIVALENCE_ARTIFACT_DIR="{{RENDERER_EQUIVALENCE_ARTIFACT_DIR}}" cargo test -p nmux-cli --features libghostty-vt --test renderer_equivalence

renderer-equivalence-compare: check-vt-toolchain
    #!/usr/bin/env bash
    set -euo pipefail
    test -n "{{RENDERER_EQUIVALENCE_ORACLE_DIR}}" || { echo "set RENDERER_EQUIVALENCE_ORACLE_DIR=/path/to/oracle-canonical-artifacts" >&2; exit 1; }
    RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null NMUX_RENDERER_EQUIVALENCE_ORACLE_DIR="{{RENDERER_EQUIVALENCE_ORACLE_DIR}}" cargo test -p nmux-cli --features libghostty-vt --test renderer_equivalence

# --- Promotion ---

promotion-sample: toolchain-info
    time -p just check-all

promotion-cold-target-sample: toolchain-info
    #!/usr/bin/env bash
    set -euo pipefail
    echo "clearing target/promotion-cold for cold target-dir validation"
    rm -rf target/promotion-cold
    time -p env CARGO_TARGET_DIR=target/promotion-cold just check-all

promotion-cold-deps-sample: toolchain-info
    @scripts/promotion-cold-deps-sample.sh

promotion-cold-deps-verify:
    cargo run -p xtask -- promotion-cold-deps-verify

promotion-local-sample:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "== promotion local sample: source-fetch provenance =="
    just source-fetch-provenance-sample
    echo "== promotion local sample: validation =="
    just promotion-sample
    echo "== promotion local sample: local workflow smoke =="
    just local-smoke
    echo "== promotion local sample: cache-present offline source-fetch probe =="
    just source-fetch-offline-probe
    echo "== promotion local sample: packaging archive runtime smoke =="
    just packaging-archive-runtime-smoke

promotion-evidence-bundle:
    @scripts/promotion-evidence-bundle.sh

promotion-evidence-verify:
    cargo run -p xtask -- promotion-evidence-verify

# --- Source fetch ---

source-fetch-provenance-sample: toolchain-info
    @scripts/source-fetch-provenance-sample.sh

source-fetch-provenance-verify:
    cargo run -p xtask -- source-fetch-provenance-verify

source-fetch-offline-probe: check-vt-toolchain
    @scripts/source-fetch-offline-probe.sh

source-fetch-offline-probe-verify:
    cargo run -p xtask -- source-fetch-offline-probe-verify

# --- Packaging ---

packaging-sample: toolchain-info check-vt-toolchain
    @scripts/packaging-sample.sh

packaging-layout-sample: packaging-sample
    @scripts/packaging-layout-sample.sh

packaging-layout-verify:
    cargo run -p xtask -- packaging-layout-verify

packaging-provenance-sample: packaging-layout-sample
    @scripts/packaging-provenance-sample.sh

packaging-provenance-verify: packaging-provenance-sample
    @just packaging-provenance-manifest-verify

packaging-provenance-manifest-verify:
    cargo run -p xtask -- packaging-provenance-manifest-verify

packaging-archive-sample: packaging-provenance-verify
    @scripts/packaging-archive-sample.sh

packaging-archive-verify:
    cargo run -p xtask -- packaging-archive-verify

packaging-archive-runtime-smoke: packaging-archive-sample
    @scripts/packaging-archive-runtime-smoke.sh
