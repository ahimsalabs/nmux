#!/usr/bin/env bash
set -euo pipefail
echo "verifying local promotion evidence bundle"
bundle_dir="$PROMOTION_EVIDENCE_DIR"
summary="$bundle_dir/SUMMARY.txt"
run_log="$bundle_dir/RUN.log"
toolchain="$bundle_dir/TOOLCHAIN.txt"
source_fetch="$bundle_dir/SOURCE_FETCH.txt"
offline_probe="$bundle_dir/OFFLINE_PROBE.txt"
package_provenance="$bundle_dir/PACKAGE_PROVENANCE.txt"
promotion_open_work="$bundle_dir/PROMOTION_OPEN_WORK.txt"
cargo_tree="$bundle_dir/CARGO_TREE.txt"
archive_file="$bundle_dir/PACKAGE_ARCHIVE.tar.gz"
archive_sha_file="$bundle_dir/ARCHIVE.sha256"
cache_state="$bundle_dir/CACHE_STATE.txt"
vcs_status="$bundle_dir/VCS_STATUS.txt"
bundle_manifest="$bundle_dir/BUNDLE_MANIFEST.txt"
hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}
require_file() {
    path="$1"
    if [ ! -s "$path" ]; then
        echo "missing or empty promotion evidence artifact: $path" >&2
        exit 1
    fi
}
require_line() {
    file="$1"
    pattern="$2"
    description="$3"
    if ! grep -Eq "$pattern" "$file"; then
        echo "missing promotion evidence record in $file: $description" >&2
        exit 1
    fi
}
require_exact() {
    file="$1"
    line="$2"
    description="$3"
    if ! grep -Fxq "$line" "$file"; then
        echo "missing promotion evidence record in $file: $description" >&2
        exit 1
    fi
}
require_absent_exact() {
    file="$1"
    line="$2"
    description="$3"
    if grep -Fxq "$line" "$file"; then
        echo "unexpected promotion evidence record in $file: $description" >&2
        exit 1
    fi
}
require_file "$summary"
require_file "$run_log"
require_file "$toolchain"
require_file "$source_fetch"
require_file "$offline_probe"
require_file "$package_provenance"
require_file "$promotion_open_work"
require_file "$cargo_tree"
require_file "$archive_file"
require_file "$archive_sha_file"
require_file "$cache_state"
require_file "$vcs_status"
require_file "$bundle_manifest"
expected_manifest="$(mktemp)"
{
    printf 'nmux promotion evidence bundle manifest\n'
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
        VCS_STATUS.txt; do
        printf '%s  %s\n' "$(hash_file "$bundle_dir/$name")" "$name"
    done
} > "$expected_manifest"
if ! cmp -s "$expected_manifest" "$bundle_manifest"; then
    echo "bundle manifest mismatch: $bundle_manifest" >&2
    rm -f "$expected_manifest"
    exit 1
fi
rm -f "$expected_manifest"
require_exact "$summary" 'nmux promotion evidence bundle' 'summary title'
require_line "$summary" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'generation timestamp'
require_line "$summary" '^started_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'bundle start timestamp'
require_line "$summary" '^completed_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'bundle completion timestamp'
require_line "$summary" '^bundle_elapsed_seconds=[0-9]+$' 'bundle elapsed seconds'
require_line "$summary" '^host=.+$' 'host identity'
require_line "$summary" '^git_revision=(unknown|[0-9a-f]{40})$' 'git revision'
require_line "$summary" '^github_actions=(true|false)$' 'GitHub Actions flag'
require_line "$summary" '^github_server_url=.+$' 'GitHub server URL field'
require_line "$summary" '^github_repository=.+$' 'GitHub repository field'
require_line "$summary" '^github_run_id=.+$' 'GitHub run ID field'
require_line "$summary" '^github_run_attempt=.+$' 'GitHub run attempt field'
require_line "$summary" '^github_ref=.+$' 'GitHub ref field'
require_line "$summary" '^github_sha=.+$' 'GitHub SHA field'
require_line "$summary" '^runner_os=.+$' 'runner OS field'
require_line "$summary" '^runner_arch=.+$' 'runner architecture field'
require_line "$summary" '^runner_name=.+$' 'runner name field'
github_actions="$(awk -F= '/^github_actions=/{print $2; exit}' "$summary")"
if [ "$github_actions" = true ]; then
    require_line "$summary" '^github_server_url=https?://.+$' 'GitHub Actions server URL'
    require_line "$summary" '^github_repository=[^/]+/[^/]+$' 'GitHub Actions repository'
    require_line "$summary" '^github_run_id=[0-9]+$' 'GitHub Actions run ID'
    require_line "$summary" '^github_run_attempt=[0-9]+$' 'GitHub Actions run attempt'
    require_line "$summary" '^github_ref=refs/.+$' 'GitHub Actions ref'
    require_line "$summary" '^github_sha=[0-9a-f]{40}$' 'GitHub Actions SHA'
    require_line "$summary" '^runner_os=(Linux|macOS|Windows)$' 'GitHub Actions runner OS'
    require_line "$summary" '^runner_arch=(X64|ARM64|X86)$' 'GitHub Actions runner architecture'
    require_absent_exact "$summary" 'runner_name=unset' 'GitHub Actions runner name must not be unset'
