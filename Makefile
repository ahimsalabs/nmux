SCHEMA := schema/nmux.fbs
GEN_DIR := crates/nmux-proto/src/generated
FLATC_VERSION := 25.12.19
ZIG_VERSION_PREFIX := 0.15.
PROMOTION_EVIDENCE_DIR ?= target/promotion-evidence

.PHONY: check check-all check-ghostty-vt check-schema check-toolchain check-vt-toolchain generate-schema packaging-archive-runtime-smoke packaging-archive-sample packaging-layout-sample packaging-provenance-sample packaging-provenance-verify packaging-sample promotion-cold-target-sample promotion-evidence-bundle promotion-evidence-verify promotion-local-sample promotion-sample require-cargo require-flatc require-ghostty-source require-zig rust-test source-fetch-provenance-sample toolchain-info

check: check-toolchain check-schema rust-test

check-all: check check-ghostty-vt

check-ghostty-vt: check-vt-toolchain
	GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-core --features libghostty-vt
	GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-cli --features libghostty-vt

promotion-sample: toolchain-info
	time -p $(MAKE) check-all

promotion-cold-target-sample: toolchain-info
	@echo "clearing target/promotion-cold for cold target-dir validation"
	rm -rf target/promotion-cold
	time -p env CARGO_TARGET_DIR=target/promotion-cold $(MAKE) check-all

promotion-local-sample:
	@echo "== promotion local sample: source-fetch provenance =="
	$(MAKE) source-fetch-provenance-sample
	@echo "== promotion local sample: validation =="
	$(MAKE) promotion-sample
	@echo "== promotion local sample: packaging archive runtime smoke =="
	$(MAKE) packaging-archive-runtime-smoke

promotion-evidence-bundle:
	@echo "writing local promotion evidence bundle"
	@set -u; \
	bundle_dir="$(PROMOTION_EVIDENCE_DIR)"; \
	run_log="$$bundle_dir/RUN.log"; \
	rm -rf "$$bundle_dir"; \
	mkdir -p "$$bundle_dir"; \
	$(MAKE) --no-print-directory promotion-local-sample > "$$run_log" 2>&1; \
	status="$$?"; \
	if [ "$$status" -ne 0 ]; then \
		cat "$$run_log"; \
		echo "promotion evidence bundle failed; partial log: $$run_log" >&2; \
		exit "$$status"; \
	fi; \
	set -e; \
	cp target/source-fetch-provenance/SOURCE_FETCH.txt "$$bundle_dir/SOURCE_FETCH.txt"; \
	cp target/packaging-libghostty-vt/package/PROVENANCE.txt "$$bundle_dir/PACKAGE_PROVENANCE.txt"; \
	cp target/packaging-libghostty-vt/package/CARGO_TREE.txt "$$bundle_dir/CARGO_TREE.txt"; \
	cp target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz.sha256 "$$bundle_dir/ARCHIVE.sha256"; \
	$(MAKE) --no-print-directory toolchain-info > "$$bundle_dir/TOOLCHAIN.txt"; \
	archive_sha="$$(cat "$$bundle_dir/ARCHIVE.sha256")"; \
	runtime_smoke="$$(grep -m1 '^packaged_runtime_smoke=' "$$run_log" | cut -d= -f2- || true)"; \
	if [ -z "$$runtime_smoke" ]; then \
		echo "missing packaged_runtime_smoke result in $$run_log" >&2; \
		exit 1; \
	fi; \
	check_all_real="$$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^real [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$run_log")"; \
	check_all_user="$$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^user [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$run_log")"; \
	check_all_sys="$$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^sys [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$run_log")"; \
	if [ -z "$$check_all_real" ] || [ -z "$$check_all_user" ] || [ -z "$$check_all_sys" ]; then \
		echo "missing check-all time -p result in $$run_log" >&2; \
		exit 1; \
	fi; \
	{ \
		printf 'nmux promotion evidence bundle\n'; \
		printf 'generated_at_utc=%s\n' "$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
		printf 'host=%s\n' "$$(uname -a)"; \
		printf 'git_revision=%s\n' "$$(git rev-parse HEAD 2>/dev/null || printf 'unknown')"; \
		printf 'github_actions=%s\n' "$${GITHUB_ACTIONS:-false}"; \
		printf 'github_server_url=%s\n' "$${GITHUB_SERVER_URL:-unset}"; \
		printf 'github_repository=%s\n' "$${GITHUB_REPOSITORY:-unset}"; \
		printf 'github_run_id=%s\n' "$${GITHUB_RUN_ID:-unset}"; \
		printf 'github_run_attempt=%s\n' "$${GITHUB_RUN_ATTEMPT:-unset}"; \
		printf 'github_ref=%s\n' "$${GITHUB_REF:-unset}"; \
		printf 'github_sha=%s\n' "$${GITHUB_SHA:-unset}"; \
		printf 'runner_os=%s\n' "$${RUNNER_OS:-unset}"; \
		printf 'runner_arch=%s\n' "$${RUNNER_ARCH:-unset}"; \
		printf 'runner_name=%s\n' "$${RUNNER_NAME:-unset}"; \
		printf 'ghostty_source_mode=%s\n' "$$([ -n "$${GHOSTTY_SOURCE_DIR:-}" ] && printf 'local' || printf 'pinned-fetch')"; \
		printf 'GHOSTTY_SOURCE_DIR=%s\n' "$${GHOSTTY_SOURCE_DIR:-unset}"; \
		printf 'GIT_CONFIG_GLOBAL=%s\n' "$${GIT_CONFIG_GLOBAL:-unset}"; \
		printf 'cache_state=%s\n' 'not captured; record Nix/Cargo/native cache context separately'; \
		printf 'run_log=%s\n' "$$run_log"; \
		printf 'toolchain=%s\n' "$$bundle_dir/TOOLCHAIN.txt"; \
		printf 'source_fetch=%s\n' "$$bundle_dir/SOURCE_FETCH.txt"; \
		printf 'package_provenance=%s\n' "$$bundle_dir/PACKAGE_PROVENANCE.txt"; \
		printf 'cargo_tree=%s\n' "$$bundle_dir/CARGO_TREE.txt"; \
		printf 'check_all_real_seconds=%s\n' "$$check_all_real"; \
		printf 'check_all_user_seconds=%s\n' "$$check_all_user"; \
		printf 'check_all_sys_seconds=%s\n' "$$check_all_sys"; \
		printf 'archive_sha256=%s\n' "$$archive_sha"; \
		printf 'packaged_runtime_smoke=%s\n' "$$runtime_smoke"; \
	} > "$$bundle_dir/SUMMARY.txt"; \
	$(MAKE) --no-print-directory promotion-evidence-verify; \
	cat "$$run_log"; \
	printf 'promotion_evidence_bundle=%s\n' "$$bundle_dir"; \
	find "$$bundle_dir" -type f | sort

