SCHEMA := schema/nmux.fbs
GEN_DIR := crates/nmux-proto/src/generated
FLATC_VERSION := 25.12.19
ZIG_VERSION_PREFIX := 0.15.

.PHONY: check check-all check-ghostty-vt check-schema check-toolchain check-vt-toolchain generate-schema promotion-sample require-cargo require-flatc require-zig rust-test toolchain-info

check: check-toolchain check-schema rust-test

check-all: check check-ghostty-vt

check-ghostty-vt: check-vt-toolchain
	GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-core --features libghostty-vt
	GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-cli --features libghostty-vt

promotion-sample: toolchain-info
	time -p $(MAKE) check-all

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
	@version="$$(flatc --version | awk '{print $$NF}')"; \
	if [ "$$version" != "$(FLATC_VERSION)" ]; then \
		echo "unsupported flatc $$version; expected $(FLATC_VERSION). Use 'nix develop . -c make check' or install matching FlatBuffers" >&2; \
		exit 1; \
	fi

require-zig:
	@command -v zig >/dev/null 2>&1 || { echo "missing zig; use 'nix develop . -c make check-ghostty-vt' or install Zig 0.15 for the optional libghostty-vt build" >&2; exit 127; }
	@version="$$(zig version)"; \
	case "$$version" in \
		$(ZIG_VERSION_PREFIX)*) ;; \
		*) echo "unsupported zig $$version; expected $(ZIG_VERSION_PREFIX)x. Use 'nix develop . -c make check-ghostty-vt' or install Zig 0.15" >&2; exit 1 ;; \
	esac

rust-test: require-cargo
	cargo test --workspace

toolchain-info:
	@printf 'cargo=%s\n' "$$(command -v cargo >/dev/null 2>&1 && cargo --version || printf 'missing')"
	@printf 'rustc=%s\n' "$$(command -v rustc >/dev/null 2>&1 && rustc --version || printf 'missing')"
	@printf 'flatc=%s\n' "$$(command -v flatc >/dev/null 2>&1 && flatc --version || printf 'missing')"
	@printf 'zig=%s\n' "$$(command -v zig >/dev/null 2>&1 && zig version || printf 'missing')"
	@printf 'GHOSTTY_SOURCE_DIR=%s\n' "$${GHOSTTY_SOURCE_DIR:-unset}"
	@printf 'GIT_CONFIG_GLOBAL=%s\n' "$${GIT_CONFIG_GLOBAL:-unset}"
