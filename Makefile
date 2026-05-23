SCHEMA := schema/nmux.fbs
GEN_DIR := crates/nmux-proto/src/generated
FLATC_VERSION := 25.12.19
ZIG_VERSION_PREFIX := 0.15.
PROMOTION_EVIDENCE_DIR ?= target/promotion-evidence
SOURCE_FETCH_REPORT ?= target/source-fetch-provenance/SOURCE_FETCH.txt
SOURCE_FETCH_OFFLINE_PROBE_REPORT ?= target/source-fetch-offline/OFFLINE_PROBE.txt
SOURCE_FETCH_OFFLINE_PROBE_LOG ?=
PACKAGING_LAYOUT ?= target/packaging-libghostty-vt/package
PACKAGING_PROVENANCE_MANIFEST ?= target/packaging-libghostty-vt/package/PROVENANCE.txt
PACKAGING_ARCHIVE ?= target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz
PACKAGING_ARCHIVE_SHA256 ?= $(PACKAGING_ARCHIVE).sha256

.PHONY: check check-all check-ghostty-vt check-schema check-toolchain check-vt-toolchain generate-schema local-smoke packaging-archive-runtime-smoke packaging-archive-sample packaging-archive-verify packaging-layout-sample packaging-layout-verify packaging-provenance-manifest-verify packaging-provenance-sample packaging-provenance-verify packaging-sample promotion-cold-deps-sample promotion-cold-deps-verify promotion-cold-target-sample promotion-evidence-bundle promotion-evidence-verify promotion-local-sample promotion-sample renderer-equivalence-smoke require-cargo require-flatc require-ghostty-source require-zig rust-test source-audit source-fetch-offline-probe source-fetch-offline-probe-verify source-fetch-provenance-sample source-fetch-provenance-verify toolchain-info

check: check-toolchain check-schema rust-test

check-all: check check-ghostty-vt

source-audit:
	@system="$$(nix eval --impure --raw --expr builtins.currentSystem)"; \
	out="$$(nix build ".#checks.$$system.source-audit" --no-link --print-out-paths)"; \
	cat "$$out/source-audit.txt"

