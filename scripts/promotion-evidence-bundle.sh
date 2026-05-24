#!/usr/bin/env bash
set -euo pipefail
echo "writing local promotion evidence bundle"
bundle_dir="$PROMOTION_EVIDENCE_DIR"
run_log="$bundle_dir/RUN.log"
start_epoch="$(date -u '+%s')"
start_utc="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
rm -rf "$bundle_dir"
mkdir -p "$bundle_dir"
just promotion-local-sample > "$run_log" 2>&1
status="$?"
if [ "$status" -ne 0 ]; then
    cat "$run_log"
    echo "promotion evidence bundle failed; partial log: $run_log" >&2
    exit "$status"
fi
set -e
cp target/source-fetch-provenance/SOURCE_FETCH.txt "$bundle_dir/SOURCE_FETCH.txt"
cp target/source-fetch-offline/OFFLINE_PROBE.txt "$bundle_dir/OFFLINE_PROBE.txt"
cp target/packaging-libghostty-vt/package/PROVENANCE.txt "$bundle_dir/PACKAGE_PROVENANCE.txt"
cp target/packaging-libghostty-vt/package/CARGO_TREE.txt "$bundle_dir/CARGO_TREE.txt"
cp target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz "$bundle_dir/PACKAGE_ARCHIVE.tar.gz"
just toolchain-info > "$bundle_dir/TOOLCHAIN.txt"
runtime_smoke="$(grep -m1 '^packaged_runtime_smoke=' "$run_log" | cut -d= -f2- || true)"
if [ -z "$runtime_smoke" ]; then
    echo "missing packaged_runtime_smoke result in $run_log" >&2
    exit 1
fi
local_smoke="$(grep -m1 '^local_smoke=' "$run_log" | cut -d= -f2- || true)"
if [ -z "$local_smoke" ]; then
    echo "missing local_smoke result in $run_log" >&2
    exit 1
fi
local_smoke_reattach="$(grep -m1 '^local_smoke_reattach=' "$run_log" | cut -d= -f2- || true)"
if [ -z "$local_smoke_reattach" ]; then
    echo "missing local_smoke_reattach result in $run_log" >&2
    exit 1
fi
local_smoke_socket_recreation="$(grep -m1 '^local_smoke_socket_recreation=' "$run_log" | cut -d= -f2- || true)"
if [ -z "$local_smoke_socket_recreation" ]; then
    echo "missing local_smoke_socket_recreation result in $run_log" >&2
    exit 1
fi
local_smoke_print_context="$(grep -m1 '^local_smoke_print_context=' "$run_log" | cut -d= -f2- || true)"
if [ -z "$local_smoke_print_context" ]; then
    echo "missing local_smoke_print_context result in $run_log" >&2
    exit 1
fi
local_smoke_json_info="$(grep -m1 '^local_smoke_json_info=' "$run_log" | cut -d= -f2- || true)"
if [ -z "$local_smoke_json_info" ]; then
    echo "missing local_smoke_json_info result in $run_log" >&2
    exit 1
fi
local_smoke_ready_json="$(grep -m1 '^local_smoke_ready_json=' "$run_log" | cut -d= -f2- || true)"
if [ -z "$local_smoke_ready_json" ]; then
    echo "missing local_smoke_ready_json result in $run_log" >&2
    exit 1
fi
local_smoke_managed_start="$(grep -m1 '^local_smoke_managed_start=' "$run_log" | cut -d= -f2- || true)"
if [ -z "$local_smoke_managed_start" ]; then
    echo "missing local_smoke_managed_start result in $run_log" >&2
    exit 1
fi
check_all_real="$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^real [0-9]+([.][0-9]+)?$/ { value = $2 } END { if (value != "") print value }' "$run_log")"
check_all_user="$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^user [0-9]+([.][0-9]+)?$/ { value = $2 } END { if (value != "") print value }' "$run_log")"
check_all_sys="$(awk '/^== promotion local sample: packaging archive runtime smoke ==/ { exit } /^sys [0-9]+([.][0-9]+)?$/ { value = $2 } END { if (value != "") print value }' "$run_log")"
if [ -z "$check_all_real" ] || [ -z "$check_all_user" ] || [ -z "$check_all_sys" ]; then
    echo "missing check-all time -p result in $run_log" >&2
    exit 1
