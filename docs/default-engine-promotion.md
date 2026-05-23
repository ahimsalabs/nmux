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
  including `flatbuffers`; on 2026-05-23,
  `nix --extra-experimental-features 'nix-command flakes' develop . -c flatc --version`
  resolved FlatBuffers 25.12.19.
- [Toolchain notes](toolchain.md) document the supported Nix path and the
  minimum non-Nix equivalents for Rust, FlatBuffers, make, Zig 0.15, and
  `libghostty-vt-sys` source-fetch policy. That checklist is setup guidance,
  not promotion evidence by itself.
- No non-Nix `make check-ghostty-vt` or `make check-all` validation run has
  passed as promotion evidence. A recorded local non-Nix `make check-all`
  attempt failed before tests because `flatc` was absent from the host PATH, so
  the Nix shell remains the only validated local workflow for the optional
  native Ghostty VT path.
- [Source fetch policy](source-fetch-policy.md) documents the current opt-in
  `libghostty-vt-sys` fetch behavior and the remaining policy choices for
  packaged/default builds. [ADR 0024](adr/0024-native-vt-source-policy-criteria.md)
  defines the criteria a later source-policy promotion decision must satisfy.
  This allows local correctness work, but does not close the promotion blocker
  by itself.
- [Packaging notes](packaging.md) document the current source-checkout
  distribution path and the binary packaging questions that must be answered
  before native VT builds become default or regular CI. [ADR 0025](adr/0025-native-vt-packaging-criteria.md)
  defines the criteria a later packaging promotion decision must satisfy.
- [Contributor workflow](contributor-workflow.md) documents when contributors
  should use the default gate, the opt-in terminal-correctness gate, and the
  combined promotion-evidence gate.
- [CI notes](ci.md) document the required GitHub Actions default-engine gate and
  the manual promotion-local-sample job.
- [ADR 0026](adr/0026-native-vt-ci-promotion-criteria.md) defines the criteria
  a later decision must satisfy before native VT becomes regular or required CI.
- The Makefile performs local tool preflight checks for `cargo`, `flatc`
  25.12.19, and the optional native-VT Zig 0.15.x requirement so non-Nix
  validation attempts fail with setup guidance instead of an opaque
  missing-command or wrong-version error.
- `make toolchain-info` prints the active Rust, FlatBuffers, Zig, and
  source-fetch environment fields that should accompany promotion-evidence
  samples.
- `make promotion-sample` prints that toolchain information and then times
  `make check-all` with `time -p` for a single local evidence command.
- `make promotion-cold-target-sample` clears `target/promotion-cold` and times
  `make check-all` with that fresh Rust target directory. It does not clear
  Cargo registry, Git source, or Nix store caches.
- `make promotion-local-sample` runs source-fetch provenance, the timed
  validation sample, and package archive runtime smoke in one local evidence
  pass.
- `make source-fetch-provenance-sample` writes the active source mode and
  locked `libghostty-vt` Cargo package records without inspecting Ghostty
  source.
- `make packaging-sample` prints that toolchain information, builds default and
  opt-in `libghostty-vt` release binaries in separate target directories, and
  reports artifact sizes plus binary versions for packaging evidence.
- `make packaging-layout-sample` stages opt-in release binaries, wrapper
  scripts, and `libghostty-vt` runtime-library artifacts in a local package
  layout and verifies the wrapped binaries run from that layout.
- `make packaging-provenance-sample` writes a provenance manifest for the
  staged layout, including file hashes, toolchain/source mode, locked
  `libghostty-vt` package records, dependency tree, native runtime-library
  artifacts, and dynamic dependency output.
- `make packaging-archive-sample` writes a tar archive and SHA-256 file for the
  staged layout, extracts it, and verifies the wrapped binaries from the
  extracted archive.
- `make packaging-archive-runtime-smoke` starts the extracted opt-in
  `libghostty-vt` daemon and attaches the extracted client to prove the packaged
  runtime layout can serve a real pane.
- The optional VT preflight rejects an invalid `GHOSTTY_SOURCE_DIR` before the
  native build starts, while unset `GHOSTTY_SOURCE_DIR` is recorded as the
  pinned-fetch source mode.
- `make check-ghostty-vt` runs the full `nmux-core` and `nmux-cli` package test
  suites with `--features libghostty-vt` and sets `GIT_CONFIG_GLOBAL=/dev/null`
  to avoid local Git URL rewrite interference.
- `make check-all` runs the regular default-engine gate plus the opt-in
  `libghostty-vt` gate for release-style validation.
