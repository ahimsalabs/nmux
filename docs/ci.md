# CI Notes

The repository has a GitHub Actions workflow at
[.github/workflows/check.yml](../.github/workflows/check.yml).

## Required Default Gate

Pull requests and pushes to `main` run the default-engine gate:

```sh
nix build ".#checks.$(nix eval --impure --raw --expr builtins.currentSystem).source-audit" --no-link --print-out-paths
nix develop . -c make toolchain-info
nix develop . -c make check
nix develop . -c make local-smoke
```

The source audit checks that the flake source excludes build output and VCS
metadata before CI enters the development shell. The remaining steps keep
regular CI on the default `interim` engine. A green required CI run does not
claim `libghostty-vt` correctness and does not change the default engine
decision. `make local-smoke` adds a real default-engine daemon/client
workflow check: it starts `nmuxd`, sends live stdin through `nmux`, persists
client state, verifies a sequential read-only reattach sees the output, verifies
nested `nmux --print-context` sees the pane identity environment, then reuses
the same socket path for a new daemon and verifies the old cached surface is
not rendered.

For ordinary implementation or documentation pushes, record the GitHub Actions
run ID or URL after pushing and let the run complete asynchronously unless the
task specifically requires CI completion as evidence. Only promotion evidence
runs need the full transcription and artifact-verification treatment below.

## Manual Native VT Check

The same workflow exposes a direct manual `workflow_dispatch` job for the
opt-in native VT correctness gate:

```sh
nix develop . -c make check-ghostty-vt
```

This job is intentionally manual and does not make `libghostty-vt` a regular
or required CI gate.

## Manual Promotion Evidence

The same workflow exposes a manual `workflow_dispatch` job for promotion
evidence:

```sh
nix develop . -c make promotion-evidence-bundle
```

That job is intentionally manual. It records source-fetch provenance, runs the
default gate plus the opt-in `libghostty-vt` gate, times the inner
`make check-all` run, and produces a verifiable native-VT package archive. The
bundle target gathers the same local promotion evidence under
`target/promotion-evidence` and runs `make promotion-evidence-verify` before
upload. The workflow uploads that directory as the `nmux-promotion-evidence`
artifact, then a dependent job downloads the uploaded artifact into
`target/downloaded-promotion-evidence` and runs
`make PROMOTION_EVIDENCE_DIR=target/downloaded-promotion-evidence promotion-evidence-verify`.
Copy passing or failing results into
[default-engine-promotion.md](default-engine-promotion.md) with runner, cache,
source-fetch, packaging, artifact round-trip, and flake context before using
them as promotion evidence.

After the first passing manual sample, keep later runs comparable instead of
treating the initial pass as promotion by itself. Record whether each run used
fresh, warm, restored, or unknown Nix/Cargo/native build caches; note any retry
or native-VT flake; capture GitHub Actions warnings that could affect the
evidence path; and add follow-up items for workflow maintenance such as hosted
runner or action-runtime changes.

Record each manual run with these fields:

| Field | Required Content |
| --- | --- |
| Date | UTC date of the workflow run. |
| Workflow run | GitHub Actions run URL or run number from `SUMMARY.txt`. |
| Git revision | Commit SHA and branch or pull request ref from `SUMMARY.txt`. |
| Runner | Runner OS, architecture, image label, and hosted/self-hosted status from `SUMMARY.txt`. |
| Toolchain | `make toolchain-info` output from the run. |
| Source mode | `ghostty_source_mode`, `GHOSTTY_SOURCE_DIR`, and `GIT_CONFIG_GLOBAL`. |
| VCS status | `VCS_STATUS.txt`, `git_revision`, `working_tree_status`, and any `jj status` output recorded by the bundle. |
| Open work | `PROMOTION_OPEN_WORK.txt`, including the promotion decision status and remaining CI, platform, non-Nix, source-policy, packaging, and frontend-hydration blockers. |
| Cache state | `CACHE_STATE.txt` plus CI cache setup context; classify whether Nix, Cargo registry, Cargo Git, Rust target, and native Ghostty/Zig build caches were cold, warm, restored, or unknown. |
| Timings and local smoke | `check_all_real_seconds`, `check_all_user_seconds`, `check_all_sys_seconds`, `local_smoke`, `local_smoke_reattach`, `local_smoke_print_context`, `local_smoke_json_info`, `local_smoke_ready_json`, `local_smoke_socket_recreation`, `bundle_elapsed_seconds`, and total GitHub job duration. |
| Provenance | `nmux-promotion-evidence` artifact, `SOURCE_FETCH.txt`, `OFFLINE_PROBE.txt`, `Cargo.lock` hash, locked `libghostty-vt`/`libghostty-vt-sys` records, and whether the cache-present offline probe passed. |
| Packaging | Archive name, SHA-256, package metadata, `packaging-provenance-manifest-verify` result against bundled provenance, `packaging-archive-verify` result, `packaging-provenance-verify` run-log result, relocated install root, clean library-path environment, and packaged runtime smoke result. |
| Artifact round-trip | Whether the dependent artifact-verify job downloaded `nmux-promotion-evidence` and passed `make promotion-evidence-verify` against the downloaded copy. |
| Outcome | Passed, failed, or canceled, including failed command and error summary. |
| Follow-up | Any flake, cache miss, source-fetch, packaging, or platform issue created from the run. |

