# Default Engine Promotion Evidence

Status: Tracking.

Last reviewed: 2026-05-23.

ADR 0023 keeps `libghostty-vt` opt-in after M13. This file is the evidence
tracker for any later decision to make `libghostty-vt` the documented default
engine or a regular CI requirement.

## Current Decision

- Default local engine: `interim`.
- Optional correctness engine: `libghostty-vt`, available in builds compiled
  with `--features libghostty-vt`.
- Regular development gate: `nix develop . -c make check`.
- Explicit combined validation gate: `nix develop . -c make check-all`.
- Promotion status: not accepted.

## Evidence Required Before Promotion

- Native build time: measured cold and warm timings for `make check-all` on the
  supported local platforms.
- CI behavior: measured full-suite runtime, cache behavior, and flake rate for
  the native Ghostty/Zig build in the intended CI environment.
- Toolchain provisioning: documented non-Nix path for Rust, FlatBuffers, make,
  and the required Zig version.
- Source-fetch policy: documented answer for `libghostty-vt-sys` source fetches,
  offline builds, vendoring, cache pinning, and local Git rewrite avoidance.
- Packaging: documented binary distribution story for the native Ghostty VT
  library on the supported targets.
- Local workflow: documented guidance for contributors who only need the
  default interim path versus contributors touching terminal correctness.
- State-sync safety: `make check-all` remains green while preserving attach,
  reconnect, live streaming, scrollback fetches, cached state, and daemon-owned
  structured input semantics.

## Current Evidence

- The Nix development shell pins the required toolchain components for the repo,
  including `flatbuffers`; on 2026-05-23, `nix develop . -c flatc --version`
  resolved FlatBuffers 25.12.19.
- [Toolchain notes](toolchain.md) document the supported Nix path and the
  minimum non-Nix equivalents for Rust, FlatBuffers, make, Zig 0.15, and
  `libghostty-vt-sys` source-fetch policy. That checklist is setup guidance,
  not promotion evidence by itself.
- No non-Nix `make check-ghostty-vt` or `make check-all` validation run has
  been recorded as promotion evidence; the Nix shell remains the only validated
  local workflow for the optional native Ghostty VT path.
- [Source fetch policy](source-fetch-policy.md) documents the current opt-in
  `libghostty-vt-sys` fetch behavior and the remaining policy choices for
  packaged/default builds. It allows local correctness work, but does not close
  the promotion blocker by itself.
- [Packaging notes](packaging.md) document the current source-checkout
  distribution path and the binary packaging questions that must be answered
  before native VT builds become default or regular CI.
- [Contributor workflow](contributor-workflow.md) documents when contributors
  should use the default gate, the opt-in terminal-correctness gate, and the
  combined promotion-evidence gate.
- `make check-ghostty-vt` runs the full `nmux-core` and `nmux-cli` package test
  suites with `--features libghostty-vt` and sets `GIT_CONFIG_GLOBAL=/dev/null`
  to avoid local Git URL rewrite interference.
- `make check-all` runs the regular default-engine gate plus the opt-in
  `libghostty-vt` gate for release-style validation.
- ADR 0018 and ADR 0023 keep the native build out of the default development
  loop until the remaining evidence in this tracker is gathered.

## Local Timing Samples

These samples are useful for trend tracking, but they do not replace CI
evidence or measurements from every supported platform.

| Date | Host | Command | Result |
| --- | --- | --- | --- |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout | `/usr/bin/time -p nix --extra-experimental-features 'nix-command flakes' develop . -c make check-all` | Passed; `real 17.01`, `user 2.83`, `sys 2.98` |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; opt-in source fetch path already available locally; feature gate used `GIT_CONFIG_GLOBAL=/dev/null` through `make check-ghostty-vt` | `/usr/bin/time -p nix --extra-experimental-features 'nix-command flakes' develop . -c make check-all` | Passed; `real 30.89`, `user 8.78`, `sys 9.51` |

## Open Work

- Measure and record `make check-all` timing on more supported local systems,
  including at least one cold-checkout or cold-cache run.
- Exercise the same gate in CI before making it a required check.
- Validate the non-Nix toolchain checklist with platform-specific setup
  commands and timings, or explicitly decide that Nix remains the only
  supported native-build workflow for now.
- Choose a source policy for packaged/default builds: pinned network fetch with
  CI/cache controls, vendored or mirrored source, `GHOSTTY_SOURCE_DIR`
  prefetching, or a native-library package/artifact cache.
- Define packaging expectations for binaries that include the native Ghostty VT
  dependency, including supported targets, static/dynamic linkage, artifact
  provenance, signing/notarization where relevant, and release checks.
- Keep contributor workflow guidance current as default-engine, opt-in
  terminal-correctness, and promotion-evidence responsibilities change.

## Promotion Rule

Do not make `libghostty-vt` the default engine, a regular CI requirement, or the
documented normal path until the open work above is resolved and a new ADR
accepts the resulting build, CI, packaging, and workflow consequences.

## Non-Goals

- This tracker does not expand the nmux protocol.
- This tracker does not cover frontend Ghostty renderer hydration; use
  [the hydration tracker](upstream/ghostty-surface-hydration.md) for that.
- This tracker does not justify copying GPL or AGPL code into nmux.
