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
upload; the workflow uploads that directory as the `nmux-promotion-evidence`
artifact. Copy passing or failing results into
[default-engine-promotion.md](default-engine-promotion.md) with runner, cache,
source-fetch, packaging, and flake context before using them as promotion
evidence.

Record each manual run with these fields:

| Field | Required Content |
| --- | --- |
| Date | UTC date of the workflow run. |
| Workflow run | GitHub Actions run URL or run number from `SUMMARY.txt`. |
| Git revision | Commit SHA and branch or pull request ref from `SUMMARY.txt`. |
| Runner | Runner OS, architecture, image label, and hosted/self-hosted status from `SUMMARY.txt`. |
| Toolchain | `make toolchain-info` output from the run. |
| Source mode | `ghostty_source_mode`, `GHOSTTY_SOURCE_DIR`, and `GIT_CONFIG_GLOBAL`. |
| Cache state | Whether Nix, Cargo registry, Cargo Git, Rust target, and native Ghostty/Zig build caches were cold, warm, restored, or unknown. |
| Timings | `time -p make check-all` output from `promotion-sample` and total job duration. |
| Provenance | `nmux-promotion-evidence` artifact, `SOURCE_FETCH.txt`, `Cargo.lock` hash, and locked `libghostty-vt`/`libghostty-vt-sys` records. |
| Packaging | Archive name, SHA-256, `packaging-provenance-verify` result, and packaged runtime smoke result. |
| Outcome | Passed, failed, or canceled, including failed command and error summary. |
| Follow-up | Any flake, cache miss, source-fetch, packaging, or platform issue created from the run. |

Use `make promotion-evidence-verify` on a locally generated artifact directory
before transcribing it into the promotion tracker. For a downloaded artifact
that is not under `target/promotion-evidence`, run
`make PROMOTION_EVIDENCE_DIR=/path/to/artifact promotion-evidence-verify`. The
verifier checks the required summary identity fields, artifact files,
source/provenance records, archive hash, and packaged runtime smoke result; it
does not replace human judgment about cache state, flake rate, or platform
coverage.

The bundle summary records GitHub Actions fields when present:
`github_server_url`, `github_repository`, `github_run_id`,
`github_run_attempt`, `github_ref`, `github_sha`, `runner_os`,
`runner_arch`, and `runner_name`. Local runs record those fields as `unset`.

Before this manual job can become a required or regular native-VT CI gate, a
later ADR must satisfy
[ADR 0026](adr/0026-native-vt-ci-promotion-criteria.md), including runner
matrix, cache behavior, source-fetch behavior, packaging claims, runtime, and
failure-triage expectations.

## Nix Setup

CI installs Nix with `cachix/install-nix-action@v31`, which the action README
documents for Linux and macOS runners. The workflow does not configure a
project binary cache yet; cache behavior remains explicit open promotion work.