Use `make promotion-evidence-verify` on a locally generated artifact directory
before transcribing it into the promotion tracker. For a downloaded artifact
that is not under `target/promotion-evidence`, run
`make PROMOTION_EVIDENCE_DIR=/path/to/artifact promotion-evidence-verify`. The
verifier checks the required summary identity fields, timing fields,
`local_smoke`, `local_smoke_reattach`, `local_smoke_print_context`,
`local_smoke_json_info`, `local_smoke_ready_json`, and
`local_smoke_socket_recreation` results,
bundle-relative summary artifact names, cache-state artifact, relocation-safe
`BUNDLE_MANIFEST.txt` hashes, VCS status artifact, summary/VCS git revision
agreement, open-work snapshot, source/provenance records, cache-present offline
probe result, bundled package provenance through
`packaging-provenance-manifest-verify`, package archive bytes,
bundle-relative archive hash, and packaged runtime smoke result. When the bundle
reports `github_actions=true`, the
verifier also requires non-placeholder GitHub run, ref, SHA, and runner fields
and checks that `github_sha` matches the bundled git revision. It does not
replace human judgment about cache classification, flake rate, or platform
coverage.

The bundle summary records GitHub Actions fields when present:
`github_server_url`, `github_repository`, `github_run_id`,
`github_run_attempt`, `github_ref`, `github_sha`, `runner_os`,
`runner_arch`, and `runner_name`; CI-generated bundles must have concrete
values for those fields. It also records extracted
`time -p make check-all` values as `check_all_real_seconds`,
`check_all_user_seconds`, and `check_all_sys_seconds`, plus
`started_at_utc`, `completed_at_utc`, and `bundle_elapsed_seconds` for the
bundle artifact generation and verifier pass before final console output.
`VCS_STATUS.txt` records the Git revision, Git working-tree status, and
optional `jj status` output observed at bundle generation time.
`PROMOTION_OPEN_WORK.txt` records the known blockers that still keep native VT
promotion out of the default engine and regular required CI path.
`CACHE_STATE.txt` records observed cache-related environment values and
directory presence; use it with the workflow cache configuration when
classifying the run as cold, warm, restored, or unknown.
Local runs record GitHub identity fields as `unset`; the overall GitHub job
duration still comes from the workflow UI or API.

Before this manual job can become a required or regular native-VT CI gate, a
later ADR must satisfy
[ADR 0026](adr/0026-native-vt-ci-promotion-criteria.md), including runner
matrix, cache behavior, source-fetch behavior, packaging claims, runtime, and
failure-triage expectations.

## Action Runtime Maintenance

The 2026-05-23 manual promotion run
[`26334374094`](https://github.com/ahimsalabs/nmux/actions/runs/26334374094)
passed, but GitHub Actions emitted Node.js 20 deprecation warnings for the
current JavaScript actions. The affected workflow actions were
`actions/checkout@v4` in every job, `actions/upload-artifact@v4` in the
promotion bundle job, and `actions/download-artifact@v4` in the artifact
verification job.

The warning says GitHub will force JavaScript actions to Node.js 24 by default
starting June 2, 2026, and remove Node.js 20 from runners on September 16,
2026. Before treating the manual promotion evidence workflow as stable enough
for required native-VT CI, run at least one clean promotion sample after the
action-runtime transition or update the workflow to action versions/settings
that explicitly support Node.js 24 and record that evidence in
[default-engine-promotion.md](default-engine-promotion.md).

## Nix Setup

CI installs Nix with `cachix/install-nix-action@v31`, which the action README
documents for Linux and macOS runners. The workflow does not configure a
project binary cache yet; cache behavior remains explicit open promotion work.
