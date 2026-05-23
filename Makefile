SCHEMA := schema/nmux.fbs
GEN_DIR := crates/nmux-proto/src/generated
FLATC_VERSION := 25.12.19
ZIG_VERSION_PREFIX := 0.15.

.PHONY: check check-all check-ghostty-vt check-schema check-toolchain check-vt-toolchain generate-schema packaging-archive-sample packaging-layout-sample packaging-provenance-sample packaging-sample promotion-local-sample promotion-sample require-cargo require-flatc require-ghostty-source require-zig rust-test toolchain-info

check: check-toolchain check-schema rust-test

check-all: check check-ghostty-vt

check-ghostty-vt: check-vt-toolchain
	GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-core --features libghostty-vt
	GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-cli --features libghostty-vt

promotion-sample: toolchain-info
	time -p $(MAKE) check-all

promotion-local-sample:
	@echo "== promotion local sample: validation =="
	$(MAKE) promotion-sample
	@echo "== promotion local sample: packaging archive =="
	$(MAKE) packaging-archive-sample

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

packaging-provenance-sample: packaging-layout-sample
	@echo "writing opt-in libghostty-vt package provenance manifest"
	@pkg_dir=target/packaging-libghostty-vt/package; \
	manifest="$$pkg_dir/PROVENANCE.txt"; \
	tree_file="$$pkg_dir/CARGO_TREE.txt"; \
	cargo tree --locked -p nmux-cli --features libghostty-vt > "$$tree_file"; \
	hash_file() { \
		if command -v sha256sum >/dev/null 2>&1; then \
			sha256sum "$$1" | awk '{print $$1}'; \
		else \
			shasum -a 256 "$$1" | awk '{print $$1}'; \
		fi; \
	}; \
	{ \
		printf 'nmux packaging provenance sample\n'; \
		printf 'generated_at_utc=%s\n' "$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
		printf 'package_layout=%s\n' "$$pkg_dir"; \
		printf 'ghostty_source_mode=%s\n' "$$([ -n "$${GHOSTTY_SOURCE_DIR:-}" ] && printf 'local' || printf 'pinned-fetch')"; \
		printf 'GHOSTTY_SOURCE_DIR=%s\n' "$${GHOSTTY_SOURCE_DIR:-unset}"; \
		printf 'GIT_CONFIG_GLOBAL=%s\n' "$${GIT_CONFIG_GLOBAL:-unset}"; \
		printf '\n[toolchain]\n'; \
		$(MAKE) --no-print-directory toolchain-info; \
		printf '\n[cargo_lock]\n'; \
		printf 'Cargo.lock sha256=%s\n' "$$(hash_file Cargo.lock)"; \
		printf '\n[staged_files]\n'; \
		find "$$pkg_dir" -type f ! -name PROVENANCE.txt | sort | while read -r file; do \
			printf '%s bytes=%s sha256=%s\n' "$$file" "$$(wc -c < "$$file" | tr -d ' ')" "$$(hash_file "$$file")"; \
		done; \
		printf '\n[native_runtime_libraries]\n'; \
		find "$$pkg_dir/lib" -type f | sort; \
		printf '\n[dynamic_dependencies]\n'; \
		if command -v otool >/dev/null 2>&1; then \
			for bin in "$$pkg_dir/libexec/nmux" "$$pkg_dir/libexec/nmuxd"; do \
				printf '%s\n' "$$bin"; \
				otool -L "$$bin"; \
			done; \
		elif command -v ldd >/dev/null 2>&1; then \
			for bin in "$$pkg_dir/libexec/nmux" "$$pkg_dir/libexec/nmuxd"; do \
				printf '%s\n' "$$bin"; \
				ldd "$$bin"; \
			done; \
		else \
			printf 'dynamic dependency inspector unavailable\n'; \
		fi; \
		printf '\n[cargo_tree]\n'; \
		cat "$$tree_file"; \
	} > "$$manifest"; \
	printf 'provenance_manifest=%s\n' "$$manifest"

packaging-archive-sample: packaging-provenance-sample
	@echo "writing and verifying opt-in libghostty-vt package archive"
	@pkg_dir=target/packaging-libghostty-vt/package; \
	archive_dir=target/packaging-libghostty-vt/archive; \
	archive="$$archive_dir/nmux-libghostty-vt-package.tar.gz"; \
	check_dir="$$archive_dir/check"; \
	hash_file() { \
		if command -v sha256sum >/dev/null 2>&1; then \
			sha256sum "$$1" | awk '{print $$1}'; \
		else \
			shasum -a 256 "$$1" | awk '{print $$1}'; \
		fi; \
	}; \
	rm -rf "$$archive_dir"; \
	mkdir -p "$$archive_dir" "$$check_dir"; \
	tar -C "$$pkg_dir/.." -czf "$$archive" package; \
	printf '%s  %s\n' "$$(hash_file "$$archive")" "$$archive" > "$$archive.sha256"; \
	tar -C "$$check_dir" -xzf "$$archive"; \
	printf 'archive=%s\n' "$$archive"; \
	printf 'archive_sha256=%s\n' "$$(cat "$$archive.sha256")"; \
	find "$$check_dir/package" -type f | sort; \
	printf 'archived libghostty-vt nmux version: '; \
	"$$check_dir/package/bin/nmux" --version; \
	printf 'archived libghostty-vt nmuxd version: '; \
	"$$check_dir/package/bin/nmuxd" --version

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