- Multiple warm local Darwin arm64 `make check-all` samples are recorded below,
  including a post-info-flag run; these are useful trend evidence, not CI
  promotion evidence. A cold-target sample clears only the Rust target
  directory and is not full cold-checkout evidence.
- A GitHub Actions workflow now runs `make check` for pull requests and pushes
  to `main`; the `make promotion-local-sample` job is manual and must be run
  before any CI promotion evidence is recorded here.
- ADR 0018 and ADR 0023 keep the native build out of the default development
  loop until the remaining evidence in this tracker is gathered.

## Nix Toolchain Checks

These checks prove the supported Nix shell provisions required tools. They are
setup evidence, not enough by themselves to promote `libghostty-vt`.

| Date | Host | Command | Result |
| --- | --- | --- | --- |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM | `nix --extra-experimental-features 'nix-command flakes' develop . -c flatc --version` | Passed; `flatc version 25.12.19` |

## Local Timing Samples

These samples are useful for trend tracking, but they do not replace CI
evidence or measurements from every supported platform.

| Date | Host | Command | Result |
| --- | --- | --- | --- |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout | `/usr/bin/time -p nix --extra-experimental-features 'nix-command flakes' develop . -c make check-all` | Passed; `real 17.01`, `user 2.83`, `sys 2.98` |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; opt-in source fetch path already available locally; feature gate used `GIT_CONFIG_GLOBAL=/dev/null` through `make check-ghostty-vt` | `/usr/bin/time -p nix --extra-experimental-features 'nix-command flakes' develop . -c make check-all` | Passed; `real 30.89`, `user 8.78`, `sys 9.51` |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout after local usability/info-flag changes; existing Nix/Cargo/native build caches; opt-in source fetch path already available locally; feature gate used `GIT_CONFIG_GLOBAL=/dev/null` through `make check-ghostty-vt` | `/usr/bin/time -p nix --extra-experimental-features 'nix-command flakes' develop . -c make check-all` | Passed; `real 34.42`, `user 10.95`, `sys 11.09` |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout after promotion-sample target addition; existing Nix/Cargo/native build caches; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset`; feature gate used `GIT_CONFIG_GLOBAL=/dev/null` through `make check-ghostty-vt` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-sample` | Passed; timed inner `make check-all`: `real 16.13`, `user 2.64`, `sys 3.20` |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout with existing Cargo registry, Git source, and Nix store caches; `target/promotion-cold` removed before the run; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset`; feature gate used `GIT_CONFIG_GLOBAL=/dev/null` through `make check-ghostty-vt` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-cold-target-sample` | Passed; cleared `target/promotion-cold` and timed inner `CARGO_TARGET_DIR=target/promotion-cold make check-all`: `real 39.67`, `user 50.58`, `sys 13.85`. This is cold Rust target-dir evidence, not a full cold checkout or dependency-fetch run. |

## Non-Nix Local Attempts

These attempts validate the documented non-Nix checklist. Failed setup attempts
are not promotion evidence for the optional native VT path, but they identify
the missing host requirements needed before a complete non-Nix timing sample can
be recorded.

| Date | Host | Command | Result |
| --- | --- | --- | --- |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, repository caches present; host shell outside Nix; no `flatc` on PATH | `/usr/bin/time -p env GIT_CONFIG_GLOBAL=/dev/null make check-all` | Failed during tool preflight before schema or Rust tests; missing `flatc`; `real 0.01`, `user 0.00`, `sys 0.00` |

## Source-Fetch Provenance Samples

These samples record local source-fetch inputs. They do not choose the source
policy for default or packaged builds.