fi
require_line "$summary" '^ghostty_source_mode=(pinned-fetch|local)$' 'Ghostty source mode'
require_line "$summary" '^GHOSTTY_SOURCE_DIR=.+$' 'GHOSTTY_SOURCE_DIR field'
require_line "$summary" '^GIT_CONFIG_GLOBAL=.+$' 'GIT_CONFIG_GLOBAL field'
require_line "$summary" '^working_tree_status=(clean|dirty|unknown)$' 'working tree status'
require_exact "$summary" 'vcs_status=VCS_STATUS.txt' 'VCS status path'
require_exact "$summary" 'promotion_open_work=PROMOTION_OPEN_WORK.txt' 'promotion open work path'
require_exact "$summary" 'cache_state=CACHE_STATE.txt' 'cache state path'
require_exact "$summary" 'run_log=RUN.log' 'run log path'
require_exact "$summary" 'toolchain=TOOLCHAIN.txt' 'toolchain path'
require_exact "$summary" 'source_fetch=SOURCE_FETCH.txt' 'source-fetch path'
require_exact "$summary" 'source_fetch_offline_probe=OFFLINE_PROBE.txt' 'source-fetch offline probe path'
require_exact "$summary" 'package_provenance=PACKAGE_PROVENANCE.txt' 'package provenance path'
require_exact "$summary" 'cargo_tree=CARGO_TREE.txt' 'cargo tree path'
require_exact "$summary" 'package_archive=PACKAGE_ARCHIVE.tar.gz' 'package archive path'
require_line "$summary" '^check_all_real_seconds=[0-9]+([.][0-9]+)?$' 'check-all real timing'
require_line "$summary" '^check_all_user_seconds=[0-9]+([.][0-9]+)?$' 'check-all user timing'
require_line "$summary" '^check_all_sys_seconds=[0-9]+([.][0-9]+)?$' 'check-all sys timing'
require_exact "$summary" 'local_smoke_reattach=passed' 'local workflow persisted reattach smoke'
require_exact "$summary" 'local_smoke_socket_recreation=passed' 'local workflow socket recreation smoke'
require_exact "$summary" 'local_smoke_print_context=passed' 'local workflow print-context smoke'
require_exact "$summary" 'local_smoke_json_info=passed' 'local workflow JSON informational smoke'
require_exact "$summary" 'local_smoke_ready_json=passed' 'local workflow ready-json smoke'
require_exact "$summary" 'local_smoke_managed_start=passed' 'local workflow managed start smoke'
require_exact "$summary" 'local_smoke=passed' 'local workflow smoke'
check_all_real="$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^real [0-9]+([.][0-9]+)?$/ { value = $2 } END { if (value != "") print value }' "$run_log")"
check_all_user="$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^user [0-9]+([.][0-9]+)?$/ { value = $2 } END { if (value != "") print value }' "$run_log")"
check_all_sys="$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^sys [0-9]+([.][0-9]+)?$/ { value = $2 } END { if (value != "") print value }' "$run_log")"
if [ -z "$check_all_real" ] || [ -z "$check_all_user" ] || [ -z "$check_all_sys" ]; then
    echo "missing check-all time -p result in $run_log" >&2
    exit 1
