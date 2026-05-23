SCHEMA := schema/nmux.fbs
GEN_DIR := crates/nmux-proto/src/generated
FLATC_VERSION := 25.12.19
ZIG_VERSION_PREFIX := 0.15.

.PHONY: check check-all check-ghostty-vt check-schema check-toolchain check-vt-toolchain generate-schema packaging-layout-sample packaging-sample promotion-sample require-cargo require-flatc require-ghostty-source require-zig rust-test toolchain-info

check: check-toolchain check-schema rust-test

check-all: check check-ghostty-vt

check-ghostty-vt: check-vt-toolchain
	GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-core --features libghostty-vt
	GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-cli --features libghostty-vt

promotion-sample: toolchain-info
	time -p $(MAKE) check-all

packaging-sample: toolchain-info check-vt-toolchain
	@echo "building default release binaries"
	CARGO_TARGET_DIR=target/packaging-default cargo build -p nmux-cli --release --bins
	@echo "default release artifacts"
	@for bin in target/packaging-default/release/nmux target/packaging-default/release/nmuxd; do \
		printf '%s bytes=%s\n' "$$bin" "$$(wc -c < "$$bin" | tr -d ' ')"; \
	done
	@printf 'default nmux version: '
	@if ! target/packaging-default/release/nmux --version; then \
		echo "default nmux version check failed" >&2; \
		exit 1; \
	fi
	@printf 'default nmuxd version: '
	@if ! target/packaging-default/release/nmuxd --version; then \
		echo "default nmuxd version check failed" >&2; \
		exit 1; \
	fi
	@echo "building opt-in libghostty-vt release binaries"
	GIT_CONFIG_GLOBAL=/dev/null CARGO_TARGET_DIR=target/packaging-libghostty-vt cargo build -p nmux-cli --release --bins --features libghostty-vt
	@echo "opt-in libghostty-vt release artifacts"
	@for bin in target/packaging-libghostty-vt/release/nmux target/packaging-libghostty-vt/release/nmuxd; do \
		printf '%s bytes=%s\n' "$$bin" "$$(wc -c < "$$bin" | tr -d ' ')"; \
	done
	@echo "opt-in libghostty-vt dynamic library artifacts"
	@find target/packaging-libghostty-vt/release -name 'libghostty-vt*.dylib' -o -name 'libghostty-vt*.so' -o -name 'libghostty-vt*.dll'
	@status=0; \
	lib_path="$$(find target/packaging-libghostty-vt/release -path '*/ghostty-install/lib/libghostty-vt.*' -print -quit)"; \
	if [ -z "$$lib_path" ]; then \
		echo "missing packaged libghostty-vt runtime library directory" >&2; \
		exit 1; \
	fi; \
	lib_dir="$$(dirname "$$lib_path")"; \
	printf 'libghostty-vt_runtime_library_dir=%s\n' "$$lib_dir"; \
	printf 'libghostty-vt nmux version: '; \
	if ! DYLD_LIBRARY_PATH="$$lib_dir" LD_LIBRARY_PATH="$$lib_dir" target/packaging-libghostty-vt/release/nmux --version; then \
		echo "libghostty-vt nmux version check failed" >&2; \
		status=1; \
	fi; \
	printf 'libghostty-vt nmuxd version: '; \
	if ! DYLD_LIBRARY_PATH="$$lib_dir" LD_LIBRARY_PATH="$$lib_dir" target/packaging-libghostty-vt/release/nmuxd --version; then \
		echo "libghostty-vt nmuxd version check failed" >&2; \
		status=1; \
	fi; \
	if [ "$$status" -ne 0 ]; then \
		echo "one or more opt-in libghostty-vt binary version checks failed; record this as packaging evidence" >&2; \
		exit "$$status"; \
	fi