local-smoke: check-toolchain
	@echo "running local nmux daemon/client smoke"
	@tmp_dir="$$(mktemp -d "$${TMPDIR:-/tmp}/nmux-local-smoke.XXXXXX")"; \
	repo_dir="$$(pwd)"; \
	cargo_target_dir="$${CARGO_TARGET_DIR:-target}"; \
	case "$$cargo_target_dir" in \
		/*) nmux_bin="$$cargo_target_dir/debug/nmux" ;; \
		*) nmux_bin="$$repo_dir/$$cargo_target_dir/debug/nmux" ;; \
	esac; \
	socket="$$tmp_dir/nmux.sock"; \
	state="$$tmp_dir/state.json"; \
	daemon_out="$$tmp_dir/daemon.out"; \
	daemon_err="$$tmp_dir/daemon.err"; \
	client1_out="$$tmp_dir/client1.out"; \
	client1_err="$$tmp_dir/client1.err"; \
	client2_out="$$tmp_dir/client2.out"; \
	client2_err="$$tmp_dir/client2.err"; \
	client3_out="$$tmp_dir/client3.out"; \
	client3_err="$$tmp_dir/client3.err"; \
	client4_out="$$tmp_dir/client4.out"; \
	client4_err="$$tmp_dir/client4.err"; \
	client5_out="$$tmp_dir/client5.out"; \
	client5_err="$$tmp_dir/client5.err"; \
	client6_out="$$tmp_dir/client6.out"; \
	client6_err="$$tmp_dir/client6.err"; \
	info_nmux_version_json="$$tmp_dir/info-nmux-version.json"; \
	info_nmuxd_version_json="$$tmp_dir/info-nmuxd-version.json"; \
	info_nmux_socket_json="$$tmp_dir/info-nmux-socket.json"; \
	info_nmuxd_socket_json="$$tmp_dir/info-nmuxd-socket.json"; \
	cleanup() { \
		status="$$?"; \
		if [ -n "$${daemon_pid:-}" ] && kill -0 "$$daemon_pid" >/dev/null 2>&1; then \
			kill "$$daemon_pid" >/dev/null 2>&1 || true; \
			wait "$$daemon_pid" >/dev/null 2>&1 || true; \
		fi; \
		rm -rf "$$tmp_dir"; \
		exit "$$status"; \
	}; \
	trap cleanup EXIT INT TERM; \
	cargo run --quiet --bin nmux -- --version-json >"$$info_nmux_version_json"; \
	cargo run --quiet --bin nmuxd -- --version-json >"$$info_nmuxd_version_json"; \
	cargo run --quiet --bin nmux -- --socket "$$socket" --print-socket-json >"$$info_nmux_socket_json"; \
	cargo run --quiet --bin nmuxd -- --socket "$$socket" --print-socket-json >"$$info_nmuxd_socket_json"; \
	command_text="printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$$line\"; done"; \
	cargo run --quiet --bin nmuxd -- --socket "$$socket" --ready-json --live-clients 2 --command "$$command_text" >"$$daemon_out" 2>"$$daemon_err" & \
	daemon_pid="$$!"; \
	if ! printf 'ping\n' | cargo run --quiet --bin nmux -- --socket "$$socket" --connect-timeout-ms 5000 --state "$$state" --live --iterations 1 --stdin --scrollback-start 1 --scrollback-count 8 >"$$client1_out" 2>"$$client1_err"; then \
		cat "$$daemon_err" "$$client1_err" >&2; \
		exit 1; \
	fi; \
	if ! cargo run --quiet --bin nmux -- --socket "$$socket" --connect-timeout-ms 5000 --state "$$state" --live --no-input --iterations 1 --scrollback-start 1 --scrollback-count 8 >"$$client2_out" 2>"$$client2_err"; then \
		cat "$$daemon_err" "$$client2_err" >&2; \
		exit 1; \
	fi; \
	if ! wait "$$daemon_pid"; then \
		daemon_pid=""; \
		cat "$$daemon_err" >&2; \
		exit 1; \
	fi; \
	daemon_pid=""; \
	if ! grep -Fq '"event":"ready"' "$$daemon_out" || ! grep -Fq "\"NMUX_SOCKET\":\"$$socket\"" "$$daemon_out" || ! grep -Fq '"mode":"live-clients"' "$$daemon_out"; then \
		echo "missing local smoke ready-json output" >&2; \
		cat "$$daemon_out" "$$daemon_err" >&2; \
		exit 1; \
	fi; \
	: >"$$daemon_out"; \
	: >"$$daemon_err"; \
	command_text="printf 'fresh daemon\n'; sleep 1"; \
	cargo run --quiet --bin nmuxd -- --socket "$$socket" --live-clients 1 --command "$$command_text" >"$$daemon_out" 2>"$$daemon_err" & \
	daemon_pid="$$!"; \
	if ! cargo run --quiet --bin nmux -- --socket "$$socket" --connect-timeout-ms 5000 --state "$$state" --live --no-input --iterations 1 --scrollback-start 1 --scrollback-count 8 >"$$client3_out" 2>"$$client3_err"; then \
		cat "$$daemon_err" "$$client3_err" >&2; \
		exit 1; \
	fi; \
	if ! wait "$$daemon_pid"; then \
		daemon_pid=""; \
		cat "$$daemon_err" >&2; \
		exit 1; \
	fi; \
	daemon_pid=""; \
	: >"$$daemon_out"; \
	: >"$$daemon_err"; \
	command_text="\"$$nmux_bin\" --print-context; cat >/dev/null"; \
	cargo run --quiet --bin nmuxd -- --socket "$$socket" --one-shot --command "$$command_text" >"$$daemon_out" 2>"$$daemon_err" & \
	daemon_pid="$$!"; \
	if ! cargo run --quiet --bin nmux -- --socket "$$socket" --connect-timeout-ms 5000 --scrollback-start 1 --scrollback-count 24 >"$$client4_out" 2>"$$client4_err"; then \
		cat "$$daemon_err" "$$client4_err" >&2; \
		exit 1; \
	fi; \
	if ! wait "$$daemon_pid"; then \
		daemon_pid=""; \
		cat "$$daemon_err" >&2; \
		exit 1; \
	fi; \
	daemon_pid=""; \
	: >"$$daemon_out"; \
	: >"$$daemon_err"; \
	command_text="\"$$nmux_bin\" --print-context-json; cat >/dev/null"; \
	cargo run --quiet --bin nmuxd -- --socket "$$socket" --one-shot --command "$$command_text" >"$$daemon_out" 2>"$$daemon_err" & \
	daemon_pid="$$!"; \
	if ! cargo run --quiet --bin nmux -- --socket "$$socket" --connect-timeout-ms 5000 --scrollback-start 1 --scrollback-count 24 >"$$client5_out" 2>"$$client5_err"; then \
		cat "$$daemon_err" "$$client5_err" >&2; \
		exit 1; \
	fi; \
	if ! wait "$$daemon_pid"; then \
		daemon_pid=""; \
		cat "$$daemon_err" >&2; \
		exit 1; \
	fi; \
	daemon_pid=""; \
	if ! cargo run --quiet --bin nmux -- --start --json --startup-timeout-ms 5000 --command "printf 'managed-json-ready\n'; cat >/dev/null" >"$$client6_out" 2>"$$client6_err"; then \
		cat "$$client6_err" >&2; \
		exit 1; \
	fi; \
	require_output() { \
		file="$$1"; \
		text="$$2"; \
		description="$$3"; \
		if ! grep -Fq "$$text" "$$file"; then \
			echo "missing local smoke output: $$description" >&2; \
			cat "$$daemon_err" "$$client1_err" "$$client2_err" "$$client3_err" "$$client4_err" "$$client5_err" "$$client6_err" >&2; \
			echo "--- $$file ---" >&2; \
			cat "$$file" >&2; \
			exit 1; \
		fi; \
	}; \
	reject_output() { \
		file="$$1"; \
		text="$$2"; \
		description="$$3"; \
		if grep -Fq "$$text" "$$file"; then \
			echo "unexpected local smoke output: $$description" >&2; \
			cat "$$daemon_err" "$$client1_err" "$$client2_err" "$$client3_err" "$$client4_err" "$$client5_err" "$$client6_err" >&2; \
			echo "--- $$file ---" >&2; \
			cat "$$file" >&2; \
			exit 1; \
		fi; \
	}; \
	require_output "$$client1_out" 'ready' 'initial daemon output'; \
	require_output "$$client1_out" 'echo:ping' 'read-write live input response'; \
	require_output "$$client2_out" 'echo:ping' 'sequential read-only reattach sees prior output'; \
	require_output "$$client3_out" 'fresh daemon' 'recreated socket path forces fresh daemon surface'; \
	reject_output "$$client3_out" 'echo:ping' 'stale cached surface after socket recreation'; \
	require_output "$$client4_out" 'NMUX=1' 'nested print-context nmux flag'; \
	require_output "$$client4_out" 'NMUX_SESSION_ID=local' 'nested print-context session id'; \
	require_output "$$client4_out" 'NMUX_PANE_ID=pane-1' 'nested print-context pane id'; \
	require_output "$$client4_out" "NMUX_SOCKET=$$socket" 'nested print-context socket path'; \
	require_output "$$client4_out" 'NMUX_ORIGIN=local' 'nested print-context origin'; \
	require_output "$$client5_out" '"NMUX":"1"' 'nested print-context-json nmux flag'; \
	require_output "$$client5_out" '"NMUX_SESSION_ID":"local"' 'nested print-context-json session id'; \
	require_output "$$client5_out" '"NMUX_PANE_ID":"pane-1"' 'nested print-context-json pane id'; \
	require_output "$$client5_out" "\"NMUX_SOCKET\":\"$$socket\"" 'nested print-context-json socket path'; \
	require_output "$$client5_out" '"NMUX_ORIGIN":"local"' 'nested print-context-json origin'; \
	require_output "$$client6_out" '{"workspace":{' 'managed start JSON workspace'; \
	require_output "$$client6_out" 'managed-json-ready' 'managed start JSON command output'; \
	require_output "$$info_nmux_version_json" '"binary":"nmux"' 'nmux version-json binary'; \
	require_output "$$info_nmux_version_json" '"version":"' 'nmux version-json version'; \
	require_output "$$info_nmuxd_version_json" '"binary":"nmuxd"' 'nmuxd version-json binary'; \
	require_output "$$info_nmuxd_version_json" '"version":"' 'nmuxd version-json version'; \
	require_output "$$info_nmux_socket_json" "\"NMUX_SOCKET\":\"$$socket\"" 'nmux print-socket-json socket path'; \
	require_output "$$info_nmux_socket_json" '"source":"--socket"' 'nmux print-socket-json source'; \
	require_output "$$info_nmuxd_socket_json" "\"NMUX_SOCKET\":\"$$socket\"" 'nmuxd print-socket-json socket path'; \
	require_output "$$info_nmuxd_socket_json" '"source":"--socket"' 'nmuxd print-socket-json source'; \
	test -s "$$state" || { echo "missing local smoke state file: $$state" >&2; exit 1; }; \
	printf 'local_smoke_reattach=passed\n'; \
	printf 'local_smoke_socket_recreation=passed\n'; \
	printf 'local_smoke_print_context=passed\n'; \
	printf 'local_smoke_json_info=passed\n'; \
	printf 'local_smoke_ready_json=passed\n'; \
	printf 'local_smoke_managed_start=passed\n'; \
	printf 'local_smoke=passed\n'

check-ghostty-vt: check-vt-toolchain
	RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-core --features libghostty-vt
	RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-cli --features libghostty-vt

renderer-equivalence-smoke: check-vt-toolchain
	RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-core --features libghostty-vt renderer_equivalence
	RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-cli --features libghostty-vt --test renderer_equivalence

promotion-sample: toolchain-info
	time -p $(MAKE) check-all

promotion-cold-target-sample: toolchain-info
	@echo "clearing target/promotion-cold for cold target-dir validation"
	rm -rf target/promotion-cold
	time -p env CARGO_TARGET_DIR=target/promotion-cold $(MAKE) check-all

promotion-cold-deps-sample: toolchain-info
	@echo "clearing target/promotion-cold-deps for isolated Cargo dependency/source-fetch validation"
	@set -u; \
	report_dir="target/promotion-cold-deps"; \
	report="$$report_dir/REPORT.txt"; \
	log="$$report_dir/RUN.log"; \
	rm -rf "$$report_dir"; \
	mkdir -p "$$report_dir"; \
	report_dir_abs="$$(cd "$$report_dir" && pwd)"; \
	cargo_home="$$report_dir_abs/cargo-home"; \
	cargo_target_dir="$$report_dir_abs/target"; \
	start_epoch="$$(date -u '+%s')"; \
	start_utc="$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
	{ \
		printf 'nmux isolated Cargo dependency/source-fetch sample\n'; \
		printf 'started_at_utc=%s\n' "$$start_utc"; \
		printf 'sample_scope=%s\n' 'isolated repo-owned CARGO_HOME and CARGO_TARGET_DIR; Nix store, source checkout, and network state may still be warm'; \
		printf 'CARGO_HOME=%s\n' "$$cargo_home"; \
		printf 'CARGO_TARGET_DIR=%s\n' "$$cargo_target_dir"; \
		printf 'GHOSTTY_SOURCE_DIR=unset\n'; \
		printf 'GIT_CONFIG_GLOBAL=/dev/null\n'; \
		printf 'command=make check-all\n'; \
	} > "$$report"; \
	if env -u GHOSTTY_SOURCE_DIR CARGO_HOME="$$cargo_home" CARGO_TARGET_DIR="$$cargo_target_dir" GIT_CONFIG_GLOBAL=/dev/null time -p $(MAKE) check-all > "$$log" 2>&1; then \
		result=passed; \
	else \
		status="$$?"; \
		result=failed; \
	fi; \
	completed_utc="$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
	end_epoch="$$(date -u '+%s')"; \
	real_seconds="$$(awk '/^real [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$log")"; \
	user_seconds="$$(awk '/^user [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$log")"; \
	sys_seconds="$$(awk '/^sys [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$log")"; \
	{ \
		printf 'completed_at_utc=%s\n' "$$completed_utc"; \
		printf 'elapsed_seconds=%s\n' "$$((end_epoch - start_epoch))"; \
		printf 'check_all_real_seconds=%s\n' "$$real_seconds"; \
		printf 'check_all_user_seconds=%s\n' "$$user_seconds"; \
		printf 'check_all_sys_seconds=%s\n' "$$sys_seconds"; \
		printf 'result=%s\n' "$$result"; \
		printf 'log=%s\n' "$$log"; \
	} >> "$$report"; \
	cat "$$report"; \
	if [ "$$result" != passed ]; then \
		cat "$$log"; \
		exit "$$status"; \
	fi; \
	$(MAKE) --no-print-directory promotion-cold-deps-verify; \
	printf 'promotion_cold_deps_sample=%s\n' "$$report"

promotion-cold-deps-verify:
	@echo "verifying isolated Cargo dependency/source-fetch sample"
	@report_dir="target/promotion-cold-deps"; \
	report="$$report_dir/REPORT.txt"; \
	require_file() { \
		path="$$1"; \
		if [ ! -s "$$path" ]; then \
			echo "missing or empty cold-deps artifact: $$path" >&2; \
			exit 1; \
		fi; \
	}; \
	require_line() { \
		file="$$1"; \
		pattern="$$2"; \
		description="$$3"; \
		if ! grep -Eq "$$pattern" "$$file"; then \
			echo "missing cold-deps record in $$file: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
	require_exact() { \
		file="$$1"; \
		line="$$2"; \
		description="$$3"; \
		if ! grep -Fxq "$$line" "$$file"; then \
			echo "missing cold-deps record in $$file: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
	require_file "$$report"; \
	log="$$(awk -F= '/^log=/{print $$2; exit}' "$$report")"; \
	require_file "$$log"; \
	require_exact "$$report" 'nmux isolated Cargo dependency/source-fetch sample' 'report title'; \
	require_line "$$report" '^started_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'start timestamp'; \
	require_line "$$report" '^completed_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'completion timestamp'; \
	require_exact "$$report" 'sample_scope=isolated repo-owned CARGO_HOME and CARGO_TARGET_DIR; Nix store, source checkout, and network state may still be warm' 'sample scope'; \
	require_line "$$report" '^CARGO_HOME=.*/target/promotion-cold-deps/cargo-home$$' 'isolated Cargo home'; \
	require_line "$$report" '^CARGO_TARGET_DIR=.*/target/promotion-cold-deps/target$$' 'isolated target dir'; \
	require_exact "$$report" 'GHOSTTY_SOURCE_DIR=unset' 'Ghostty source env'; \
	require_exact "$$report" 'GIT_CONFIG_GLOBAL=/dev/null' 'Git config isolation'; \
	require_exact "$$report" 'command=make check-all' 'sample command'; \
	require_line "$$report" '^elapsed_seconds=[0-9]+$$' 'elapsed seconds'; \
	require_line "$$report" '^check_all_real_seconds=[0-9]+([.][0-9]+)?$$' 'real timing'; \
	require_line "$$report" '^check_all_user_seconds=[0-9]+([.][0-9]+)?$$' 'user timing'; \
	require_line "$$report" '^check_all_sys_seconds=[0-9]+([.][0-9]+)?$$' 'sys timing'; \
	require_exact "$$report" 'result=passed' 'sample result'; \
	require_line "$$report" '^log=target/promotion-cold-deps/RUN\.log$$' 'run log path'; \
	log_real="$$(awk '/^real [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$log")"; \
	log_user="$$(awk '/^user [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$log")"; \
	log_sys="$$(awk '/^sys [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$log")"; \
	if [ -z "$$log_real" ] || [ -z "$$log_user" ] || [ -z "$$log_sys" ]; then \
		echo "missing time -p result in $$log" >&2; \
		exit 1; \
	fi; \
	require_exact "$$report" "check_all_real_seconds=$$log_real" 'real timing matches log'; \
	require_exact "$$report" "check_all_user_seconds=$$log_user" 'user timing matches log'; \
	require_exact "$$report" "check_all_sys_seconds=$$log_sys" 'sys timing matches log'; \
	require_line "$$log" '^flatc --json --strict-json --no-warnings -o /tmp schema/nmux\.fbs$$' 'schema check ran'; \
	require_line "$$log" '^cargo test --workspace$$' 'default workspace tests ran'; \
	require_line "$$log" '^RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-core --features libghostty-vt$$' 'opt-in core tests ran serially'; \
	require_line "$$log" '^RUST_TEST_THREADS=1 GIT_CONFIG_GLOBAL=/dev/null cargo test -p nmux-cli --features libghostty-vt$$' 'opt-in cli tests ran serially'; \
	printf 'promotion_cold_deps_verified=%s\n' "$$report"

promotion-local-sample:
	@echo "== promotion local sample: source-fetch provenance =="
	$(MAKE) source-fetch-provenance-sample
	@echo "== promotion local sample: validation =="
	$(MAKE) promotion-sample
	@echo "== promotion local sample: local workflow smoke =="
	$(MAKE) local-smoke
	@echo "== promotion local sample: cache-present offline source-fetch probe =="
	$(MAKE) source-fetch-offline-probe
	@echo "== promotion local sample: packaging archive runtime smoke =="
	$(MAKE) packaging-archive-runtime-smoke

promotion-evidence-bundle:
	@echo "writing local promotion evidence bundle"
	@set -u; \
	bundle_dir="$(PROMOTION_EVIDENCE_DIR)"; \
	run_log="$$bundle_dir/RUN.log"; \
	start_epoch="$$(date -u '+%s')"; \
	start_utc="$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
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
	cp target/source-fetch-offline/OFFLINE_PROBE.txt "$$bundle_dir/OFFLINE_PROBE.txt"; \
	cp target/packaging-libghostty-vt/package/PROVENANCE.txt "$$bundle_dir/PACKAGE_PROVENANCE.txt"; \
	cp target/packaging-libghostty-vt/package/CARGO_TREE.txt "$$bundle_dir/CARGO_TREE.txt"; \
	cp target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz "$$bundle_dir/PACKAGE_ARCHIVE.tar.gz"; \
	$(MAKE) --no-print-directory toolchain-info > "$$bundle_dir/TOOLCHAIN.txt"; \
	runtime_smoke="$$(grep -m1 '^packaged_runtime_smoke=' "$$run_log" | cut -d= -f2- || true)"; \
	if [ -z "$$runtime_smoke" ]; then \
		echo "missing packaged_runtime_smoke result in $$run_log" >&2; \
		exit 1; \
	fi; \
	local_smoke="$$(grep -m1 '^local_smoke=' "$$run_log" | cut -d= -f2- || true)"; \
	if [ -z "$$local_smoke" ]; then \
		echo "missing local_smoke result in $$run_log" >&2; \
		exit 1; \
	fi; \
	local_smoke_reattach="$$(grep -m1 '^local_smoke_reattach=' "$$run_log" | cut -d= -f2- || true)"; \
	if [ -z "$$local_smoke_reattach" ]; then \
		echo "missing local_smoke_reattach result in $$run_log" >&2; \
		exit 1; \
	fi; \
	local_smoke_socket_recreation="$$(grep -m1 '^local_smoke_socket_recreation=' "$$run_log" | cut -d= -f2- || true)"; \
	if [ -z "$$local_smoke_socket_recreation" ]; then \
		echo "missing local_smoke_socket_recreation result in $$run_log" >&2; \
		exit 1; \
	fi; \
	local_smoke_print_context="$$(grep -m1 '^local_smoke_print_context=' "$$run_log" | cut -d= -f2- || true)"; \
	if [ -z "$$local_smoke_print_context" ]; then \
		echo "missing local_smoke_print_context result in $$run_log" >&2; \
		exit 1; \
	fi; \
	local_smoke_json_info="$$(grep -m1 '^local_smoke_json_info=' "$$run_log" | cut -d= -f2- || true)"; \
	if [ -z "$$local_smoke_json_info" ]; then \
		echo "missing local_smoke_json_info result in $$run_log" >&2; \
		exit 1; \
	fi; \
	local_smoke_ready_json="$$(grep -m1 '^local_smoke_ready_json=' "$$run_log" | cut -d= -f2- || true)"; \
	if [ -z "$$local_smoke_ready_json" ]; then \
		echo "missing local_smoke_ready_json result in $$run_log" >&2; \
		exit 1; \
	fi; \
	local_smoke_managed_start="$$(grep -m1 '^local_smoke_managed_start=' "$$run_log" | cut -d= -f2- || true)"; \
	if [ -z "$$local_smoke_managed_start" ]; then \
		echo "missing local_smoke_managed_start result in $$run_log" >&2; \
		exit 1; \
	fi; \
	check_all_real="$$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^real [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$run_log")"; \
	check_all_user="$$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^user [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$run_log")"; \
	check_all_sys="$$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^sys [0-9]+([.][0-9]+)?$$/ { value = $$2 } END { if (value != "") print value }' "$$run_log")"; \
	if [ -z "$$check_all_real" ] || [ -z "$$check_all_user" ] || [ -z "$$check_all_sys" ]; then \
		echo "missing check-all time -p result in $$run_log" >&2; \
		exit 1; \
	fi; \
	hash_file() { \
		if command -v sha256sum >/dev/null 2>&1; then \
			sha256sum "$$1" | awk '{print $$1}'; \
		else \
			shasum -a 256 "$$1" | awk '{print $$1}'; \
		fi; \
	}; \
	archive_digest="$$(hash_file "$$bundle_dir/PACKAGE_ARCHIVE.tar.gz")"; \
	printf '%s  %s\n' "$$archive_digest" 'PACKAGE_ARCHIVE.tar.gz' > "$$bundle_dir/ARCHIVE.sha256"; \
	archive_sha="$$(cat "$$bundle_dir/ARCHIVE.sha256")"; \
	path_status() { \
		if [ -e "$$1" ]; then \
			printf 'present'; \
		else \
			printf 'missing'; \
		fi; \
	}; \
	write_cache_state() { \
		cache_state="$$bundle_dir/CACHE_STATE.txt"; \
		cargo_home="$${CARGO_HOME:-$$HOME/.cargo}"; \
		cargo_target_dir="$${CARGO_TARGET_DIR:-target}"; \
		{ \
			printf 'nmux promotion evidence cache state\n'; \
			printf 'generated_at_utc=%s\n' "$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
			printf 'cache_state_scope=%s\n' 'observed filesystem and environment state; does not by itself prove cold or warm cache history'; \
			printf 'HOME=%s\n' "$${HOME:-unset}"; \
			printf 'CARGO_HOME=%s\n' "$$cargo_home"; \
			printf 'CARGO_TARGET_DIR=%s\n' "$$cargo_target_dir"; \
			printf 'GHOSTTY_SOURCE_DIR=%s\n' "$${GHOSTTY_SOURCE_DIR:-unset}"; \
			printf 'GIT_CONFIG_GLOBAL=%s\n' "$${GIT_CONFIG_GLOBAL:-unset}"; \
			printf 'nix_store_status=%s\n' "$$(path_status /nix/store)"; \
			printf 'cargo_home_status=%s\n' "$$(path_status "$$cargo_home")"; \
			printf 'cargo_registry_status=%s\n' "$$(path_status "$$cargo_home/registry")"; \
			printf 'cargo_git_status=%s\n' "$$(path_status "$$cargo_home/git")"; \
			printf 'cargo_target_dir_status=%s\n' "$$(path_status "$$cargo_target_dir")"; \
			printf 'promotion_cold_target_dir_status=%s\n' "$$(path_status target/promotion-cold)"; \
			printf 'packaging_default_target_dir_status=%s\n' "$$(path_status target/packaging-default)"; \
			printf 'packaging_libghostty_vt_target_dir_status=%s\n' "$$(path_status target/packaging-libghostty-vt)"; \
			printf 'source_fetch_provenance_status=%s\n' "$$(path_status target/source-fetch-provenance)"; \
			printf 'source_fetch_offline_probe_status=%s\n' "$$(path_status target/source-fetch-offline/OFFLINE_PROBE.txt)"; \
			printf 'cache_state_note=%s\n' 'classify cold, warm, restored, or unknown in the promotion tracker from this artifact plus CI/cache setup context'; \
		} > "$$cache_state"; \
	}; \
	write_vcs_status() { \
		vcs_status="$$bundle_dir/VCS_STATUS.txt"; \
		git_status_file="$$(mktemp)"; \
		vcs_worktree_status=unknown; \
		if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then \
			if git status --porcelain=v1 > "$$git_status_file" 2>/dev/null; then \
				if [ -s "$$git_status_file" ]; then \
					vcs_worktree_status=dirty; \
				else \
					vcs_worktree_status=clean; \
				fi; \
			fi; \
		fi; \
		{ \
			printf 'nmux promotion evidence VCS status\n'; \
			printf 'generated_at_utc=%s\n' "$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
			printf 'vcs_status_scope=%s\n' 'observed repository identity and working-tree state at bundle generation time'; \
			printf 'git_revision=%s\n' "$$(git rev-parse HEAD 2>/dev/null || printf 'unknown')"; \
			printf 'git_status_porcelain=%s\n' "$$vcs_worktree_status"; \
			printf '[git_status_porcelain_v1]\n'; \
			if [ "$$vcs_worktree_status" = clean ]; then \
				printf 'clean\n'; \
			elif [ "$$vcs_worktree_status" = dirty ]; then \
				cat "$$git_status_file"; \
			else \
				printf 'unknown\n'; \
			fi; \
			if command -v jj >/dev/null 2>&1; then \
				printf 'jj_status_available=true\n'; \
				printf '[jj_status]\n'; \
				jj status 2>&1 || true; \
			else \
				printf 'jj_status_available=false\n'; \
				printf '[jj_status]\n'; \
				printf 'unavailable\n'; \
			fi; \
		} > "$$vcs_status"; \
		rm -f "$$git_status_file"; \
	}; \
	write_promotion_open_work() { \
		promotion_open_work="$$bundle_dir/PROMOTION_OPEN_WORK.txt"; \
		{ \
			printf 'nmux native VT promotion open work\n'; \
			printf 'generated_at_utc=%s\n' "$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
			printf 'promotion_decision=not-promoted\n'; \
			printf 'default_terminal_engine=interim-text\n'; \
			printf 'libghostty_vt_status=opt-in\n'; \
			printf 'open_work_scope=%s\n' 'known blockers that must be resolved before libghostty-vt can become the default engine or a regular required CI gate'; \
			printf 'open_work_ci=%s\n' 'manual promotion evidence bundle and downloaded-artifact verifier jobs still need recorded CI runs'; \
			printf 'open_work_platforms=%s\n' 'more supported local systems and at least one full cold-checkout or cold-machine run still need timing evidence'; \
			printf 'open_work_non_nix=%s\n' 'non-Nix toolchain checklist still needs a successful platform-specific validation run'; \
			printf 'open_work_source_policy=%s\n' 'packaged/default build source policy still needs a decision and evidence'; \
			printf 'open_work_packaging=%s\n' 'native VT binary distribution expectations still need supported-target, signing, notarization, installed-package, and platform distribution decisions despite local layout, archive, provenance, and runtime-smoke verifiers'; \
			printf 'open_work_frontend=%s\n' 'frontend Ghostty renderer hydration remains separate from backend terminal-state extraction'; \
		} > "$$promotion_open_work"; \
	}; \
	write_summary() { \
		completed_utc="$$1"; \
		elapsed_seconds="$$2"; \
		{ \
			printf 'nmux promotion evidence bundle\n'; \
			printf 'generated_at_utc=%s\n' "$$completed_utc"; \
			printf 'started_at_utc=%s\n' "$$start_utc"; \
			printf 'completed_at_utc=%s\n' "$$completed_utc"; \
			printf 'bundle_elapsed_seconds=%s\n' "$$elapsed_seconds"; \
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
			printf 'working_tree_status=%s\n' "$$vcs_worktree_status"; \
			printf 'vcs_status=%s\n' 'VCS_STATUS.txt'; \
			printf 'promotion_open_work=%s\n' 'PROMOTION_OPEN_WORK.txt'; \
			printf 'cache_state=%s\n' 'CACHE_STATE.txt'; \
			printf 'run_log=%s\n' 'RUN.log'; \
			printf 'toolchain=%s\n' 'TOOLCHAIN.txt'; \
			printf 'source_fetch=%s\n' 'SOURCE_FETCH.txt'; \
			printf 'source_fetch_offline_probe=%s\n' 'OFFLINE_PROBE.txt'; \
			printf 'package_provenance=%s\n' 'PACKAGE_PROVENANCE.txt'; \
			printf 'cargo_tree=%s\n' 'CARGO_TREE.txt'; \
			printf 'package_archive=%s\n' 'PACKAGE_ARCHIVE.tar.gz'; \
			printf 'check_all_real_seconds=%s\n' "$$check_all_real"; \
			printf 'check_all_user_seconds=%s\n' "$$check_all_user"; \
			printf 'check_all_sys_seconds=%s\n' "$$check_all_sys"; \
			printf 'local_smoke_reattach=%s\n' "$$local_smoke_reattach"; \
			printf 'local_smoke_socket_recreation=%s\n' "$$local_smoke_socket_recreation"; \
			printf 'local_smoke_print_context=%s\n' "$$local_smoke_print_context"; \
			printf 'local_smoke_json_info=%s\n' "$$local_smoke_json_info"; \
			printf 'local_smoke_ready_json=%s\n' "$$local_smoke_ready_json"; \
			printf 'local_smoke_managed_start=%s\n' "$$local_smoke_managed_start"; \
			printf 'local_smoke=%s\n' "$$local_smoke"; \
			printf 'archive_sha256=%s\n' "$$archive_sha"; \
			printf 'packaged_runtime_smoke=%s\n' "$$runtime_smoke"; \
		} > "$$bundle_dir/SUMMARY.txt"; \
	}; \
	write_manifest() { \
		manifest="$$bundle_dir/BUNDLE_MANIFEST.txt"; \
		{ \
			printf 'nmux promotion evidence bundle manifest\n'; \
			for name in \
				ARCHIVE.sha256 \
				CACHE_STATE.txt \
				CARGO_TREE.txt \
				OFFLINE_PROBE.txt \
				PACKAGE_ARCHIVE.tar.gz \
				PACKAGE_PROVENANCE.txt \
				PROMOTION_OPEN_WORK.txt \
				RUN.log \
				SOURCE_FETCH.txt \
				SUMMARY.txt \
				TOOLCHAIN.txt \
				VCS_STATUS.txt; do \
				printf '%s  %s\n' "$$(hash_file "$$bundle_dir/$$name")" "$$name"; \
			done; \
		} > "$$manifest"; \
	}; \
	end_epoch="$$(date -u '+%s')"; \
	completed_utc="$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
	write_cache_state; \
	write_vcs_status; \
	write_promotion_open_work; \
	write_summary "$$completed_utc" "$$((end_epoch - start_epoch))"; \
	write_manifest; \
	$(MAKE) --no-print-directory PROMOTION_EVIDENCE_DIR="$$bundle_dir" promotion-evidence-verify; \
	end_epoch="$$(date -u '+%s')"; \
	completed_utc="$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
	write_cache_state; \
	write_vcs_status; \
	write_promotion_open_work; \
	write_summary "$$completed_utc" "$$((end_epoch - start_epoch))"; \
	write_manifest; \
	$(MAKE) --no-print-directory PROMOTION_EVIDENCE_DIR="$$bundle_dir" promotion-evidence-verify; \
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
	offline_probe="$$bundle_dir/OFFLINE_PROBE.txt"; \
	package_provenance="$$bundle_dir/PACKAGE_PROVENANCE.txt"; \
	promotion_open_work="$$bundle_dir/PROMOTION_OPEN_WORK.txt"; \
	cargo_tree="$$bundle_dir/CARGO_TREE.txt"; \
	archive_file="$$bundle_dir/PACKAGE_ARCHIVE.tar.gz"; \
	archive_sha_file="$$bundle_dir/ARCHIVE.sha256"; \
	cache_state="$$bundle_dir/CACHE_STATE.txt"; \
	vcs_status="$$bundle_dir/VCS_STATUS.txt"; \
	bundle_manifest="$$bundle_dir/BUNDLE_MANIFEST.txt"; \
	hash_file() { \
		if command -v sha256sum >/dev/null 2>&1; then \
			sha256sum "$$1" | awk '{print $$1}'; \
		else \
			shasum -a 256 "$$1" | awk '{print $$1}'; \
		fi; \
	}; \
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
	require_absent_exact() { \
		file="$$1"; \
		line="$$2"; \
		description="$$3"; \
		if grep -Fxq "$$line" "$$file"; then \
			echo "unexpected promotion evidence record in $$file: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
	require_file "$$summary"; \
	require_file "$$run_log"; \
	require_file "$$toolchain"; \
	require_file "$$source_fetch"; \
	require_file "$$offline_probe"; \
	require_file "$$package_provenance"; \
	require_file "$$promotion_open_work"; \
	require_file "$$cargo_tree"; \
	require_file "$$archive_file"; \
	require_file "$$archive_sha_file"; \
	require_file "$$cache_state"; \
	require_file "$$vcs_status"; \
	require_file "$$bundle_manifest"; \
	expected_manifest="$$(mktemp)"; \
	{ \
		printf 'nmux promotion evidence bundle manifest\n'; \
		for name in \
			ARCHIVE.sha256 \
			CACHE_STATE.txt \
			CARGO_TREE.txt \
			OFFLINE_PROBE.txt \
			PACKAGE_ARCHIVE.tar.gz \
			PACKAGE_PROVENANCE.txt \
			PROMOTION_OPEN_WORK.txt \
			RUN.log \
			SOURCE_FETCH.txt \
			SUMMARY.txt \
			TOOLCHAIN.txt \
			VCS_STATUS.txt; do \
			printf '%s  %s\n' "$$(hash_file "$$bundle_dir/$$name")" "$$name"; \
		done; \
	} > "$$expected_manifest"; \
	if ! cmp -s "$$expected_manifest" "$$bundle_manifest"; then \
		echo "bundle manifest mismatch: $$bundle_manifest" >&2; \
		rm -f "$$expected_manifest"; \
		exit 1; \
	fi; \
	rm -f "$$expected_manifest"; \
	require_exact "$$summary" 'nmux promotion evidence bundle' 'summary title'; \
	require_line "$$summary" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'generation timestamp'; \
	require_line "$$summary" '^started_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'bundle start timestamp'; \
	require_line "$$summary" '^completed_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'bundle completion timestamp'; \
	require_line "$$summary" '^bundle_elapsed_seconds=[0-9]+$$' 'bundle elapsed seconds'; \
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
	github_actions="$$(awk -F= '/^github_actions=/{print $$2; exit}' "$$summary")"; \
	if [ "$$github_actions" = true ]; then \
		require_line "$$summary" '^github_server_url=https?://.+$$' 'GitHub Actions server URL'; \
		require_line "$$summary" '^github_repository=[^/]+/[^/]+$$' 'GitHub Actions repository'; \
		require_line "$$summary" '^github_run_id=[0-9]+$$' 'GitHub Actions run ID'; \
		require_line "$$summary" '^github_run_attempt=[0-9]+$$' 'GitHub Actions run attempt'; \
		require_line "$$summary" '^github_ref=refs/.+$$' 'GitHub Actions ref'; \
		require_line "$$summary" '^github_sha=[0-9a-f]{40}$$' 'GitHub Actions SHA'; \
		require_line "$$summary" '^runner_os=(Linux|macOS|Windows)$$' 'GitHub Actions runner OS'; \
		require_line "$$summary" '^runner_arch=(X64|ARM64|X86)$$' 'GitHub Actions runner architecture'; \
		require_absent_exact "$$summary" 'runner_name=unset' 'GitHub Actions runner name must not be unset'; \
	fi; \
	require_line "$$summary" '^ghostty_source_mode=(pinned-fetch|local)$$' 'Ghostty source mode'; \
	require_line "$$summary" '^GHOSTTY_SOURCE_DIR=.+$$' 'GHOSTTY_SOURCE_DIR field'; \
	require_line "$$summary" '^GIT_CONFIG_GLOBAL=.+$$' 'GIT_CONFIG_GLOBAL field'; \
	require_line "$$summary" '^working_tree_status=(clean|dirty|unknown)$$' 'working tree status'; \
	require_exact "$$summary" 'vcs_status=VCS_STATUS.txt' 'VCS status path'; \
	require_exact "$$summary" 'promotion_open_work=PROMOTION_OPEN_WORK.txt' 'promotion open work path'; \
	require_exact "$$summary" 'cache_state=CACHE_STATE.txt' 'cache state path'; \
	require_exact "$$summary" 'run_log=RUN.log' 'run log path'; \
	require_exact "$$summary" 'toolchain=TOOLCHAIN.txt' 'toolchain path'; \
	require_exact "$$summary" 'source_fetch=SOURCE_FETCH.txt' 'source-fetch path'; \
	require_exact "$$summary" 'source_fetch_offline_probe=OFFLINE_PROBE.txt' 'source-fetch offline probe path'; \
	require_exact "$$summary" 'package_provenance=PACKAGE_PROVENANCE.txt' 'package provenance path'; \
	require_exact "$$summary" 'cargo_tree=CARGO_TREE.txt' 'cargo tree path'; \
	require_exact "$$summary" 'package_archive=PACKAGE_ARCHIVE.tar.gz' 'package archive path'; \
	require_line "$$summary" '^check_all_real_seconds=[0-9]+([.][0-9]+)?$$' 'check-all real timing'; \
	require_line "$$summary" '^check_all_user_seconds=[0-9]+([.][0-9]+)?$$' 'check-all user timing'; \
	require_line "$$summary" '^check_all_sys_seconds=[0-9]+([.][0-9]+)?$$' 'check-all sys timing'; \
	require_exact "$$summary" 'local_smoke_reattach=passed' 'local workflow persisted reattach smoke'; \
	require_exact "$$summary" 'local_smoke_socket_recreation=passed' 'local workflow socket recreation smoke'; \
	require_exact "$$summary" 'local_smoke_print_context=passed' 'local workflow print-context smoke'; \
	require_exact "$$summary" 'local_smoke_json_info=passed' 'local workflow JSON informational smoke'; \
	require_exact "$$summary" 'local_smoke_ready_json=passed' 'local workflow ready-json smoke'; \
	require_exact "$$summary" 'local_smoke_managed_start=passed' 'local workflow managed start smoke'; \
	require_exact "$$summary" 'local_smoke=passed' 'local workflow smoke'; \
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
	require_line "$$run_log" '^== promotion local sample: source-fetch provenance ==$$' 'source-fetch provenance run-log section'; \
	require_exact "$$run_log" 'writing source-fetch provenance report' 'source-fetch provenance report writer ran'; \
	require_exact "$$run_log" 'verifying source-fetch provenance report' 'source-fetch provenance verifier ran'; \
	require_exact "$$run_log" 'source_fetch_provenance_verified=target/source-fetch-provenance/SOURCE_FETCH.txt' 'source-fetch provenance verifier result'; \
	require_exact "$$run_log" 'source_fetch_provenance=target/source-fetch-provenance/SOURCE_FETCH.txt' 'source-fetch provenance artifact path'; \
	$(MAKE) --no-print-directory SOURCE_FETCH_REPORT="$$source_fetch" source-fetch-provenance-verify; \
	require_exact "$$offline_probe" 'nmux source-fetch offline probe' 'offline probe title'; \
	require_exact "$$offline_probe" 'probe_scope=cache-present opt-in native VT build only; not cold checkout, CI cache miss, network-failure, or default/package source policy evidence' 'offline probe scope'; \
	require_exact "$$offline_probe" 'CARGO_NET_OFFLINE=true' 'offline probe Cargo offline mode'; \
	require_exact "$$offline_probe" 'GIT_CONFIG_GLOBAL=/dev/null' 'offline probe Git config isolation'; \
	require_exact "$$offline_probe" 'CARGO_TARGET_DIR=target/source-fetch-offline' 'offline probe target dir'; \
	require_exact "$$offline_probe" 'package=nmux-core' 'offline probe package'; \
	require_exact "$$offline_probe" 'features=libghostty-vt' 'offline probe features'; \
	require_line "$$offline_probe" '^elapsed_seconds=[0-9]+$$' 'offline probe elapsed seconds'; \
	require_exact "$$offline_probe" 'source_fetch_offline_probe=passed' 'offline probe result'; \
	require_line "$$run_log" '^== promotion local sample: cache-present offline source-fetch probe ==$$' 'offline probe run-log section'; \
	require_line "$$run_log" '^running cache-present source-fetch offline probe$$' 'offline probe command ran'; \
	require_line "$$run_log" '^   Compiling libghostty-vt-sys v0\.1\.1$$|^    Checking libghostty-vt-sys v0\.1\.1$$|^    Finished `test` profile ' 'offline probe native VT build activity'; \
	require_line "$$run_log" '^  Executable unittests src/lib\.rs \(target/source-fetch-offline/debug/deps/nmux_core-.+\)$$' 'offline probe nmux-core test binary'; \
	require_exact "$$run_log" 'verifying source-fetch offline probe report' 'offline probe verifier ran'; \
	require_exact "$$run_log" 'source_fetch_offline_probe_verified=target/source-fetch-offline/OFFLINE_PROBE.txt' 'offline probe verifier result'; \
	require_exact "$$run_log" 'source_fetch_offline_probe=target/source-fetch-offline/OFFLINE_PROBE.txt' 'offline probe artifact path'; \
	$(MAKE) --no-print-directory SOURCE_FETCH_OFFLINE_PROBE_REPORT="$$offline_probe" SOURCE_FETCH_OFFLINE_PROBE_LOG="$$run_log" source-fetch-offline-probe-verify; \
	require_line "$$run_log" '^== promotion local sample: local workflow smoke ==$$' 'local smoke run-log section'; \
	require_exact "$$run_log" 'running local nmux daemon/client smoke' 'local smoke ran'; \
	require_exact "$$run_log" 'local_smoke_reattach=passed' 'local smoke persisted reattach result'; \
	require_exact "$$run_log" 'local_smoke_socket_recreation=passed' 'local smoke socket recreation result'; \
	require_exact "$$run_log" 'local_smoke_print_context=passed' 'local smoke print-context result'; \
	require_exact "$$run_log" 'local_smoke_json_info=passed' 'local smoke JSON informational result'; \
	require_exact "$$run_log" 'local_smoke_ready_json=passed' 'local smoke ready-json result'; \
	require_exact "$$run_log" 'local_smoke_managed_start=passed' 'local smoke managed start result'; \
	require_exact "$$run_log" 'local_smoke=passed' 'local smoke result'; \
	require_line "$$package_provenance" '^\[staged_files\]$$' 'packaging staged file hashes'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/bin/nmux bytes=[0-9]+ sha256=[0-9a-f]{64}$$' 'packaged nmux wrapper hash'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/bin/nmuxd bytes=[0-9]+ sha256=[0-9a-f]{64}$$' 'packaged nmuxd wrapper hash'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/PACKAGE_METADATA\.txt bytes=[0-9]+ sha256=[0-9a-f]{64}$$' 'package metadata hash'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/libexec/nmux bytes=[0-9]+ sha256=[0-9a-f]{64}$$' 'packaged nmux binary hash'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/libexec/nmuxd bytes=[0-9]+ sha256=[0-9a-f]{64}$$' 'packaged nmuxd binary hash'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/lib/libghostty-vt.* bytes=[0-9]+ sha256=[0-9a-f]{64}$$' 'packaged native runtime library hash'; \
	require_line "$$package_provenance" '^\[native_runtime_libraries\]$$' 'packaging runtime libraries'; \
	require_line "$$package_provenance" '^target/packaging-libghostty-vt/package/lib/libghostty-vt' 'packaging runtime library path'; \
	require_line "$$package_provenance" '^package_format=local-tar-archive-layout$$' 'package metadata format'; \
	require_line "$$package_provenance" '^terminal_engine=libghostty-vt$$' 'package metadata terminal engine'; \
	require_line "$$package_provenance" '^terminal_engine_status=opt-in$$' 'package metadata terminal engine status'; \
	require_line "$$package_provenance" '^\[dynamic_dependencies\]$$' 'packaging dynamic dependencies'; \
	$(MAKE) --no-print-directory PACKAGING_PROVENANCE_MANIFEST="$$package_provenance" packaging-provenance-manifest-verify; \
	require_line "$$run_log" '^provenance_manifest_verified=target/packaging-libghostty-vt/package/PROVENANCE\.txt$$' 'package provenance verifier result'; \
	require_line "$$run_log" '^packaged_runtime_smoke_install_root=/tmp/nmuxpkg\.[^/]+/install$$' 'relocated package install root'; \
	require_line "$$run_log" '^packaged_runtime_smoke_library_env=unset$$' 'clean packaged runtime library environment'; \
	require_line "$$run_log" '^packaged_runtime_smoke=passed$$' 'runtime smoke result'; \
	require_line "$$cargo_tree" '^nmux-cli v' 'cargo tree root'; \
	require_line "$$archive_sha_file" '^[0-9a-f]{64}  PACKAGE_ARCHIVE\.tar\.gz$$' 'archive SHA-256 file'; \
	$(MAKE) --no-print-directory PACKAGING_ARCHIVE="$$archive_file" PACKAGING_ARCHIVE_SHA256="$$archive_sha_file" packaging-archive-verify || exit "$$?"; \
	require_exact "$$cache_state" 'nmux promotion evidence cache state' 'cache state title'; \
	require_line "$$cache_state" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'cache state timestamp'; \
	require_line "$$cache_state" '^cache_state_scope=.+$$' 'cache state scope'; \
	require_line "$$cache_state" '^CARGO_HOME=.+$$' 'cache state CARGO_HOME'; \
	require_line "$$cache_state" '^CARGO_TARGET_DIR=.+$$' 'cache state CARGO_TARGET_DIR'; \
	require_line "$$cache_state" '^nix_store_status=(present|missing)$$' 'Nix store cache status'; \
	require_line "$$cache_state" '^cargo_home_status=(present|missing)$$' 'Cargo home cache status'; \
	require_line "$$cache_state" '^cargo_registry_status=(present|missing)$$' 'Cargo registry cache status'; \
	require_line "$$cache_state" '^cargo_git_status=(present|missing)$$' 'Cargo git cache status'; \
	require_line "$$cache_state" '^cargo_target_dir_status=(present|missing)$$' 'Cargo target cache status'; \
	require_line "$$cache_state" '^packaging_libghostty_vt_target_dir_status=(present|missing)$$' 'native VT target cache status'; \
	require_line "$$cache_state" '^source_fetch_offline_probe_status=(present|missing)$$' 'source-fetch offline probe status'; \
	require_line "$$cache_state" '^cache_state_note=.+$$' 'cache state interpretation note'; \
	require_exact "$$vcs_status" 'nmux promotion evidence VCS status' 'VCS status title'; \
	require_line "$$vcs_status" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'VCS status timestamp'; \
	require_line "$$vcs_status" '^vcs_status_scope=.+$$' 'VCS status scope'; \
	require_line "$$vcs_status" '^git_revision=(unknown|[0-9a-f]{40})$$' 'VCS git revision'; \
	require_line "$$vcs_status" '^git_status_porcelain=(clean|dirty|unknown)$$' 'VCS git working-tree status'; \
	require_exact "$$vcs_status" '[git_status_porcelain_v1]' 'VCS git status section'; \
	require_line "$$vcs_status" '^jj_status_available=(true|false)$$' 'VCS jj availability'; \
	require_exact "$$vcs_status" '[jj_status]' 'VCS jj status section'; \
	vcs_worktree_status="$$(awk -F= '/^git_status_porcelain=/{print $$2; exit}' "$$vcs_status")"; \
	summary_git_revision="$$(awk -F= '/^git_revision=/{print $$2; exit}' "$$summary")"; \
	vcs_git_revision="$$(awk -F= '/^git_revision=/{print $$2; exit}' "$$vcs_status")"; \
	if [ "$$summary_git_revision" != "$$vcs_git_revision" ]; then \
		echo "promotion evidence git revision mismatch: SUMMARY.txt has $$summary_git_revision but VCS_STATUS.txt has $$vcs_git_revision" >&2; \
		exit 1; \
	fi; \
	if [ "$$github_actions" = true ]; then \
		github_sha="$$(awk -F= '/^github_sha=/{print $$2; exit}' "$$summary")"; \
		if [ "$$summary_git_revision" != "$$github_sha" ]; then \
			echo "promotion evidence GitHub SHA mismatch: SUMMARY.txt git_revision is $$summary_git_revision but github_sha is $$github_sha" >&2; \
			exit 1; \
		fi; \
	fi; \
	require_exact "$$summary" "working_tree_status=$$vcs_worktree_status" 'summary working tree status matches VCS artifact'; \
	require_exact "$$promotion_open_work" 'nmux native VT promotion open work' 'promotion open work title'; \
	require_line "$$promotion_open_work" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'promotion open work timestamp'; \
	require_exact "$$promotion_open_work" 'promotion_decision=not-promoted' 'promotion decision'; \
	require_exact "$$promotion_open_work" 'default_terminal_engine=interim-text' 'default terminal engine'; \
	require_exact "$$promotion_open_work" 'libghostty_vt_status=opt-in' 'libghostty-vt opt-in status'; \
	require_exact "$$promotion_open_work" 'open_work_scope=known blockers that must be resolved before libghostty-vt can become the default engine or a regular required CI gate' 'promotion open work scope'; \
	require_exact "$$promotion_open_work" 'open_work_ci=manual promotion evidence bundle and downloaded-artifact verifier jobs still need recorded CI runs' 'promotion open work CI gap'; \
	require_exact "$$promotion_open_work" 'open_work_platforms=more supported local systems and at least one full cold-checkout or cold-machine run still need timing evidence' 'promotion open work platform gap'; \
	require_exact "$$promotion_open_work" 'open_work_non_nix=non-Nix toolchain checklist still needs a successful platform-specific validation run' 'promotion open work non-Nix gap'; \
	require_exact "$$promotion_open_work" 'open_work_source_policy=packaged/default build source policy still needs a decision and evidence' 'promotion open work source-policy gap'; \
	require_exact "$$promotion_open_work" 'open_work_packaging=native VT binary distribution expectations still need supported-target, signing, notarization, installed-package, and platform distribution decisions despite local layout, archive, provenance, and runtime-smoke verifiers' 'promotion open work packaging gap'; \
	require_exact "$$promotion_open_work" 'open_work_frontend=frontend Ghostty renderer hydration remains separate from backend terminal-state extraction' 'promotion open work frontend gap'; \
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
	$(MAKE) --no-print-directory source-fetch-provenance-verify; \
	printf 'source_fetch_provenance=%s\n' "$$report"

source-fetch-provenance-verify:
	@echo "verifying source-fetch provenance report"
	@report="$(SOURCE_FETCH_REPORT)"; \
	require_file() { \
		path="$$1"; \
		if [ ! -s "$$path" ]; then \
			echo "missing or empty source-fetch provenance artifact: $$path" >&2; \
			exit 1; \
		fi; \
	}; \
	require_line() { \
		file="$$1"; \
		pattern="$$2"; \
		description="$$3"; \
		if ! grep -Eq "$$pattern" "$$file"; then \
			echo "missing source-fetch provenance record in $$file: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
	require_exact() { \
		file="$$1"; \
		line="$$2"; \
		description="$$3"; \
		if ! grep -Fxq "$$line" "$$file"; then \
			echo "missing source-fetch provenance record in $$file: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
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
	require_lock_section() { \
		package="$$1"; \
		expected="$$(mktemp)"; \
		section="$$(mktemp)"; \
		if ! cargo_lock_record "$$package" > "$$expected"; then \
			echo "missing Cargo.lock record for $$package" >&2; \
			rm -f "$$expected" "$$section"; \
			exit 1; \
		fi; \
		awk -v header="[cargo_lock:$$package]" '\
			$$0 == header { in_section = 1; next } \
			in_section && $$0 ~ /^\[[^[]/ { exit } \
			in_section { print }' "$$report" > "$$section"; \
		status=0; \
		while IFS= read -r line; do \
			if [ -n "$$line" ] && ! grep -Fxq "$$line" "$$section"; then \
				echo "missing source-fetch provenance Cargo.lock line for $$package: $$line" >&2; \
				status=1; \
				break; \
			fi; \
		done < "$$expected"; \
		rm -f "$$expected" "$$section"; \
		if [ "$$status" -ne 0 ]; then exit "$$status"; fi; \
	}; \
	require_file "$$report"; \
	require_exact "$$report" 'nmux source-fetch provenance sample' 'report title'; \
	require_line "$$report" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'generation timestamp'; \
	require_line "$$report" '^ghostty_source_mode=(pinned-fetch|local)$$' 'Ghostty source mode'; \
	require_line "$$report" '^GHOSTTY_SOURCE_DIR=.+$$' 'GHOSTTY_SOURCE_DIR field'; \
	require_line "$$report" '^GIT_CONFIG_GLOBAL=.+$$' 'GIT_CONFIG_GLOBAL field'; \
	require_exact "$$report" "Cargo.lock sha256=$$(hash_file Cargo.lock)" 'Cargo.lock hash'; \
	require_exact "$$report" '[toolchain]' 'toolchain section'; \
	require_line "$$report" '^cargo=cargo ' 'cargo version'; \
	require_line "$$report" '^rustc=rustc ' 'rustc version'; \
	require_line "$$report" '^flatc=flatc version 25\.12\.19$$' 'flatc version'; \
	require_line "$$report" '^zig=0\.15\.' 'Zig 0.15 version'; \
	require_line "$$report" '^ghostty_source_dir_status=(unset|present|missing)$$' 'Ghostty source dir status'; \
	require_exact "$$report" '[cargo_lock:libghostty-vt]' 'libghostty-vt section'; \
	require_line "$$report" '^name = "libghostty-vt"$$' 'libghostty-vt package name'; \
	require_line "$$report" '^checksum = "[0-9a-f]{64}"$$' 'libghostty-vt checksum'; \
	require_lock_section libghostty-vt; \
	require_exact "$$report" '[cargo_lock:libghostty-vt-sys]' 'libghostty-vt-sys section'; \
	require_line "$$report" '^name = "libghostty-vt-sys"$$' 'libghostty-vt-sys package name'; \
	require_lock_section libghostty-vt-sys; \
	require_exact "$$report" '[policy_note]' 'policy note section'; \
	require_exact "$$report" 'This report records local source-fetch inputs for evidence. It does not choose the default or packaged-build source policy.' 'policy note'; \
	printf 'source_fetch_provenance_verified=%s\n' "$$report"

source-fetch-offline-probe: check-vt-toolchain
	@echo "running cache-present source-fetch offline probe"
	@report_dir=target/source-fetch-offline; \
	report="$$report_dir/OFFLINE_PROBE.txt"; \
	log="$$report_dir/OFFLINE_PROBE.log"; \
	start_epoch="$$(date -u '+%s')"; \
	start_utc="$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
	rm -rf "$$report_dir"; \
	mkdir -p "$$report_dir"; \
	status=0; \
	env CARGO_NET_OFFLINE=true GIT_CONFIG_GLOBAL=/dev/null CARGO_TARGET_DIR=target/source-fetch-offline cargo test -p nmux-core --features libghostty-vt --no-run > "$$log" 2>&1 || status="$$?"; \
	end_epoch="$$(date -u '+%s')"; \
	completed_utc="$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
	{ \
		printf 'nmux source-fetch offline probe\n'; \
		printf 'generated_at_utc=%s\n' "$$completed_utc"; \
		printf 'started_at_utc=%s\n' "$$start_utc"; \
		printf 'completed_at_utc=%s\n' "$$completed_utc"; \
		printf 'elapsed_seconds=%s\n' "$$((end_epoch - start_epoch))"; \
		printf 'probe_scope=%s\n' 'cache-present opt-in native VT build only; not cold checkout, CI cache miss, network-failure, or default/package source policy evidence'; \
		printf 'ghostty_source_mode=%s\n' "$$([ -n "$${GHOSTTY_SOURCE_DIR:-}" ] && printf 'local' || printf 'pinned-fetch')"; \
		printf 'GHOSTTY_SOURCE_DIR=%s\n' "$${GHOSTTY_SOURCE_DIR:-unset}"; \
		printf 'CARGO_NET_OFFLINE=%s\n' 'true'; \
		printf 'GIT_CONFIG_GLOBAL=%s\n' '/dev/null'; \
		printf 'CARGO_TARGET_DIR=%s\n' 'target/source-fetch-offline'; \
		printf 'package=%s\n' 'nmux-core'; \
		printf 'features=%s\n' 'libghostty-vt'; \
		printf 'command=%s\n' 'cargo test -p nmux-core --features libghostty-vt --no-run'; \
		printf 'log=%s\n' "$$log"; \
		if [ "$$status" -eq 0 ]; then \
			printf 'source_fetch_offline_probe=passed\n'; \
		else \
			printf 'source_fetch_offline_probe=failed\n'; \
			printf 'exit_status=%s\n' "$$status"; \
		fi; \
	} > "$$report"; \
	cat "$$log"; \
	if [ "$$status" -ne 0 ]; then \
		echo "source-fetch offline probe failed; see $$report and $$log" >&2; \
		exit "$$status"; \
	fi; \
	$(MAKE) --no-print-directory source-fetch-offline-probe-verify; \
	printf 'source_fetch_offline_probe=%s\n' "$$report"

source-fetch-offline-probe-verify:
	@echo "verifying source-fetch offline probe report"
	@report="$(SOURCE_FETCH_OFFLINE_PROBE_REPORT)"; \
	override_log="$(SOURCE_FETCH_OFFLINE_PROBE_LOG)"; \
	require_file() { \
		path="$$1"; \
		if [ ! -s "$$path" ]; then \
			echo "missing or empty source-fetch offline probe artifact: $$path" >&2; \
			exit 1; \
		fi; \
	}; \
	require_line() { \
		file="$$1"; \
		pattern="$$2"; \
		description="$$3"; \
		if ! grep -Eq "$$pattern" "$$file"; then \
			echo "missing source-fetch offline probe record in $$file: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
	require_exact() { \
		file="$$1"; \
		line="$$2"; \
		description="$$3"; \
		if ! grep -Fxq "$$line" "$$file"; then \
			echo "missing source-fetch offline probe record in $$file: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
	require_file "$$report"; \
	if [ -n "$$override_log" ]; then \
		log="$$override_log"; \
	else \
		log="$$(awk -F= '/^log=/{print $$2; exit}' "$$report")"; \
	fi; \
	require_file "$$log"; \
	require_exact "$$report" 'nmux source-fetch offline probe' 'report title'; \
	require_line "$$report" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'generation timestamp'; \
	require_line "$$report" '^started_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'start timestamp'; \
	require_line "$$report" '^completed_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'completion timestamp'; \
	require_line "$$report" '^elapsed_seconds=[0-9]+$$' 'elapsed seconds'; \
	require_exact "$$report" 'probe_scope=cache-present opt-in native VT build only; not cold checkout, CI cache miss, network-failure, or default/package source policy evidence' 'probe scope'; \
	require_line "$$report" '^ghostty_source_mode=(pinned-fetch|local)$$' 'Ghostty source mode'; \
	require_line "$$report" '^GHOSTTY_SOURCE_DIR=.+$$' 'GHOSTTY_SOURCE_DIR field'; \
	require_exact "$$report" 'CARGO_NET_OFFLINE=true' 'Cargo offline mode'; \
	require_exact "$$report" 'GIT_CONFIG_GLOBAL=/dev/null' 'Git config isolation'; \
	require_exact "$$report" 'CARGO_TARGET_DIR=target/source-fetch-offline' 'target dir'; \
	require_exact "$$report" 'package=nmux-core' 'package'; \
	require_exact "$$report" 'features=libghostty-vt' 'features'; \
	require_exact "$$report" 'command=cargo test -p nmux-core --features libghostty-vt --no-run' 'command'; \
	require_exact "$$report" 'log=target/source-fetch-offline/OFFLINE_PROBE.log' 'log path'; \
	require_exact "$$report" 'source_fetch_offline_probe=passed' 'probe result'; \
	require_line "$$log" '^   Compiling libghostty-vt-sys v0\.1\.1$$|^    Checking libghostty-vt-sys v0\.1\.1$$|^    Finished `test` profile ' 'opt-in native VT build activity'; \
	require_line "$$log" '^  Executable unittests src/lib\.rs \(target/source-fetch-offline/debug/deps/nmux_core-.+\)$$' 'nmux-core test binary'; \
	printf 'source_fetch_offline_probe_verified=%s\n' "$$report"

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
	{ \
		printf 'nmux opt-in native VT package metadata\n'; \
		printf 'generated_at_utc=%s\n' "$$(date -u '+%Y-%m-%dT%H:%M:%SZ')"; \
		printf 'package_format=%s\n' 'local-tar-archive-layout'; \
		printf 'release_status=%s\n' 'local evidence artifact; not a signed, notarized, installed, or published release package'; \
		printf 'target_host=%s\n' "$$(rustc -vV | awk '/^host: / { print $$2 }')"; \
		printf 'terminal_engine=%s\n' 'libghostty-vt'; \
		printf 'terminal_engine_status=%s\n' 'opt-in'; \
		printf 'binaries=%s\n' 'nmux,nmuxd'; \
		printf 'runtime_library_strategy=%s\n' 'bundled dynamic libghostty-vt libraries loaded by wrapper-managed DYLD_LIBRARY_PATH/LD_LIBRARY_PATH'; \
		printf 'source_mode=%s\n' "$$([ -n "$${GHOSTTY_SOURCE_DIR:-}" ] && printf 'local' || printf 'pinned-fetch')"; \
		printf 'GHOSTTY_SOURCE_DIR=%s\n' "$${GHOSTTY_SOURCE_DIR:-unset}"; \
	} > "$$pkg_dir/PACKAGE_METADATA.txt"; \
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
	"$$pkg_dir/bin/nmuxd" --version; \
	$(MAKE) --no-print-directory PACKAGING_LAYOUT="$$pkg_dir" packaging-layout-verify

packaging-layout-verify:
	@echo "verifying existing opt-in libghostty-vt package layout"
	@pkg_dir="$(PACKAGING_LAYOUT)"; \
	metadata="$$pkg_dir/PACKAGE_METADATA.txt"; \
	require_file() { \
		path="$$1"; \
		if [ ! -s "$$path" ]; then \
			echo "missing or empty package layout artifact: $$path" >&2; \
			exit 1; \
		fi; \
	}; \
	require_executable() { \
		path="$$1"; \
		require_file "$$path"; \
		if [ ! -x "$$path" ]; then \
			echo "package layout artifact is not executable: $$path" >&2; \
			exit 1; \
		fi; \
	}; \
	require_line() { \
		file="$$1"; \
		pattern="$$2"; \
		description="$$3"; \
		if ! grep -Eq "$$pattern" "$$file"; then \
			echo "missing package layout record in $$file: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
	require_wrapper_line() { \
		wrapper="$$1"; \
		line="$$2"; \
		description="$$3"; \
		if ! grep -Fxq "$$line" "$$wrapper"; then \
			echo "missing wrapper record in $$wrapper: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
	require_executable "$$pkg_dir/bin/nmux"; \
	require_executable "$$pkg_dir/bin/nmuxd"; \
	require_executable "$$pkg_dir/libexec/nmux"; \
	require_executable "$$pkg_dir/libexec/nmuxd"; \
	require_file "$$metadata"; \
	found_runtime_library=0; \
	for lib in "$$pkg_dir"/lib/libghostty-vt*; do \
		if [ -f "$$lib" ]; then \
			require_file "$$lib"; \
			found_runtime_library=1; \
		fi; \
	done; \
	if [ "$$found_runtime_library" -ne 1 ]; then \
		echo "missing libghostty-vt runtime library in package layout: $$pkg_dir/lib" >&2; \
		exit 1; \
	fi; \
	require_line "$$metadata" '^nmux opt-in native VT package metadata$$' 'metadata title'; \
	require_line "$$metadata" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$$' 'metadata timestamp'; \
	require_line "$$metadata" '^package_format=local-tar-archive-layout$$' 'package format'; \
	require_line "$$metadata" '^release_status=local evidence artifact; not a signed, notarized, installed, or published release package$$' 'release status'; \
	require_line "$$metadata" '^target_host=.+$$' 'target host'; \
	require_line "$$metadata" '^terminal_engine=libghostty-vt$$' 'terminal engine'; \
	require_line "$$metadata" '^terminal_engine_status=opt-in$$' 'terminal engine status'; \
	require_line "$$metadata" '^binaries=nmux,nmuxd$$' 'binary list'; \
	require_line "$$metadata" '^runtime_library_strategy=bundled dynamic libghostty-vt libraries loaded by wrapper-managed DYLD_LIBRARY_PATH/LD_LIBRARY_PATH$$' 'runtime-library strategy'; \
	require_line "$$metadata" '^source_mode=(pinned-fetch|local)$$' 'source mode'; \
	require_line "$$metadata" '^GHOSTTY_SOURCE_DIR=.+$$' 'GHOSTTY_SOURCE_DIR'; \
	for bin in nmux nmuxd; do \
		wrapper="$$pkg_dir/bin/$$bin"; \
		require_wrapper_line "$$wrapper" '#!/bin/sh' 'shell shebang'; \
		require_wrapper_line "$$wrapper" 'set -eu' 'strict shell mode'; \
		require_wrapper_line "$$wrapper" 'bin_dir=$$(CDPATH= cd "$$(dirname "$$0")" && pwd)' 'relative wrapper directory'; \
		require_wrapper_line "$$wrapper" 'lib_dir=$$bin_dir/../lib' 'relative library directory'; \
		require_wrapper_line "$$wrapper" 'DYLD_LIBRARY_PATH=$$lib_dir$${DYLD_LIBRARY_PATH:+:$$DYLD_LIBRARY_PATH}' 'DYLD library path'; \
		require_wrapper_line "$$wrapper" 'LD_LIBRARY_PATH=$$lib_dir$${LD_LIBRARY_PATH:+:$$LD_LIBRARY_PATH}' 'LD library path'; \
		require_wrapper_line "$$wrapper" 'export DYLD_LIBRARY_PATH LD_LIBRARY_PATH' 'library path export'; \
		require_wrapper_line "$$wrapper" "exec \"\$$bin_dir/../libexec/$$bin\" \"\$$@\"" 'relative libexec handoff'; \
	done; \
	printf 'packaged libghostty-vt nmux version: '; \
	env -u DYLD_LIBRARY_PATH -u LD_LIBRARY_PATH "$$pkg_dir/bin/nmux" --version; \
	printf 'packaged libghostty-vt nmuxd version: '; \
	env -u DYLD_LIBRARY_PATH -u LD_LIBRARY_PATH "$$pkg_dir/bin/nmuxd" --version; \
	printf 'packaging_layout_verified=%s\n' "$$pkg_dir"

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
		printf '\n[package_metadata]\n'; \
		cat "$$pkg_dir/PACKAGE_METADATA.txt"; \
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
	@$(MAKE) --no-print-directory packaging-provenance-manifest-verify

packaging-provenance-manifest-verify:
	@echo "verifying opt-in libghostty-vt package provenance manifest"
	@manifest="$(PACKAGING_PROVENANCE_MANIFEST)"; \
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
	require_dynamic_dependency() { \
		bin="$$1"; \
		dependency="$$2"; \
		description="$$3"; \
		if ! awk -v bin="$$bin" -v dependency="$$dependency" '\
			$$0 == bin || $$0 == bin ":" { in_bin = 1; next } \
			in_bin && $$0 ~ /^target\/packaging-libghostty-vt\/package\/libexec\/nmuxd?:?$$/ { exit } \
			in_bin && index($$0, dependency) { found = 1; exit } \
			END { exit(found ? 0 : 1) }' "$$manifest"; then \
			echo "missing dynamic dependency record for $$description" >&2; \
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
	require_line '^\[package_metadata\]$$' 'package metadata section'; \
	require_line '^package_format=local-tar-archive-layout$$' 'package metadata format'; \
	require_line '^target_host=.+$$' 'package metadata target host'; \
	require_line '^terminal_engine=libghostty-vt$$' 'package metadata terminal engine'; \
	require_line '^terminal_engine_status=opt-in$$' 'package metadata terminal engine status'; \
	require_line '^runtime_library_strategy=bundled dynamic libghostty-vt libraries loaded by wrapper-managed DYLD_LIBRARY_PATH/LD_LIBRARY_PATH$$' 'package metadata runtime-library strategy'; \
	require_line '^source_mode=(pinned-fetch|local)$$' 'package metadata source mode'; \
	require_line '^\[staged_files\]$$' 'staged file section'; \
	require_file_record "$$pkg_dir/bin/nmux"; \
	require_file_record "$$pkg_dir/bin/nmuxd"; \
	require_file_record "$$pkg_dir/PACKAGE_METADATA.txt"; \
	require_file_record "$$pkg_dir/libexec/nmux"; \
	require_file_record "$$pkg_dir/libexec/nmuxd"; \
	require_line "^$$pkg_dir/lib/libghostty-vt.* bytes=[0-9]+ sha256=[0-9a-f]{64}$$" 'libghostty-vt runtime library hash'; \
	require_line '^\[native_runtime_libraries\]$$' 'native runtime library section'; \
	require_line "^$$pkg_dir/lib/libghostty-vt" 'native runtime library path'; \
	require_line '^\[dynamic_dependencies\]$$' 'dynamic dependencies section'; \
	require_line "^$$pkg_dir/libexec/nmux$$" 'nmux dynamic dependency heading'; \
	require_line "^$$pkg_dir/libexec/nmuxd$$" 'nmuxd dynamic dependency heading'; \
	require_dynamic_dependency "$$pkg_dir/libexec/nmux" 'libghostty-vt' 'nmux libghostty-vt runtime library'; \
	require_dynamic_dependency "$$pkg_dir/libexec/nmuxd" 'libghostty-vt' 'nmuxd libghostty-vt runtime library'; \
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
	"$$check_dir/package/bin/nmuxd" --version; \
	$(MAKE) --no-print-directory PACKAGING_ARCHIVE="$$archive" PACKAGING_ARCHIVE_SHA256="$$archive.sha256" packaging-archive-verify

packaging-archive-verify:
	@echo "verifying existing opt-in libghostty-vt package archive"
	@archive="$(PACKAGING_ARCHIVE)"; \
	archive_sha_file="$(PACKAGING_ARCHIVE_SHA256)"; \
	work_dir="$$(mktemp -d "/tmp/nmuxpkg-verify.XXXXXX")"; \
	trap 'status=$$?; rm -rf "$$work_dir"; exit "$$status"' EXIT INT TERM; \
	hash_file() { \
		if command -v sha256sum >/dev/null 2>&1; then \
			sha256sum "$$1" | awk '{print $$1}'; \
		else \
			shasum -a 256 "$$1" | awk '{print $$1}'; \
		fi; \
	}; \
	require_file() { \
		path="$$1"; \
		if [ ! -s "$$path" ]; then \
			echo "missing or empty archive artifact: $$path" >&2; \
			exit 1; \
		fi; \
	}; \
	require_line() { \
		file="$$1"; \
		pattern="$$2"; \
		description="$$3"; \
		if ! grep -Eq "$$pattern" "$$file"; then \
			echo "missing archive record in $$file: $$description" >&2; \
			exit 1; \
		fi; \
	}; \
	require_file_record() { \
		rel="$$1"; \
		file="$$pkg_dir/$$rel"; \
		staged="target/packaging-libghostty-vt/package/$$rel"; \
		require_file "$$file"; \
		bytes="$$(wc -c < "$$file" | tr -d ' ')"; \
		sha="$$(hash_file "$$file")"; \
		if ! grep -Fxq "$$staged bytes=$$bytes sha256=$$sha" "$$provenance"; then \
			echo "missing or mismatched staged file hash record: $$staged" >&2; \
			exit 1; \
		fi; \
	}; \
	require_file "$$archive"; \
	require_file "$$archive_sha_file"; \
	expected_sha="$$(awk 'NR == 1 { print $$1 }' "$$archive_sha_file")"; \
	if ! printf '%s\n' "$$expected_sha" | grep -Eq '^[0-9a-f]{64}$$'; then \
		echo "invalid archive SHA-256 record: $$archive_sha_file" >&2; \
		exit 1; \
	fi; \
	actual_sha="$$(hash_file "$$archive")"; \
	if [ "$$actual_sha" != "$$expected_sha" ]; then \
		echo "archive SHA-256 mismatch: $$archive" >&2; \
		echo "expected $$expected_sha" >&2; \
		echo "actual   $$actual_sha" >&2; \
		exit 1; \
	fi; \
	if tar -tzf "$$archive" | awk '$$0 ~ /^\// || $$0 ~ /(^|\/)\.\.($$|\/)/ { bad = 1 } END { exit bad ? 0 : 1 }'; then \
		echo "archive contains unsafe absolute or parent-relative paths: $$archive" >&2; \
		exit 1; \
	fi; \
	tar -C "$$work_dir" -xzf "$$archive"; \
	pkg_dir="$$work_dir/package"; \
	metadata="$$pkg_dir/PACKAGE_METADATA.txt"; \
	provenance="$$pkg_dir/PROVENANCE.txt"; \
	cargo_tree="$$pkg_dir/CARGO_TREE.txt"; \
	require_file "$$metadata"; \
	require_file "$$provenance"; \
	require_file "$$cargo_tree"; \
	require_file_record "bin/nmux"; \
	require_file_record "bin/nmuxd"; \
	require_file_record "PACKAGE_METADATA.txt"; \
	require_file_record "CARGO_TREE.txt"; \
	require_file_record "libexec/nmux"; \
	require_file_record "libexec/nmuxd"; \
	found_runtime_library=0; \
	for lib in "$$pkg_dir"/lib/libghostty-vt*; do \
		if [ -f "$$lib" ]; then \
			found_runtime_library=1; \
			require_file_record "lib/$$(basename "$$lib")"; \
		fi; \
	done; \
	if [ "$$found_runtime_library" -ne 1 ]; then \
		echo "missing libghostty-vt runtime library in archive: $$archive" >&2; \
		exit 1; \
	fi; \
	require_line "$$metadata" '^package_format=local-tar-archive-layout$$' 'package metadata format'; \
	require_line "$$metadata" '^terminal_engine=libghostty-vt$$' 'package metadata terminal engine'; \
	require_line "$$metadata" '^terminal_engine_status=opt-in$$' 'package metadata terminal engine status'; \
	require_line "$$metadata" '^runtime_library_strategy=bundled dynamic libghostty-vt libraries loaded by wrapper-managed DYLD_LIBRARY_PATH/LD_LIBRARY_PATH$$' 'package metadata runtime-library strategy'; \
	require_line "$$provenance" '^\[package_metadata\]$$' 'provenance package metadata section'; \
	require_line "$$provenance" '^\[staged_files\]$$' 'provenance staged file section'; \
	require_line "$$provenance" '^\[native_runtime_libraries\]$$' 'provenance runtime library section'; \
	require_line "$$provenance" '^\[dynamic_dependencies\]$$' 'provenance dynamic dependencies section'; \
	require_line "$$provenance" '^\[cargo_tree\]$$' 'provenance cargo tree section'; \
	require_line "$$provenance" '^target/packaging-libghostty-vt/package/libexec/nmux$$' 'nmux dynamic dependency heading'; \
	require_line "$$provenance" '^target/packaging-libghostty-vt/package/libexec/nmuxd$$' 'nmuxd dynamic dependency heading'; \
	if ! awk '\
		$$0 == "target/packaging-libghostty-vt/package/libexec/nmux" { in_nmux = 1; in_nmuxd = 0; next } \
		$$0 == "target/packaging-libghostty-vt/package/libexec/nmuxd" { in_nmux = 0; in_nmuxd = 1; next } \
		in_nmux && index($$0, "libghostty-vt") { found_nmux = 1 } \
		in_nmuxd && index($$0, "libghostty-vt") { found_nmuxd = 1 } \
		END { exit(found_nmux && found_nmuxd ? 0 : 1) }' "$$provenance"; then \
		echo "missing libghostty-vt dynamic dependency records in archive provenance" >&2; \
		exit 1; \
	fi; \
	require_line "$$cargo_tree" '^nmux-cli v' 'cargo tree root'; \
	$(MAKE) --no-print-directory PACKAGING_LAYOUT="$$pkg_dir" packaging-layout-verify; \
	printf 'packaging_archive_verified=%s\n' "$$archive"

packaging-archive-runtime-smoke: packaging-archive-sample
	@echo "running packaged opt-in libghostty-vt archive runtime smoke"
	@archive=target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz; \
	work_dir="$$(mktemp -d "/tmp/nmuxpkg.XXXXXX")"; \
	install_root="$$work_dir/install"; \
	mkdir -p "$$install_root"; \
	tar -C "$$install_root" -xzf "$$archive"; \
	pkg_dir="$$install_root/package"; \
	socket="$$work_dir/nmuxd.sock"; \
	client_out="$$work_dir/client.out"; \
	client_err="$$work_dir/client.err"; \
	daemon_out="$$work_dir/daemon.out"; \
	daemon_err="$$work_dir/daemon.err"; \
	daemon_pid=""; \
	trap 'status=$$?; if [ -n "$${daemon_pid:-}" ] && kill -0 "$$daemon_pid" >/dev/null 2>&1; then kill "$$daemon_pid" >/dev/null 2>&1 || true; wait "$$daemon_pid" >/dev/null 2>&1 || true; fi; rm -rf "$$work_dir"; exit "$$status"' EXIT INT TERM; \
	printf 'packaged_runtime_smoke_install_root=%s\n' "$$install_root"; \
	printf 'packaged_runtime_smoke_library_env=unset\n'; \
	env -u DYLD_LIBRARY_PATH -u LD_LIBRARY_PATH "$$pkg_dir/bin/nmuxd" --socket "$$socket" --one-shot --terminal-engine libghostty-vt --command "printf 'packaged-runtime-smoke\n'; cat >/dev/null" >"$$daemon_out" 2>"$$daemon_err" & \
	daemon_pid="$$!"; \
	if ! env -u DYLD_LIBRARY_PATH -u LD_LIBRARY_PATH "$$pkg_dir/bin/nmux" --socket "$$socket" --connect-timeout-ms 5000 --no-input --scrollback-start 1 --scrollback-count 5 >"$$client_out" 2>"$$client_err"; then \
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
