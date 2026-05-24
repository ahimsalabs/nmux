# ADR 0026: Native VT CI Promotion Criteria

## Status

Accepted.

## Date

2026-05-23

## Context

ADR 0034 makes `libghostty-vt` the default engine and default CI path. The
native-VT feature suites run with `RUST_TEST_THREADS=1`, and the interim engine
is covered separately with `--no-default-features`.

The repository now has a GitHub Actions default-engine job for pull requests and
pushes to `main`, plus a manual `workflow_dispatch` promotion evidence job that
runs `just promotion-evidence-bundle`. That manual job remains useful release
evidence, but the regular default gate now owns native build provisioning,
package smoke, source mode reporting, and failure triage.

## Decision

The native VT path is a regular CI gate. Maintain these criteria:

- The required and optional CI jobs are named explicitly, including which branch
  and pull-request events run each job.
- The required gate remains clear about whether it proves default-engine
  behavior, native-VT correctness, packaging readiness, or release readiness.
- CI records `just toolchain-info`, source mode, `GHOSTTY_SOURCE_DIR`,
  `GIT_CONFIG_GLOBAL`, runner OS/image, and cache state for native-VT jobs.
- Native build timing is recorded for cache-hit and cache-miss runs on every
  required runner class.
- Source-fetch behavior satisfies ADR 0024, including cache-miss and network
  failure behavior.
- Packaging behavior satisfies ADR 0025 before any CI job claims packaged
  binary readiness.
- Failure triage is documented for toolchain preflight failures, source-fetch
  failures, native build failures, test failures, packaging failures, and
  runtime-smoke failures.
- The job matrix is explicit about supported operating systems and whether
  unsupported platforms are skipped, allowed to fail, or out of scope.
- The cost of the native VT job remains accepted for normal contributors,
  including expected runtime and cache behavior.
- The `interim` engine remains independently tested as the explicit fallback.

The GitHub Actions required path is default-engine `just check` plus `just
local-smoke`, with a separate interim fallback job. The promotion job remains
manual release evidence collection.

## Consequences

CI promotion work can proceed by adding evidence rows and improving the manual
job without changing repository protection expectations. Changes to native VT
CI should keep workflow updates, docs updates, recorded runner evidence, and
clear failure ownership together.

## Licensing And Compatibility

This decision does not change licensing posture. CI must not copy GPL or AGPL
code into nmux, and native-VT jobs must continue treating generated Ghostty
build output under `target/` as build output, not repository source material.
