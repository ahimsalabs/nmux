# ADR 0026: Native VT CI Promotion Criteria

## Status

Accepted.

## Date

2026-05-23

## Context

ADR 0018 keeps `libghostty-vt` out of regular `make check` and default CI while
still requiring a full opt-in `make check-ghostty-vt` gate for feature-sensitive
changes. ADR 0023 keeps the post-M13 default engine as `interim` until build,
CI, source-fetch, packaging, and workflow costs are accepted deliberately.

The repository now has a GitHub Actions default-engine job for pull requests and
pushes to `main`, plus a manual `workflow_dispatch` promotion evidence job that
runs `make promotion-evidence-bundle`. That manual job is useful evidence, but
a regular or required native-VT CI gate would have different consequences:
network fetch behavior, cache misses, Zig/native build provisioning, runner
cost, artifact provenance, and failure triage become part of every protected
change.

## Decision

A future ADR that makes the native VT path a regular or required CI gate must
prove these criteria:

- The required and optional CI jobs are named explicitly, including which branch
  and pull-request events run each job.
- The required gate remains clear about whether it proves default-engine
  behavior, native-VT correctness, packaging readiness, or release readiness.
- CI records `make toolchain-info`, source mode, `GHOSTTY_SOURCE_DIR`,
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
- The cost of making the native VT job required is accepted for normal
  contributors, including expected runtime and cache behavior.
- The default `interim` engine remains independently tested unless a later ADR
  also changes the default engine decision.

Until a later ADR satisfies those criteria and updates the workflow, the GitHub
Actions required path remains default-engine `make check` plus `make
local-smoke`, and the native-VT promotion job remains manual evidence
collection.

## Consequences

CI promotion work can proceed by adding evidence rows and improving the manual
job without accidentally changing repository protection expectations. A green
default-engine CI run still does not claim native-VT correctness, and a green
manual promotion run still does not by itself make native VT required.

If the project later chooses to make native VT required, this ADR requires the
change to include workflow updates, docs updates, recorded runner evidence, and
clear failure ownership.

## Licensing And Compatibility

This decision does not change licensing posture. CI must not copy GPL or AGPL
code into nmux, and native-VT jobs must continue treating generated Ghostty
build output under `target/` as build output, not repository source material.
