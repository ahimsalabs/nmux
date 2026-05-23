SCHEMA := schema/nmux.fbs
GEN_DIR := crates/nmux-proto/src/generated

.PHONY: check check-all check-ghostty-vt check-schema check-toolchain check-vt-toolchain generate-schema require-cargo require-flatc require-zig rust-test

check: check-toolchain check-schema rust-test

check-all: check check-ghostty-vt

check-ghostty-vt: check-vt-toolchain
	GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-core --features libghostty-vt
	GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-cli --features libghostty-vt

check-schema: require-flatc
	flatc --json --strict-json --no-warnings -o /tmp $(SCHEMA)

check-toolchain: require-flatc require-cargo

check-vt-toolchain: check-toolchain require-zig

generate-schema: require-flatc
	rm -rf $(GEN_DIR)
	mkdir -p $(GEN_DIR)
	flatc --rust -o $(GEN_DIR) $(SCHEMA)

require-cargo:
	@command -v cargo >/dev/null 2>&1 || { echo "missing cargo; use 'nix develop . -c make check' or install a Rust toolchain compatible with this workspace" >&2; exit 127; }

require-flatc:
	@command -v flatc >/dev/null 2>&1 || { echo "missing flatc; use 'nix develop . -c make check' or install FlatBuffers compatible with the pinned Rust flatbuffers crate" >&2; exit 127; }

require-zig:
	@command -v zig >/dev/null 2>&1 || { echo "missing zig; use 'nix develop . -c make check-ghostty-vt' or install Zig 0.15 for the optional libghostty-vt build" >&2; exit 127; }

rust-test: require-cargo
	cargo test --workspace