fi
require_exact "$summary" "check_all_real_seconds=$check_all_real" 'check-all real timing matches run log'
require_exact "$summary" "check_all_user_seconds=$check_all_user" 'check-all user timing matches run log'
require_exact "$summary" "check_all_sys_seconds=$check_all_sys" 'check-all sys timing matches run log'
archive_sha="$(cat "$archive_sha_file")"
require_exact "$summary" "archive_sha256=$archive_sha" 'archive SHA-256'
require_exact "$summary" 'packaged_runtime_smoke=passed' 'packaged runtime smoke'
require_line "$toolchain" '^cargo=cargo ' 'cargo version'
require_line "$toolchain" '^rustc=rustc ' 'rustc version'
require_line "$toolchain" '^flatc=flatc version 25\.12\.19$' 'flatc version'
require_line "$toolchain" '^zig=0\.15\.' 'Zig version'
require_line "$source_fetch" '^Cargo\.lock sha256=[0-9a-f]{64}$' 'source-fetch Cargo.lock hash'
require_line "$source_fetch" '^\[cargo_lock:libghostty-vt\]$' 'source-fetch libghostty-vt record'
require_line "$source_fetch" '^name = "libghostty-vt"$' 'source-fetch libghostty-vt package name'
require_line "$source_fetch" '^source = "git\+https://github\.com/uzaaft/libghostty-rs\.git\?rev=31d1f70004ff80727e36437cd540984f927333ce#31d1f70004ff80727e36437cd540984f927333ce"$' 'source-fetch pinned libghostty-vt revision'
require_line "$source_fetch" '^\[cargo_lock:libghostty-vt-sys\]$' 'source-fetch libghostty-vt-sys record'
require_line "$source_fetch" '^name = "libghostty-vt-sys"$' 'source-fetch libghostty-vt-sys package name'
require_line "$run_log" '^== promotion local sample: source-fetch provenance ==$' 'source-fetch provenance run-log section'
require_exact "$run_log" 'writing source-fetch provenance report' 'source-fetch provenance report writer ran'
require_exact "$run_log" 'verifying source-fetch provenance report' 'source-fetch provenance verifier ran'
require_exact "$run_log" 'source_fetch_provenance_verified=target/source-fetch-provenance/SOURCE_FETCH.txt' 'source-fetch provenance verifier result'
require_exact "$run_log" 'source_fetch_provenance=target/source-fetch-provenance/SOURCE_FETCH.txt' 'source-fetch provenance artifact path'
SOURCE_FETCH_REPORT="$source_fetch" just source-fetch-provenance-verify
require_exact "$offline_probe" 'nmux source-fetch offline probe' 'offline probe title'
require_exact "$offline_probe" 'probe_scope=cache-present opt-in native VT build only; not cold checkout, CI cache miss, network-failure, or default/package source policy evidence' 'offline probe scope'
require_exact "$offline_probe" 'CARGO_NET_OFFLINE=true' 'offline probe Cargo offline mode'
require_exact "$offline_probe" 'GIT_CONFIG_GLOBAL=/dev/null' 'offline probe Git config isolation'
require_exact "$offline_probe" 'CARGO_TARGET_DIR=target/source-fetch-offline' 'offline probe target dir'
require_exact "$offline_probe" 'package=nmux-core' 'offline probe package'
require_exact "$offline_probe" 'features=libghostty-vt' 'offline probe features'
require_line "$offline_probe" '^elapsed_seconds=[0-9]+$' 'offline probe elapsed seconds'
require_exact "$offline_probe" 'source_fetch_offline_probe=passed' 'offline probe result'
require_line "$run_log" '^== promotion local sample: cache-present offline source-fetch probe ==$' 'offline probe run-log section'
require_line "$run_log" '^running cache-present source-fetch offline probe$' 'offline probe command ran'
require_line "$run_log" '^   Compiling libghostty-vt-sys v0\.1\.1$|^    Checking libghostty-vt-sys v0\.1\.1$|^    Finished `test` profile ' 'offline probe native VT build activity'
require_line "$run_log" '^  Executable unittests src/lib\.rs \(target/source-fetch-offline/debug/deps/nmux_core-.+\)$' 'offline probe nmux-core test binary'
require_exact "$run_log" 'verifying source-fetch offline probe report' 'offline probe verifier ran'
require_exact "$run_log" 'source_fetch_offline_probe_verified=target/source-fetch-offline/OFFLINE_PROBE.txt' 'offline probe verifier result'
require_exact "$run_log" 'source_fetch_offline_probe=target/source-fetch-offline/OFFLINE_PROBE.txt' 'offline probe artifact path'
SOURCE_FETCH_OFFLINE_PROBE_REPORT="$offline_probe" SOURCE_FETCH_OFFLINE_PROBE_LOG="$run_log" just source-fetch-offline-probe-verify
require_line "$run_log" '^== promotion local sample: local workflow smoke ==$' 'local smoke run-log section'
require_exact "$run_log" 'running local nmux daemon/client smoke' 'local smoke ran'
require_exact "$run_log" 'local_smoke_reattach=passed' 'local smoke persisted reattach result'
require_exact "$run_log" 'local_smoke_socket_recreation=passed' 'local smoke socket recreation result'
require_exact "$run_log" 'local_smoke_print_context=passed' 'local smoke print-context result'
require_exact "$run_log" 'local_smoke_json_info=passed' 'local smoke JSON informational result'
require_exact "$run_log" 'local_smoke_ready_json=passed' 'local smoke ready-json result'
require_exact "$run_log" 'local_smoke_managed_start=passed' 'local smoke managed start result'
require_exact "$run_log" 'local_smoke=passed' 'local smoke result'
require_line "$package_provenance" '^\[staged_files\]$' 'packaging staged file hashes'
require_line "$package_provenance" '^target/packaging-libghostty-vt/package/bin/nmux bytes=[0-9]+ sha256=[0-9a-f]{64}$' 'packaged nmux wrapper hash'
require_line "$package_provenance" '^target/packaging-libghostty-vt/package/PACKAGE_METADATA\.txt bytes=[0-9]+ sha256=[0-9a-f]{64}$' 'package metadata hash'
require_line "$package_provenance" '^target/packaging-libghostty-vt/package/libexec/nmux bytes=[0-9]+ sha256=[0-9a-f]{64}$' 'packaged nmux binary hash'
require_line "$package_provenance" '^target/packaging-libghostty-vt/package/lib/libghostty-vt.* bytes=[0-9]+ sha256=[0-9a-f]{64}$' 'packaged native runtime library hash'
require_line "$package_provenance" '^\[native_runtime_libraries\]$' 'packaging runtime libraries'
require_line "$package_provenance" '^target/packaging-libghostty-vt/package/lib/libghostty-vt' 'packaging runtime library path'
require_line "$package_provenance" '^package_format=local-tar-archive-layout$' 'package metadata format'
require_line "$package_provenance" '^terminal_engine=libghostty-vt$' 'package metadata terminal engine'
require_line "$package_provenance" '^terminal_engine_status=opt-in$' 'package metadata terminal engine status'
require_line "$package_provenance" '^\[dynamic_dependencies\]$' 'packaging dynamic dependencies'
PACKAGING_PROVENANCE_MANIFEST="$package_provenance" just packaging-provenance-manifest-verify
require_line "$run_log" '^provenance_manifest_verified=target/packaging-libghostty-vt/package/PROVENANCE\.txt$' 'package provenance verifier result'
require_line "$run_log" '^packaged_runtime_smoke_install_root=/tmp/nmuxpkg\.[^/]+/install$' 'relocated package install root'
require_line "$run_log" '^packaged_runtime_smoke_library_env=unset$' 'clean packaged runtime library environment'
require_line "$run_log" '^packaged_runtime_smoke=passed$' 'runtime smoke result'
require_line "$cargo_tree" '^nmux-cli v' 'cargo tree root'
require_line "$archive_sha_file" '^[0-9a-f]{64}  PACKAGE_ARCHIVE\.tar\.gz$' 'archive SHA-256 file'
PACKAGING_ARCHIVE="$archive_file" PACKAGING_ARCHIVE_SHA256="$archive_sha_file" just packaging-archive-verify || exit "$?"
require_exact "$cache_state" 'nmux promotion evidence cache state' 'cache state title'
require_line "$cache_state" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'cache state timestamp'
require_line "$cache_state" '^cache_state_scope=.+$' 'cache state scope'
require_line "$cache_state" '^CARGO_HOME=.+$' 'cache state CARGO_HOME'
require_line "$cache_state" '^CARGO_TARGET_DIR=.+$' 'cache state CARGO_TARGET_DIR'
require_line "$cache_state" '^nix_store_status=(present|missing)$' 'Nix store cache status'
require_line "$cache_state" '^cargo_home_status=(present|missing)$' 'Cargo home cache status'
require_line "$cache_state" '^cargo_registry_status=(present|missing)$' 'Cargo registry cache status'
require_line "$cache_state" '^cargo_git_status=(present|missing)$' 'Cargo git cache status'
require_line "$cache_state" '^cargo_target_dir_status=(present|missing)$' 'Cargo target cache status'
require_line "$cache_state" '^packaging_libghostty_vt_target_dir_status=(present|missing)$' 'native VT target cache status'
require_line "$cache_state" '^source_fetch_offline_probe_status=(present|missing)$' 'source-fetch offline probe status'
require_line "$cache_state" '^cache_state_note=.+$' 'cache state interpretation note'
require_exact "$vcs_status" 'nmux promotion evidence VCS status' 'VCS status title'
require_line "$vcs_status" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'VCS status timestamp'
require_line "$vcs_status" '^vcs_status_scope=.+$' 'VCS status scope'
require_line "$vcs_status" '^git_revision=(unknown|[0-9a-f]{40})$' 'VCS git revision'
require_line "$vcs_status" '^git_status_porcelain=(clean|dirty|unknown)$' 'VCS git working-tree status'
require_exact "$vcs_status" '[git_status_porcelain_v1]' 'VCS git status section'
require_line "$vcs_status" '^jj_status_available=(true|false)$' 'VCS jj availability'
require_exact "$vcs_status" '[jj_status]' 'VCS jj status section'
vcs_worktree_status="$(awk -F= '/^git_status_porcelain=/{print $2; exit}' "$vcs_status")"
summary_git_revision="$(awk -F= '/^git_revision=/{print $2; exit}' "$summary")"
vcs_git_revision="$(awk -F= '/^git_revision=/{print $2; exit}' "$vcs_status")"
if [ "$summary_git_revision" != "$vcs_git_revision" ]; then
    echo "promotion evidence git revision mismatch: SUMMARY.txt has $summary_git_revision but VCS_STATUS.txt has $vcs_git_revision" >&2
    exit 1