fi
hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}
archive_digest="$(hash_file "$bundle_dir/PACKAGE_ARCHIVE.tar.gz")"
printf '%s  %s\n' "$archive_digest" 'PACKAGE_ARCHIVE.tar.gz' > "$bundle_dir/ARCHIVE.sha256"
archive_sha="$(cat "$bundle_dir/ARCHIVE.sha256")"
path_status() {
    if [ -e "$1" ]; then
        printf 'present'
    else
        printf 'missing'
    fi
}
write_cache_state() {
    cache_state="$bundle_dir/CACHE_STATE.txt"
    cargo_home="${CARGO_HOME:-$HOME/.cargo}"
    cargo_target_dir="${CARGO_TARGET_DIR:-target}"
    {
        printf 'nmux promotion evidence cache state\n'
        printf 'generated_at_utc=%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
        printf 'cache_state_scope=%s\n' 'observed filesystem and environment state; does not by itself prove cold or warm cache history'
        printf 'HOME=%s\n' "${HOME:-unset}"
        printf 'CARGO_HOME=%s\n' "$cargo_home"
        printf 'CARGO_TARGET_DIR=%s\n' "$cargo_target_dir"
        printf 'GHOSTTY_SOURCE_DIR=%s\n' "${GHOSTTY_SOURCE_DIR:-unset}"
        printf 'GIT_CONFIG_GLOBAL=%s\n' "${GIT_CONFIG_GLOBAL:-unset}"
        printf 'nix_store_status=%s\n' "$(path_status /nix/store)"
        printf 'cargo_home_status=%s\n' "$(path_status "$cargo_home")"
        printf 'cargo_registry_status=%s\n' "$(path_status "$cargo_home/registry")"
        printf 'cargo_git_status=%s\n' "$(path_status "$cargo_home/git")"
        printf 'cargo_target_dir_status=%s\n' "$(path_status "$cargo_target_dir")"
        printf 'promotion_cold_target_dir_status=%s\n' "$(path_status target/promotion-cold)"
        printf 'packaging_default_target_dir_status=%s\n' "$(path_status target/packaging-default)"
        printf 'packaging_libghostty_vt_target_dir_status=%s\n' "$(path_status target/packaging-libghostty-vt)"
        printf 'source_fetch_provenance_status=%s\n' "$(path_status target/source-fetch-provenance)"
        printf 'source_fetch_offline_probe_status=%s\n' "$(path_status target/source-fetch-offline/OFFLINE_PROBE.txt)"
        printf 'cache_state_note=%s\n' 'classify cold, warm, restored, or unknown in the promotion tracker from this artifact plus CI/cache setup context'
    } > "$cache_state"
}
write_vcs_status() {
    vcs_status="$bundle_dir/VCS_STATUS.txt"
    git_status_file="$(mktemp)"
    vcs_worktree_status=unknown
    if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        if git status --porcelain=v1 > "$git_status_file" 2>/dev/null; then
            if [ -s "$git_status_file" ]; then
                vcs_worktree_status=dirty
            else
                vcs_worktree_status=clean
            fi
        fi
    fi
    {
        printf 'nmux promotion evidence VCS status\n'
        printf 'generated_at_utc=%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
        printf 'vcs_status_scope=%s\n' 'observed repository identity and working-tree state at bundle generation time'
        printf 'git_revision=%s\n' "$(git rev-parse HEAD 2>/dev/null || printf 'unknown')"
        printf 'git_status_porcelain=%s\n' "$vcs_worktree_status"
        printf '[git_status_porcelain_v1]\n'
        if [ "$vcs_worktree_status" = clean ]; then
            printf 'clean\n'
        elif [ "$vcs_worktree_status" = dirty ]; then
            cat "$git_status_file"
        else
            printf 'unknown\n'
        fi
        if command -v jj >/dev/null 2>&1; then
            printf 'jj_status_available=true\n'
            printf '[jj_status]\n'
            jj status 2>&1 || true
        else
            printf 'jj_status_available=false\n'
            printf '[jj_status]\n'
            printf 'unavailable\n'
        fi
    } > "$vcs_status"
    rm -f "$git_status_file"
}
write_promotion_open_work() {
    promotion_open_work="$bundle_dir/PROMOTION_OPEN_WORK.txt"
    {
        printf 'nmux native VT promotion open work\n'
        printf 'generated_at_utc=%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
        printf 'promotion_decision=not-promoted\n'
        printf 'default_terminal_engine=interim-text\n'
        printf 'libghostty_vt_status=opt-in\n'
        printf 'open_work_scope=%s\n' 'known blockers that must be resolved before libghostty-vt can become the default engine or a regular required CI gate'
        printf 'open_work_ci=%s\n' 'manual promotion evidence bundle and downloaded-artifact verifier jobs still need recorded CI runs'
        printf 'open_work_platforms=%s\n' 'more supported local systems and at least one full cold-checkout or cold-machine run still need timing evidence'
        printf 'open_work_non_nix=%s\n' 'non-Nix toolchain checklist still needs a successful platform-specific validation run'
        printf 'open_work_source_policy=%s\n' 'packaged/default build source policy still needs a decision and evidence'
        printf 'open_work_packaging=%s\n' 'native VT binary distribution expectations still need supported-target, signing, notarization, installed-package, and platform distribution decisions despite local layout, archive, provenance, and runtime-smoke verifiers'
        printf 'open_work_frontend=%s\n' 'frontend Ghostty renderer hydration remains separate from backend terminal-state extraction'
    } > "$promotion_open_work"
}
write_summary() {
    completed_utc="$1"
    elapsed_seconds="$2"
    {
        printf 'nmux promotion evidence bundle\n'
        printf 'generated_at_utc=%s\n' "$completed_utc"
        printf 'started_at_utc=%s\n' "$start_utc"
        printf 'completed_at_utc=%s\n' "$completed_utc"
        printf 'bundle_elapsed_seconds=%s\n' "$elapsed_seconds"
        printf 'host=%s\n' "$(uname -a)"
        printf 'git_revision=%s\n' "$(git rev-parse HEAD 2>/dev/null || printf 'unknown')"
        printf 'github_actions=%s\n' "${GITHUB_ACTIONS:-false}"
        printf 'github_server_url=%s\n' "${GITHUB_SERVER_URL:-unset}"
        printf 'github_repository=%s\n' "${GITHUB_REPOSITORY:-unset}"
        printf 'github_run_id=%s\n' "${GITHUB_RUN_ID:-unset}"
        printf 'github_run_attempt=%s\n' "${GITHUB_RUN_ATTEMPT:-unset}"
        printf 'github_ref=%s\n' "${GITHUB_REF:-unset}"
        printf 'github_sha=%s\n' "${GITHUB_SHA:-unset}"
        printf 'runner_os=%s\n' "${RUNNER_OS:-unset}"
        printf 'runner_arch=%s\n' "${RUNNER_ARCH:-unset}"
        printf 'runner_name=%s\n' "${RUNNER_NAME:-unset}"
        printf 'ghostty_source_mode=%s\n' "$([ -n "${GHOSTTY_SOURCE_DIR:-}" ] && printf 'local' || printf 'pinned-fetch')"
        printf 'GHOSTTY_SOURCE_DIR=%s\n' "${GHOSTTY_SOURCE_DIR:-unset}"
        printf 'GIT_CONFIG_GLOBAL=%s\n' "${GIT_CONFIG_GLOBAL:-unset}"
        printf 'working_tree_status=%s\n' "$vcs_worktree_status"
        printf 'vcs_status=%s\n' 'VCS_STATUS.txt'
        printf 'promotion_open_work=%s\n' 'PROMOTION_OPEN_WORK.txt'
        printf 'cache_state=%s\n' 'CACHE_STATE.txt'
        printf 'run_log=%s\n' 'RUN.log'
        printf 'toolchain=%s\n' 'TOOLCHAIN.txt'
        printf 'source_fetch=%s\n' 'SOURCE_FETCH.txt'
        printf 'source_fetch_offline_probe=%s\n' 'OFFLINE_PROBE.txt'
        printf 'package_provenance=%s\n' 'PACKAGE_PROVENANCE.txt'
        printf 'cargo_tree=%s\n' 'CARGO_TREE.txt'
        printf 'package_archive=%s\n' 'PACKAGE_ARCHIVE.tar.gz'
        printf 'check_all_real_seconds=%s\n' "$check_all_real"
        printf 'check_all_user_seconds=%s\n' "$check_all_user"
        printf 'check_all_sys_seconds=%s\n' "$check_all_sys"
        printf 'local_smoke_reattach=%s\n' "$local_smoke_reattach"
        printf 'local_smoke_socket_recreation=%s\n' "$local_smoke_socket_recreation"
        printf 'local_smoke_print_context=%s\n' "$local_smoke_print_context"
        printf 'local_smoke_json_info=%s\n' "$local_smoke_json_info"
        printf 'local_smoke_ready_json=%s\n' "$local_smoke_ready_json"
        printf 'local_smoke_managed_start=%s\n' "$local_smoke_managed_start"
        printf 'local_smoke=%s\n' "$local_smoke"
        printf 'archive_sha256=%s\n' "$archive_sha"
        printf 'packaged_runtime_smoke=%s\n' "$runtime_smoke"
    } > "$bundle_dir/SUMMARY.txt"
}
write_manifest() {
    manifest="$bundle_dir/BUNDLE_MANIFEST.txt"
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
    } > "$manifest"
}
end_epoch="$(date -u '+%s')"
completed_utc="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
write_cache_state
write_vcs_status
write_promotion_open_work
write_summary "$completed_utc" "$((end_epoch - start_epoch))"
write_manifest
PROMOTION_EVIDENCE_DIR="$bundle_dir" just promotion-evidence-verify
end_epoch="$(date -u '+%s')"
completed_utc="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
write_cache_state
write_vcs_status
write_promotion_open_work
write_summary "$completed_utc" "$((end_epoch - start_epoch))"
write_manifest
PROMOTION_EVIDENCE_DIR="$bundle_dir" just promotion-evidence-verify
cat "$run_log"
printf 'promotion_evidence_bundle=%s\n' "$bundle_dir"
find "$bundle_dir" -type f | sort