| Date | Host | Command | Result |
| --- | --- | --- | --- |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make source-fetch-provenance-sample` | Passed. Wrote `target/source-fetch-provenance/SOURCE_FETCH.txt` with `Cargo.lock` SHA-256 `2e28c9036cf76971ebefb3f43a39bf3f3c122aeac89981cbf666105eb20d88c4`, `libghostty-vt` 0.1.1 checksum `d8afe5cc9ae303133220e530b28b7addbbf591160bb1564b88f7ee61387fee74`, and `libghostty-vt-sys` 0.1.1 checksum `aee97068da1692162c4523d54843bdcb43fecf086a9ee412a3375817e433faca`. |

## Packaging Samples

These samples prove release binary build behavior in a specific environment.
They do not answer install paths, signing/notarization, update channels, target
support, or native-library provenance by themselves.

| Date | Host | Command | Result |
| --- | --- | --- | --- |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-sample` | Passed. Default binaries ran `--version`; sizes: `nmux` 1065296 bytes, `nmuxd` 1182352 bytes. Opt-in `libghostty-vt` binaries built; sizes: `nmux` 1065424 bytes, `nmuxd` 1251648 bytes; `--version` passed with `DYLD_LIBRARY_PATH`/`LD_LIBRARY_PATH` set to the produced `ghostty-install/lib` runtime-library directory. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; same toolchain/source mode as packaging sample above | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-layout-sample` | Passed. Staged opt-in package layout at `target/packaging-libghostty-vt/package` with `bin/nmux`, `bin/nmuxd`, `libexec/nmux`, `libexec/nmuxd`, and `lib/libghostty-vt*`; wrapped `nmux --version` and `nmuxd --version` both reported 0.1.0 from the staged layout. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; same toolchain/source mode as packaging sample above | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-provenance-sample` | Passed. Wrote `target/packaging-libghostty-vt/package/PROVENANCE.txt` with toolchain/source mode, `Cargo.lock` SHA-256 `2e28c9036cf76971ebefb3f43a39bf3f3c122aeac89981cbf666105eb20d88c4`, staged file sizes and hashes, native runtime-library artifacts, `otool -L` output, and locked dependency tree. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; same toolchain/source mode as packaging sample above | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-archive-sample` | Passed. Wrote `target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz`, SHA-256 `1f824cdc7634e5f85e092d72fe20635cce8be834bbae33c000ac3a522fba8bdd`, extracted it under `target/packaging-libghostty-vt/archive/check`, and verified wrapped `nmux --version` and `nmuxd --version` from the extracted layout. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; same toolchain/source mode as packaging sample above; runtime smoke socket allocated under `/tmp` to keep Unix socket path below platform limits | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-archive-runtime-smoke` | Passed. Wrote and extracted `target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz`, SHA-256 `56dab0d0af12a744dff48f79108a804d768592b90e4dc7aa40ad9542edd227f0`, verified wrapped binary versions, started wrapped `nmuxd --terminal-engine libghostty-vt --one-shot`, attached wrapped `nmux`, and observed sentinel output `packaged-runtime-smoke` from the packaged daemon. |

## Local Combined Samples

These samples run source-fetch provenance, validation, and archive packaging
runtime evidence together. They are useful before updating separate provenance,
timing, and packaging rows, but they do not replace CI, cold-cache, or
multi-platform evidence.

| Date | Host | Command | Result |
| --- | --- | --- | --- |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-local-sample` | Passed. Wrote `target/source-fetch-provenance/SOURCE_FETCH.txt` with pinned-fetch source mode and locked `libghostty-vt` package records; timed inner `make check-all`: `real 16.42`, `user 2.63`, `sys 3.17`; then wrote and verified `target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz` with SHA-256 `778306572191745af4da969657daf2e3c5ccf548098ecec18fff6e9b66e0cf56`, verified wrapped binary versions, started wrapped `nmuxd --terminal-engine libghostty-vt --one-shot`, attached wrapped `nmux`, and observed sentinel output `packaged-runtime-smoke` from the packaged daemon. |

## Open Work

- Measure and record `make check-all` timing on more supported local systems,
  including a full cold-checkout or dependency-fetch run. Current cold-target
  evidence clears only `target/promotion-cold`.
- Exercise the manual promotion-local-sample job in CI before making the opt-in VT
  gate required.
- Validate the non-Nix toolchain checklist with platform-specific setup
  commands, `make promotion-sample` output, and timings; the current local
  non-Nix attempt failed before tests because `flatc` was absent from the host
  PATH outside the Nix shell, while the supported Nix shell has already
  provisioned the required `flatc`.
- Choose a source policy for packaged/default builds: pinned network fetch with
  CI/cache controls, vendored or mirrored source, `GHOSTTY_SOURCE_DIR`
  prefetching, or a native-library package/artifact cache.
- Define packaging expectations for binaries that include the native Ghostty VT
  dependency, including supported targets, static/dynamic linkage, artifact
  provenance, signing/notarization where relevant, release checks, and recorded
  `make packaging-sample`, `make packaging-layout-sample`,
  `make packaging-provenance-sample`, `make packaging-archive-sample`, and
  `make packaging-archive-runtime-smoke` results. The current Darwin packaging
  samples show the opt-in release binaries can run with an explicit runtime
  library path and staged wrapper layout, but packaged binaries still need
  signing and platform distribution strategy.
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