promotion-evidence-verify:
	@echo "verifying local promotion evidence bundle"
	@bundle_dir="$(PROMOTION_EVIDENCE_DIR)"; \
	summary="$$bundle_dir/SUMMARY.txt"; \
	run_log="$$bundle_dir/RUN.log"; \
	toolchain="$$bundle_dir/TOOLCHAIN.txt"; \
	source_fetch="$$bundle_dir/SOURCE_FETCH.txt"; \
	package_provenance="$$bundle_dir/PACKAGE_PROVENANCE.txt"; \
	cargo_tree="$$bundle_dir/CARGO_TREE.txt"; \
	archive_sha_file="$$bundle_dir/ARCHIVE.sha256"; \
	require_file() { \
		path="$$1"; \
		if [ ! -s "$$path" ]; then \
			echo "missing or empty promotion evidence artifact: $$path" >&2; \
			exit 1; \
		fi; \
	}; \
	require_line() { \
		file="$$1"; \
		pattern="$$2"; \
		description="$$3"; \
		if ! grep -Eq "$$pattern" "$$file"; then \
			echo "missing promotion evidence record in $$file: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
	require_exact() { \
		file="$$1"; \
		line="$$2"; \
		description="$$3"; \
		if ! grep -Fxq "$$line" "$$file"; then \
			echo "missing promotion evidence record in $$file: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
	require_file "$$summary"; \
	require_file "$$run_log"; \
	require_file "$$toolchain"; \
	require_file "$$source_fetch"; \
	require_file "$$package_provenance"; \
	require_file "$$cargo_tree"; \
	require_file "$$archive_sha_file"; \
	require_exact "$$summary" 'nmux promotion evidence bundle' 'summary title'; \
	require_line "$$summary" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'generation timestamp'; \
	require_line "$$summary" '^host=.+$$' 'host identity'; \
	require_line "$$summary" '^git_revision=(unknown|[0-9a-f]{40})$$' 'git revision'; \
	require_line "$$summary" '^github_actions=(true|false)$$' 'GitHub Actions flag'; \
	require_line "$$summary" '^github_server_url=.+$$' 'GitHub server URL field'; \
	require_line "$$summary" '^github_repository=.+$$' 'GitHub repository field'; \
	require_line "$$summary" '^github_run_id=.+$$' 'GitHub run ID field'; \
	require_line "$$summary" '^github_run_attempt=.+$$' 'GitHub run attempt field'; \
	require_line "$$summary" '^github_ref=.+$$' 'GitHub ref field'; \
	require_line "$$summary" '^github_sha=.+$$' 'GitHub SHA field'; \
	require_line "$$summary" '^runner_os=.+$$' 'runner OS field'; \
	require_line "$$summary" '^runner_arch=.+$$' 'runner architecture field'; \
	require_line "$$summary" '^runner_name=.+$$' 'runner name field'; \
	require_line "$$summary" '^ghostty_source_mode=(pinned-fetch|local)$$' 'Ghostty source mode'; \
	require_line "$$summary" '^GHOSTTY_SOURCE_DIR=.+$$' 'GHOSTTY_SOURCE_DIR field'; \
	require_line "$$summary" '^GIT_CONFIG_GLOBAL=.+$$' 'GIT_CONFIG_GLOBAL field'; \
	require_line "$$summary" '^cache_state=.+$$' 'cache state field'; \
	require_exact "$$summary" "run_log=$$run_log" 'run log path'; \
	require_exact "$$summary" "toolchain=$$toolchain" 'toolchain path'; \
	require_exact "$$summary" "source_fetch=$$source_fetch" 'source-fetch path'; \
	require_exact "$$summary" "package_provenance=$$package_provenance" 'package provenance path'; \
	require_exact "$$summary" "cargo_tree=$$cargo_tree" 'cargo tree path'; \
	require_line "$$summary" '^check_all_real_seconds=[0-9]+([.][0-9]+)?$$' 'check-all real timing'; \
	require_line "$$summary" '^check_all_user_seconds=[0-9]+([.][0-9]+)?$$' 'check-all user timing'; \
	require_line "$$summary" '^check_all_sys_seconds=[0-9]+([.][0-9]+)?$$' 'check-all sys timing'; \
	check_all_real="$$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^real [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$run_log")"; \
	check_all_user="$$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^user [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$run_log")"; \
	check_all_sys="$$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^sys [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$run_log")"; \
	if [ -z "$$check_all_real" ] || [ -z "$$check_all_user" ] || [ -z "$$check_all_sys" ]; then \
		echo "missing check-all time -p result in $$run_log" >&2; \
		exit 1; \
	fi; \
	require_exact "$$summary" "check_all_real_seconds=$$check_all_real" 'check-all real timing matches run log'; \
	require_exact "$$summary" "check_all_user_seconds=$$check_all_user" 'check-all user timing matches run log'; \
	require_exact "$$summary" "check_all_sys_seconds=$$check_all_sys" 'check-all sys timing matches run log'; \
	archive_sha="$$(cat "$$archive_sha_file")"; \
	require_exact "$$summary" "archive_sha256=$$archive_sha" 'archive SHA-256'; \
	require_exact "$$summary" 'packaged_runtime_smoke=passed' 'packaged runtime smoke'; \
	require_line "$$toolchain" '^cargo=cargo ' 'cargo version'; \
	require_line "$$toolchain" '^rustc=rustc ' 'rustc version'; \
	require_line "$$toolchain" '^flatc=flatc version 25\.12\.19$$' 'flatc version'; \
	require_line "$$toolchain" '^zig=0\.15\.' 'Zig version'; \
	require_line "$$source_fetch" '^Cargo\.lock sha256=[0-9a-f]{64}$$' 'source-fetch Cargo.lock hash'; \
	require_line "$$source_fetch" '^\[cargo_lock:libghostty-vt\]$$' 'source-fetch libghostty-vt record'; \
	require_line "$$source_fetch" '^name = "libghostty-vt"$$' 'source-fetch libghostty-vt package name'; \
	require_line "$$source_fetch" '^checksum = "[0-9a-f]{64}"$$' 'source-fetch package checksum'; \
	require_line "$$source_fetch" '^\[cargo_lock:libghostty-vt-sys\]$$' 'source-fetch libghostty-vt-sys record'; \
	require_line "$$source_fetch" '^name = "libghostty-vt-sys"$$' 'source-fetch libghostty-vt-sys package name'; \
	require_line "$$package_provenance" '^\[staged_files\]$$' 'packaging staged file hashes'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/bin/nmux bytes=[0-9]+ sha256=[0-9a-f]{64}$$' 'packaged nmux wrapper hash'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/bin/nmuxd bytes=[0-9]+ sha256=[0-9a-f]{64}$$' 'packaged nmuxd wrapper hash'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/libexec/nmux bytes=[0-9]+ sha256=[0-9a-f]{64}$$' 'packaged nmux binary hash'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/libexec/nmuxd bytes=[0-9]+ sha256=[0-9a-f]{64}$$' 'packaged nmuxd binary hash'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/lib/libghostty-vt.* bytes=[0-9]+ sha256=[0-9a-f]{64}$$' 'packaged native runtime library hash'; \
	require_line "$$package_provenance" '^\[native_runtime_libraries\]$$' 'packaging runtime libraries'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/lib/libghostty-vt' 'packaging runtime library path'; \
	require_line "$$package_provenance" '^\[dynamic_dependencies\]$$' 'packaging dynamic dependencies'; \
	require_line "$$run_log" '^provenance_manifest_verified=target/packaging-libghostty-vt/package/PROVENANCE\.txt$$' 'package provenance verifier result'; \
	require_line "$$run_log" '^packaged_runtime_smoke=passed$$' 'runtime smoke result'; \
	require_line "$$cargo_tree" '^nmux-cli v' 'cargo tree root'; \
	require_line "$$archive_sha_file" '^[0-9a-f]{64}  target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package\.tar\.gz$$' 'archive SHA-256 file'; \
	printf 'promotion_evidence_verified=%s\n' "$$summary"

source-fetch-provenance-sample: toolchain-info
	@echo "writing source-fetch provenance report"
	@report_dir=target/source-fetch-provenance; \
	report="$$report_dir/SOURCE_FETCH.txt"; \
	hash_file() { \
		if command -v sha256sum >/dev/null 2>&1; then \
			sha256sum "$$1" | awk '{print $$1}'; \
		else \
			shasum -a 256 "$$1" | awk '{print $$1}'; \
		fi; \
	}; \
	cargo_lock_record() { \
		awk -v package="$$1" '\
			$$0 == "[[package]]" { block = $$0 ORS; in_block = 1; name = ""; next } \
			in_block { block = block $$0 ORS } \
			in_block && $$1 == "name" && $$3 == "\"" package "\"" { name = package } \
			in_block && $$0 == "" { if (name == package) { printf "%s", block; found = 1 } in_block = 0; block = ""; name = "" } \
			END { if (in_block && name == package) { printf "%s", block; found = 1 } if (!found) { exit 1 } }' Cargo.lock; \
	}; \
	rm -rf "$$report_dir"; \
	mkdir -p "$$report_dir"; \
	{ \
		printf 'nmux source-fetch provenance sample\n'; \
		printf 'generated_at_utc=%s\n' "$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
		printf 'ghostty_source_mode=%s\n' "$$([ -n "$${GHOSTTY_SOURCE_DIR:-}" ] && printf 'local' || printf 'pinned-fetch')"; \
		printf 'GHOSTTY_SOURCE_DIR=%s\n' "$${GHOSTTY_SOURCE_DIR:-unset}"; \
		printf 'GIT_CONFIG_GLOBAL=%s\n' "$${GIT_CONFIG_GLOBAL:-unset}"; \
		printf 'Cargo.lock sha256=%s\n' "$$(hash_file Cargo.lock)"; \
		printf '\n[toolchain]\n'; \
		$(MAKE) --no-print-directory toolchain-info; \
		printf '\n[cargo_lock:libghostty-vt]\n'; \
		cargo_lock_record libghostty-vt; \
		printf '\n[cargo_lock:libghostty-vt-sys]\n'; \
		cargo_lock_record libghostty-vt-sys; \
		printf '\n[policy_note]\n'; \
		printf '%s\n' 'This report records local source-fetch inputs for evidence. It does not choose the default or packaged-build source policy.'; \
	} > "$$report"; \
	printf 'source_fetch_provenance=%s\n' "$$report"

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
	cargo_lock_record() { \
		awk -v package="$$1" '\
			$$0 == "[[package]]" { block = $$0 ORS; in_block = 1; name = ""; next } \
			in_block { block = block $$0 ORS } \
			in_block && $$1 == "name" && $$3 == "\"" package "\"" { name = package } \
			in_block && $$0 == "" { if (name == package) { printf "%s", block; found = 1 } in_block = 0; block = ""; name = "" } \
			END { if (in_block && name == package) { printf "%s", block; found = 1 } if (!found) { exit 1 } }' Cargo.lock; \
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
		printf '\n[cargo_lock:libghostty-vt]\n'; \
		cargo_lock_record libghostty-vt; \
		printf '\n[cargo_lock:libghostty-vt-sys]\n'; \
		cargo_lock_record libghostty-vt-sys; \
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

packaging-provenance-verify: packaging-provenance-sample
	@echo "verifying opt-in libghostty-vt package provenance manifest"
	@manifest=target/packaging-libghostty-vt/package/PROVENANCE.txt; \
	pkg_dir=target/packaging-libghostty-vt/package; \
	require_line() { \
		pattern="$$1"; \
		description="$$2"; \
		if ! grep -Eq "$$pattern" "$$manifest"; then \
			echo "missing provenance record: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
	require_file_record() { \
		path="$$1"; \
		if ! grep -Eq "^$$path bytes=[0-9]+ sha256=[0-9a-f]{64}$$" "$$manifest"; then \
			echo "missing staged file hash record: $$path" >&2; \
			exit 1; \
		fi; \
	}; \
	cargo_lock_record() { \
		awk -v package="$$1" '\
			$$0 == "[[package]]" { block = $$0 ORS; in_block = 1; name = ""; next } \
			in_block { block = block $$0 ORS } \
			in_block && $$1 == "name" && $$3 == "\"" package "\"" { name = package } \
			in_block && $$0 == "" { if (name == package) { printf "%s", block; found = 1 } in_block = 0; block = ""; name = "" } \
			END { if (in_block && name == package) { printf "%s", block; found = 1 } if (!found) { exit 1 } }' Cargo.lock; \
	}; \
	require_cargo_lock_record() { \
		package="$$1"; \
		expected_file="$$(mktemp)"; \
		section_file="$$(mktemp)"; \
		if ! cargo_lock_record "$$package" > "$$expected_file"; then \
			echo "missing Cargo.lock record for $$package" >&2; \
			rm -f "$$expected_file" "$$section_file"; \
			exit 1; \
		fi; \
		awk -v header="[cargo_lock:$$package]" '\
			$$0 == header { in_section = 1; next } \
			in_section && $$0 ~ /^\[[^[]/ { exit } \
			in_section { print }' "$$manifest" > "$$section_file"; \
		status=0; \
		while IFS= read -r line; do \
			if [ -n "$$line" ] && ! grep -Fxq "$$line" "$$section_file"; then \
				echo "missing Cargo.lock record line for $$package: $$line" >&2; \
				exit 1; \
			fi; \
		done < "$$expected_file" || status="$$?"; \
		rm -f "$$expected_file" "$$section_file"; \
		if [ "$$status" -ne 0 ]; then exit "$$status"; fi; \
	}; \
	test -s "$$manifest" || { echo "missing provenance manifest: $$manifest" >&2; exit 1; }; \
	require_line '^ghostty_source_mode=(pinned-fetch|local)$$' 'ghostty source mode'; \
	require_line '^GHOSTTY_SOURCE_DIR=' 'GHOSTTY_SOURCE_DIR'; \
	require_line '^GIT_CONFIG_GLOBAL=' 'GIT_CONFIG_GLOBAL'; \
	require_line '^\[toolchain\]$$' 'toolchain section'; \
	require_line '^cargo=cargo ' 'cargo version'; \
	require_line '^rustc=rustc ' 'rustc version'; \
	require_line '^flatc=flatc version 25\.12\.19$$' 'flatc version'; \
	require_line '^zig=0\.15\.' 'Zig 0.15 version'; \
	require_line '^\[cargo_lock\]$$' 'Cargo.lock section'; \
	require_line '^Cargo\.lock sha256=[0-9a-f]{64}$$' 'Cargo.lock hash'; \
	require_line '^\[cargo_lock:libghostty-vt\]$$' 'locked libghostty-vt package section'; \
	require_cargo_lock_record libghostty-vt; \
	require_line '^\[cargo_lock:libghostty-vt-sys\]$$' 'locked libghostty-vt-sys package section'; \
	require_cargo_lock_record libghostty-vt-sys; \
	require_line '^\[staged_files\]$$' 'staged file section'; \
	require_file_record "$$pkg_dir/bin/nmux"; \
	require_file_record "$$pkg_dir/bin/nmuxd"; \
	require_file_record "$$pkg_dir/libexec/nmux"; \
	require_file_record "$$pkg_dir/libexec/nmuxd"; \
	require_line "^$$pkg_dir/lib/libghostty-vt.* bytes=[0-9]+ sha256=[0-9a-f]{64}$$" 'libghostty-vt runtime library hash'; \
	require_line '^\[native_runtime_libraries\]$$' 'native runtime library section'; \
	require_line "^$$pkg_dir/lib/libghostty-vt" 'native runtime library path'; \
	require_line '^\[dynamic_dependencies\]$$' 'dynamic dependencies section'; \
	require_line "^$$pkg_dir/libexec/nmux$$" 'nmux dynamic dependency heading'; \
	require_line "^$$pkg_dir/libexec/nmuxd$$" 'nmuxd dynamic dependency heading'; \
	require_line '^\[cargo_tree\]$$' 'cargo tree section'; \
	require_line '^nmux-cli v' 'nmux-cli cargo tree root'; \
	printf 'provenance_manifest_verified=%s\n' "$$manifest"

packaging-archive-sample: packaging-provenance-verify
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

packaging-archive-runtime-smoke: packaging-archive-sample
	@echo "running packaged opt-in libghostty-vt archive runtime smoke"
	@pkg_dir=target/packaging-libghostty-vt/archive/check/package; \
	work_dir="$$(mktemp -d "/tmp/nmuxpkg.XXXXXX")"; \
	socket="$$work_dir/nmuxd.sock"; \
	client_out="$$work_dir/client.out"; \
	client_err="$$work_dir/client.err"; \
	daemon_out="$$work_dir/daemon.out"; \
	daemon_err="$$work_dir/daemon.err"; \
	daemon_pid=""; \
	trap 'status=$$?; if [ -n "$${daemon_pid:-}" ] && kill -0 "$$daemon_pid" >/dev/null 2>&1; then kill "$$daemon_pid" >/dev/null 2>&1 || true; wait "$$daemon_pid" >/dev/null 2>&1 || true; fi; rm -rf "$$work_dir"; exit "$$status"' EXIT INT TERM; \
	"$$pkg_dir/bin/nmuxd" --socket "$$socket" --one-shot --terminal-engine libghostty-vt --command "printf 'packaged-runtime-smoke\n'; cat >/dev/null" >"$$daemon_out" 2>"$$daemon_err" & \
	daemon_pid="$$!"; \
	if ! "$$pkg_dir/bin/nmux" --socket "$$socket" --connect-timeout-ms 5000 --no-input --scrollback-start 1 --scrollback-count 5 >"$$client_out" 2>"$$client_err"; then \
		echo "packaged runtime smoke client failed" >&2; \
		cat "$$client_err" >&2; \
		exit 1; \
	fi; \
	if ! wait "$$daemon_pid"; then \
		daemon_pid=""; \
		echo "packaged runtime smoke daemon failed" >&2; \
		cat "$$daemon_err" >&2; \
		exit 1; \
	fi; \
	daemon_pid=""; \
	if ! grep -q 'packaged-runtime-smoke' "$$client_out"; then \
		echo "packaged runtime smoke output missing sentinel" >&2; \
		cat "$$client_out" >&2; \
		cat "$$client_err" >&2; \
		cat "$$daemon_err" >&2; \
		exit 1; \
	fi; \
	printf 'packaged_runtime_smoke=passed\n'; \
	printf 'packaged_runtime_smoke_output=%s\n' "$$(grep -m1 'packaged-runtime-smoke' "$$client_out")"

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