fi
if [ "$github_actions" = true ]; then
    github_sha="$(awk -F= '/^github_sha=/{print $2; exit}' "$summary")"
    if [ "$summary_git_revision" != "$github_sha" ]; then
        echo "promotion evidence GitHub SHA mismatch: SUMMARY.txt git_revision is $summary_git_revision but github_sha is $github_sha" >&2
        exit 1
    fi
fi
require_exact "$summary" "working_tree_status=$vcs_worktree_status" 'summary working tree status matches VCS artifact'
require_exact "$promotion_open_work" 'nmux native VT promotion open work' 'promotion open work title'
require_line "$promotion_open_work" '^generated_at_utc=[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' 'promotion open work timestamp'
require_exact "$promotion_open_work" 'promotion_decision=not-promoted' 'promotion decision'
require_exact "$promotion_open_work" 'default_terminal_engine=interim-text' 'default terminal engine'
require_exact "$promotion_open_work" 'libghostty_vt_status=opt-in' 'libghostty-vt opt-in status'
require_exact "$promotion_open_work" 'open_work_scope=known blockers that must be resolved before libghostty-vt can become the default engine or a regular required CI gate' 'promotion open work scope'
require_exact "$promotion_open_work" 'open_work_ci=manual promotion evidence bundle and downloaded-artifact verifier jobs still need recorded CI runs' 'promotion open work CI gap'
require_exact "$promotion_open_work" 'open_work_platforms=more supported local systems and at least one full cold-checkout or cold-machine run still need timing evidence' 'promotion open work platform gap'
require_exact "$promotion_open_work" 'open_work_non_nix=non-Nix toolchain checklist still needs a successful platform-specific validation run' 'promotion open work non-Nix gap'
require_exact "$promotion_open_work" 'open_work_source_policy=packaged/default build source policy still needs a decision and evidence' 'promotion open work source-policy gap'
require_exact "$promotion_open_work" 'open_work_packaging=native VT binary distribution expectations still need supported-target, signing, notarization, installed-package, and platform distribution decisions despite local layout, archive, provenance, and runtime-smoke verifiers' 'promotion open work packaging gap'
require_exact "$promotion_open_work" 'open_work_frontend=frontend Ghostty renderer hydration remains separate from backend terminal-state extraction' 'promotion open work frontend gap'
printf 'promotion_evidence_verified=%s\n' "$summary"