packaging-layout-sample: packaging-sample
	@echo "staging opt-in libghostty-vt package layout"
	@pkg_dir=target/packaging-libghostty-vt/package; \
	lib_path="$$(find target/packaging-libghostty-vt/release -path '*/ghostty-install/lib/libghostty-vt.*' -print -quit)"; \
	if [ -z "$$lib_path" ]; then \
		echo "missing packaged libghostty-vt runtime library directory" >&2; \
		exit 1; \
	fi; \
	lib_dir="$$(dirname "$$lib_path")"; \
	rm -rf "$$pkg_dir"; \
	mkdir -p "$$pkg_dir/bin" "$$pkg_dir/lib" "$$pkg_dir/libexec"; \
	cp target/packaging-libghostty-vt/release/nmux "$$pkg_dir/libexec/nmux"; \
	cp target/packaging-libghostty-vt/release/nmuxd "$$pkg_dir/libexec/nmuxd"; \
	cp "$$lib_dir"/libghostty-vt* "$$pkg_dir/lib/"; \
	for bin in nmux nmuxd; do \
		{ \
			printf '%s\n' '#!/bin/sh'; \
			printf '%s\n' 'set -eu'; \
			printf '%s\n' 'bin_dir=$$(CDPATH= cd "$$(dirname "$$0")" && pwd)'; \
			printf '%s\n' 'lib_dir=$$bin_dir/../lib'; \
			printf '%s\n' 'DYLD_LIBRARY_PATH=$$lib_dir$${DYLD_LIBRARY_PATH:+:$$DYLD_LIBRARY_PATH}'; \
			printf '%s\n' 'LD_LIBRARY_PATH=$$lib_dir$${LD_LIBRARY_PATH:+:$$LD_LIBRARY_PATH}'; \
			printf '%s\n' 'export DYLD_LIBRARY_PATH LD_LIBRARY_PATH'; \
			printf 'exec "$$bin_dir/../libexec/%s" "$$@"\n' "$$bin"; \
		} > "$$pkg_dir/bin/$$bin"; \
		chmod +x "$$pkg_dir/bin/$$bin"; \
	done; \
	printf 'package_layout=%s\n' "$$pkg_dir"; \
	find "$$pkg_dir" -type f | sort; \
	printf 'packaged libghostty-vt nmux version: '; \
	"$$pkg_dir/bin/nmux" --version; \
	printf 'packaged libghostty-vt nmuxd version: '; \
	"$$pkg_dir/bin/nmuxd" --version

check-schema: require-flatc
	flatc --json --strict-json --no-warnings -o /tmp $(SCHEMA)

check-toolchain: require-flatc require-cargo

check-vt-toolchain: check-toolchain require-zig require-ghostty-source

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

require-ghostty-source:
	@if [ -n "$${GHOSTTY_SOURCE_DIR:-}" ] && [ ! -d "$${GHOSTTY_SOURCE_DIR}" ]; then \
		echo "invalid GHOSTTY_SOURCE_DIR=$${GHOSTTY_SOURCE_DIR}; expected a readable Ghostty source directory or unset it to use the pinned libghostty-vt-sys fetch" >&2; \
		exit 1; \
	fi

rust-test: require-cargo
	cargo test --workspace

toolchain-info:
	@printf 'cargo=%s\n' "$$(command -v cargo >/dev/null 2>&1 && cargo --version || printf 'missing')"
	@printf 'rustc=%s\n' "$$(command -v rustc >/dev/null 2>&1 && rustc --version || printf 'missing')"
	@printf 'flatc=%s\n' "$$(command -v flatc >/dev/null 2>&1 && flatc --version || printf 'missing')"
	@printf 'zig=%s\n' "$$(command -v zig >/dev/null 2>&1 && zig version || printf 'missing')"
	@printf 'GHOSTTY_SOURCE_DIR=%s\n' "$${GHOSTTY_SOURCE_DIR:-unset}"
	@printf 'ghostty_source_mode=%s\n' "$$([ -n "$${GHOSTTY_SOURCE_DIR:-}" ] && printf 'local' || printf 'pinned-fetch')"
	@printf 'ghostty_source_dir_status=%s\n' "$$([ -z "$${GHOSTTY_SOURCE_DIR:-}" ] && printf 'unset' || { [ -d "$${GHOSTTY_SOURCE_DIR}" ] && printf 'present' || printf 'missing'; })"
	@printf 'GIT_CONFIG_GLOBAL=%s\n' "$${GIT_CONFIG_GLOBAL:-unset}"
