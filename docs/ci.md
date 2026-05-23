# CI Notes

The repository has a GitHub Actions workflow at
[.github/workflows/check.yml](../.github/workflows/check.yml).

## Required Default Gate

Pull requests and pushes to `main` run the default-engine gate:

```sh
nix develop . -c make toolchain-info
nix develop . -c make check
```

This keeps regular CI on the default `interim` engine. A green required CI run
does not claim `libghostty-vt` correctness and does not change the default
engine decision.

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

Record each manual run with these fields:

| Field | Required Content |
| --- | --- |
| Date | UTC date of the workflow run. |
| Workflow run | GitHub Actions run URL or run number from `SUMMARY.txt`. |
| Git revision | Commit SHA and branch or pull request ref from `SUMMARY.txt`. |
| Runner | Runner OS, architecture, image label, and hosted/self-hosted status from `SUMMARY.txt`. |
| Toolchain | `make toolchain-info` output from the run. |
| Source mode | `ghostty_source_mode`, `GHOSTTY_SOURCE_DIR`, and `GIT_CONFIG_GLOBAL`. |
| Cache state | `CACHE_STATE.txt` plus CI cache setup context; classify whether Nix, Cargo registry, Cargo Git, Rust target, and native Ghostty/Zig build caches were cold, warm, restored, or unknown. |
| Timings | `check_all_real_seconds`, `check_all_user_seconds`, `check_all_sys_seconds`, `bundle_elapsed_seconds`, and total GitHub job duration. |
| Provenance | `nmux-promotion-evidence` artifact, `SOURCE_FETCH.txt`, `OFFLINE_PROBE.txt`, `Cargo.lock` hash, locked `libghostty-vt`/`libghostty-vt-sys` records, and whether the cache-present offline probe passed. |
| Packaging | Archive name, SHA-256, package metadata, `packaging-archive-verify` result, `packaging-provenance-verify` result, relocated install root, clean library-path environment, and packaged runtime smoke result. |
| Artifact round-trip | Whether the dependent artifact-verify job downloaded `nmux-promotion-evidence` and passed `make promotion-evidence-verify` against the downloaded copy. |
| Outcome | Passed, failed, or canceled, including failed command and error summary. |
| Follow-up | Any flake, cache miss, source-fetch, packaging, or platform issue created from the run. |

Use `make promotion-evidence-verify` on a locally generated artifact directory
before transcribing it into the promotion tracker. For a downloaded artifact
that is not under `target/promotion-evidence`, run
`make PROMOTION_EVIDENCE_DIR=/path/to/artifact promotion-evidence-verify`. The
verifier checks the required summary identity fields, timing fields,
bundle-relative summary artifact names, cache-state artifact, relocation-safe
`BUNDLE_MANIFEST.txt` hashes, source/provenance records, cache-present offline
probe result, package archive bytes, bundle-relative archive hash, and packaged
runtime smoke result; it does not replace human judgment about cache
classification, flake rate, or platform coverage.

The bundle summary records GitHub Actions fields when present:
`github_server_url`, `github_repository`, `github_run_id`,
`github_run_attempt`, `github_ref`, `github_sha`, `runner_os`,
`runner_arch`, and `runner_name`. It also records extracted
`time -p make check-all` values as `check_all_real_seconds`,
`check_all_user_seconds`, and `check_all_sys_seconds`, plus
`started_at_utc`, `completed_at_utc`, and `bundle_elapsed_seconds` for the
bundle artifact generation and verifier pass before final console output.
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

## Nix Setup

CI installs Nix with `cachix/install-nix-action@v31`, which the action README
documents for Linux and macOS runners. The workflow does not configure a
project binary cache yet; cache behavior remains explicit open promotion work.
