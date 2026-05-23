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
- [CI notes](ci.md) document the required GitHub Actions default-engine gate,
  the manual promotion evidence bundle job, the uploaded and downloaded
  `nmux-promotion-evidence` artifact verification path, and the fields required
  when recording CI promotion evidence here.
- [ADR 0026](adr/0026-native-vt-ci-promotion-criteria.md) defines the criteria
  a later decision must satisfy before native VT becomes regular or required CI.
- The Makefile performs local tool preflight checks for `cargo`, `flatc`
  25.12.19, and the optional native-VT Zig 0.15.x requirement so non-Nix
  validation attempts fail with setup guidance instead of an opaque
  missing-command or wrong-version error.
- `make toolchain-info` prints the active Rust, FlatBuffers, Zig, and
  source-fetch environment fields that should accompany promotion-evidence
  samples.
- `make promotion-evidence-verify` checks that `SUMMARY.txt` and
  `VCS_STATUS.txt` agree on the git revision, and CI-generated bundles must
  include concrete GitHub run, ref, SHA, and runner fields with `github_sha`
  matching the bundled revision.
- `make promotion-sample` prints that toolchain information and then times
  `make check-all` with `time -p` for a single local evidence command.
- `make promotion-cold-target-sample` clears `target/promotion-cold` and times
  `make check-all` with that fresh Rust target directory. It does not clear
  Cargo registry, Git source, or Nix store caches.
- `make promotion-cold-deps-sample` clears `target/promotion-cold-deps` and
  times `make check-all` with isolated repo-owned `CARGO_HOME` and
  `CARGO_TARGET_DIR`, `GHOSTTY_SOURCE_DIR` unset, and
  `GIT_CONFIG_GLOBAL=/dev/null`. It is dependency/source-fetch evidence, not
  full cold machine evidence because the Nix store, checkout, and network state
  may still be warm.
- `make promotion-cold-deps-verify` validates an existing cold-deps
  `REPORT.txt` and `RUN.log` for the expected isolation fields, timing fields,
  passing result, and default plus opt-in test commands without rerunning the
  isolated dependency/source-fetch sample.
- `make promotion-local-sample` runs source-fetch provenance, the timed
  validation sample, the cache-present offline source-fetch probe, and package
  archive runtime smoke in one local evidence pass.
- `make promotion-evidence-bundle` runs the local sample and gathers its log,
  toolchain output, bundle start/completion timestamps plus elapsed duration,
  extracted `make check-all` timing, source-fetch report, offline probe report,
  package provenance, cargo tree, package archive, archive checksum, observed
  cache-state report, VCS status report, open-work snapshot, and bundle
  artifact manifest under `target/promotion-evidence`. The bundled
  `ARCHIVE.sha256` uses the bundle-relative `PACKAGE_ARCHIVE.tar.gz` path.
- `make promotion-evidence-verify` checks an existing bundle for required
  summary identity fields, bundle timing fields, `make check-all` timing
  fields, artifact files, bundle-relative summary artifact names, cache-state
  artifact, VCS status artifact, open-work snapshot, relocation-safe
  `BUNDLE_MANIFEST.txt` hashes, source/provenance records, cache-present
  offline probe result, package archive bytes, archive hash,
  `packaging-archive-verify` output,
  `packaging-provenance-verify` output, packaged runtime smoke result, exact
  current open-work blocker lines, run-log evidence that source-fetch
  provenance verification ran, and run-log evidence that the cache-present
  offline probe compiled the opt-in native-VT test binary and ran its verifier.
  The bundle target runs it before printing the artifact list.
- `make source-fetch-provenance-sample` writes the active source mode and
  locked `libghostty-vt` Cargo package records without inspecting Ghostty
  source.
- `make source-fetch-provenance-verify` validates an existing source-fetch
  provenance report against the current `Cargo.lock`, toolchain records,
  source-mode fields, and policy note without regenerating the report. Override
  `SOURCE_FETCH_REPORT` when checking a copied or bundled report.
- `make source-fetch-offline-probe` checks whether the opt-in
  `nmux-core --features libghostty-vt` build can compile from current caches
  with `CARGO_NET_OFFLINE=true`; this is cache-present evidence only.
- `make source-fetch-offline-probe-verify` validates an existing offline probe
  report and log for the expected offline mode, target dir, command, passing
  result, and generated `nmux-core` test binary without rerunning the probe.
- `make packaging-sample` prints that toolchain information, builds default and
  opt-in `libghostty-vt` release binaries in separate target directories, and
  reports artifact sizes plus binary versions for packaging evidence.
- `make packaging-layout-sample` stages opt-in release binaries, wrapper
  scripts, and `libghostty-vt` runtime-library artifacts in a local package
  layout and verifies the wrapped binaries run from that layout.
- `make packaging-layout-verify` validates an existing staged opt-in native-VT
  package layout without rebuilding, including required files, wrapper scripts,
  layout metadata, runtime-library presence, and wrapped binary version checks.
- `make packaging-provenance-sample` writes a provenance manifest for the
  staged layout, including file hashes, package metadata, toolchain/source
  mode, locked `libghostty-vt` package records, dependency tree, native
  runtime-library artifacts, and dynamic dependency output.
- `make packaging-provenance-verify` regenerates that manifest and fails if the
  required toolchain, source-mode, locked native-VT package, staged-file,
  runtime-library, per-binary `libghostty-vt` dynamic-dependency, or cargo-tree
  records are missing.
- `make packaging-archive-sample` writes a tar archive and SHA-256 file for the
  staged layout, extracts it, verifies the wrapped binaries from the extracted
  archive, and then runs the no-rebuild archive verifier.
- `make packaging-archive-verify` validates an already-produced archive plus
  sidecar hash without rebuilding, including extracted layout, package metadata,
  provenance file hashes, native runtime library, dynamic dependency records,
  and the staged-layout verifier against the extracted layout.
- `make packaging-archive-runtime-smoke` extracts the archive into a fresh
  `/tmp` install root with `DYLD_LIBRARY_PATH` and `LD_LIBRARY_PATH` unset,
  then starts the wrapped opt-in `libghostty-vt` daemon and attaches the wrapped
  client to prove the relocated package layout can serve a real pane.
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
  directory and is not full cold-checkout evidence. An isolated cold-deps
  sample clears repo-owned Cargo home and target directories, but still does
  not prove cold Nix store, source checkout, or network state.
- A GitHub Actions workflow now runs `make check` for pull requests and pushes
  to `main`; the `make promotion-evidence-bundle` job is manual, uploads the
  `nmux-promotion-evidence` artifact, and a dependent job downloads that
  artifact and runs `make promotion-evidence-verify` against the downloaded
  copy. The manual workflow must be run before any CI promotion evidence is
  recorded here.
- `target/promotion-evidence/SUMMARY.txt` records GitHub Actions run, ref, SHA,
  and runner fields when present, so CI rows can be copied from the uploaded
  artifact instead of inferred from the web UI.
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
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout and Nix store; `target/promotion-cold-deps` removed before the run; isolated repo-owned `CARGO_HOME=target/promotion-cold-deps/cargo-home` and `CARGO_TARGET_DIR=target/promotion-cold-deps/target`; `GHOSTTY_SOURCE_DIR=unset`; `GIT_CONFIG_GLOBAL=/dev/null`; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2 | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-cold-deps-sample` | Passed; wrote `target/promotion-cold-deps/REPORT.txt` and `RUN.log`; `RUN.log` showed `Updating crates.io index`, dependency downloads, and downloads of `libghostty-vt-sys v0.1.1` plus `libghostty-vt v0.1.1`; timed inner `make check-all`: `real 40.54`, `user 50.70`, `sys 14.58`, `elapsed_seconds=41`. This is isolated Cargo dependency/source-fetch evidence, not full cold machine evidence because the checkout, Nix store, and network state may still be warm. An earlier same-target attempt during implementation failed once in `live_cli_persists_rendered_surface_state`, so keep watching live CLI flake rate in cold-deps runs. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout and Nix store; working copy included a relaxed `live_cli_persists_rendered_surface_state` assertion that accepts any persisted pane surface version at least as recent as the live update; `target/promotion-cold-deps` removed before the run; isolated repo-owned `CARGO_HOME=target/promotion-cold-deps/cargo-home` and `CARGO_TARGET_DIR=target/promotion-cold-deps/target`; `GHOSTTY_SOURCE_DIR=unset`; `GIT_CONFIG_GLOBAL=/dev/null`; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2 | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-cold-deps-sample` | Passed; wrote `target/promotion-cold-deps/REPORT.txt` and `RUN.log`; timed inner `make check-all`: `real 40.05`, `user 50.43`, `sys 14.38`, `elapsed_seconds=40`. This rerun confirms the isolated dependency/source-fetch path after replacing the brittle exact surface-version assertion; it is still not full cold machine evidence because the checkout, Nix store, and network state may be warm. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout and Nix store; working copy included standalone `promotion-cold-deps-verify`; `target/promotion-cold-deps` removed before the run; isolated repo-owned `CARGO_HOME=target/promotion-cold-deps/cargo-home` and `CARGO_TARGET_DIR=target/promotion-cold-deps/target`; `GHOSTTY_SOURCE_DIR=unset`; `GIT_CONFIG_GLOBAL=/dev/null`; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2 | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-cold-deps-sample` | Passed; wrote and verified `target/promotion-cold-deps/REPORT.txt` plus `RUN.log`; timed inner `make check-all`: `real 39.35`, `user 50.41`, `sys 13.24`, `elapsed_seconds=39`. This confirms the isolated dependency/source-fetch sample is now self-verifying, but it is still not full cold machine evidence because the checkout, Nix store, and network state may be warm. |

## Non-Nix Local Attempts

These attempts validate the documented non-Nix checklist. Failed setup attempts
are not promotion evidence for the optional native VT path, but they identify
the missing host requirements needed before a complete non-Nix timing sample can
be recorded. Use the field template in [toolchain notes](toolchain.md) when
adding new rows so setup commands, exact tool versions, source mode, cache
state, timings, packaging follow-up, and gaps are explicit.

| Date | Host | Command | Result |
| --- | --- | --- | --- |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, repository caches present; host shell outside Nix; no `flatc` on PATH | `/usr/bin/time -p env GIT_CONFIG_GLOBAL=/dev/null make check-all` | Failed during tool preflight before schema or Rust tests; missing `flatc`; `real 0.01`, `user 0.00`, `sys 0.00` |

## Source-Fetch Provenance Samples

These samples record local source-fetch inputs. They do not choose the source
policy for default or packaged builds.

| Date | Host | Command | Result |
| --- | --- | --- | --- |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make source-fetch-provenance-sample` | Passed. Wrote `target/source-fetch-provenance/SOURCE_FETCH.txt` with `Cargo.lock` SHA-256 `2e28c9036cf76971ebefb3f43a39bf3f3c122aeac89981cbf666105eb20d88c4`, `libghostty-vt` 0.1.1 checksum `d8afe5cc9ae303133220e530b28b7addbbf591160bb1564b88f7ee61387fee74`, and `libghostty-vt-sys` 0.1.1 checksum `aee97068da1692162c4523d54843bdcb43fecf086a9ee412a3375817e433faca`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; working copy included standalone `source-fetch-provenance-verify`; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make source-fetch-provenance-sample` | Passed. Wrote and verified `target/source-fetch-provenance/SOURCE_FETCH.txt` against the current `Cargo.lock`; recorded `Cargo.lock` SHA-256 `2e28c9036cf76971ebefb3f43a39bf3f3c122aeac89981cbf666105eb20d88c4`, `libghostty-vt` 0.1.1 checksum `d8afe5cc9ae303133220e530b28b7addbbf591160bb1564b88f7ee61387fee74`, and `libghostty-vt-sys` 0.1.1 checksum `aee97068da1692162c4523d54843bdcb43fecf086a9ee412a3375817e433faca`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, existing Cargo registry/Nix/native source caches; separate Rust target directory `target/source-fetch-offline`; `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=/dev/null`, `CARGO_NET_OFFLINE=true` | `nix --extra-experimental-features 'nix-command flakes' develop . -c env CARGO_NET_OFFLINE=true GIT_CONFIG_GLOBAL=/dev/null CARGO_TARGET_DIR=target/source-fetch-offline cargo test -p nmux-core --features libghostty-vt --no-run` | Passed. Compiled `nmux-core --features libghostty-vt` test binary from existing local caches with Cargo offline mode in `18.47s`. This is cache-present offline evidence only; it does not prove a cold checkout, CI cache miss, network-failure behavior, or a default/package source policy. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, existing Cargo registry/Nix/native source caches; separate Rust target directory `target/source-fetch-offline`; `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=/dev/null`, `CARGO_NET_OFFLINE=true`; working copy included the reusable offline probe target | `nix --extra-experimental-features 'nix-command flakes' develop . -c make source-fetch-offline-probe` | Passed. Wrote `target/source-fetch-offline/OFFLINE_PROBE.txt` and `OFFLINE_PROBE.log`; cleared the probe target directory, compiled `nmux-core --features libghostty-vt --no-run` from existing local caches in `18s`, and recorded `source_fetch_offline_probe=passed`. This remains cache-present evidence only; it does not prove cold checkout, CI cache miss, network-failure behavior, or a default/package source policy. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, existing Cargo registry/Nix/native source caches; separate Rust target directory `target/source-fetch-offline`; `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=/dev/null`, `CARGO_NET_OFFLINE=true`; working copy included standalone `source-fetch-offline-probe-verify` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make source-fetch-offline-probe` | Passed. Wrote and verified `target/source-fetch-offline/OFFLINE_PROBE.txt` plus `OFFLINE_PROBE.log`; cleared the probe target directory, compiled `nmux-core --features libghostty-vt --no-run` from existing local caches in `19s`, and recorded `source_fetch_offline_probe=passed`. This remains cache-present evidence only; it does not prove cold checkout, CI cache miss, network-failure behavior, or a default/package source policy. |

## Packaging Samples

These samples prove release binary build behavior in a specific environment.
They do not answer install paths, signing/notarization, update channels, target
support, or native-library provenance by themselves.

| Date | Host | Command | Result |
| --- | --- | --- | --- |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-sample` | Passed. Default binaries ran `--version`; sizes: `nmux` 1065296 bytes, `nmuxd` 1182352 bytes. Opt-in `libghostty-vt` binaries built; sizes: `nmux` 1065424 bytes, `nmuxd` 1251648 bytes; `--version` passed with `DYLD_LIBRARY_PATH`/`LD_LIBRARY_PATH` set to the produced `ghostty-install/lib` runtime-library directory. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; same toolchain/source mode as packaging sample above | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-layout-sample` | Passed. Staged opt-in package layout at `target/packaging-libghostty-vt/package` with `bin/nmux`, `bin/nmuxd`, `libexec/nmux`, `libexec/nmuxd`, and `lib/libghostty-vt*`; wrapped `nmux --version` and `nmuxd --version` both reported 0.1.0 from the staged layout. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; same toolchain/source mode as packaging sample above; working copy included standalone `packaging-layout-verify` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-layout-sample` | Passed. Staged the opt-in package layout and then verified the existing layout without rebuilding provenance or archive artifacts; required wrapper scripts, libexec binaries, `PACKAGE_METADATA.txt`, bundled `libghostty-vt` runtime libraries, wrapper-managed `../lib` and `../libexec` handoff, layout metadata, and wrapped `nmux --version`/`nmuxd --version` with library-path environment variables unset. A copied layout at `/tmp/nmux-package-layout-copy` also passed `make PACKAGING_LAYOUT=/tmp/nmux-package-layout-copy packaging-layout-verify`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; same toolchain/source mode as packaging sample above | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-provenance-sample` | Passed. Wrote `target/packaging-libghostty-vt/package/PROVENANCE.txt` with package metadata, toolchain/source mode, `Cargo.lock` SHA-256 `2e28c9036cf76971ebefb3f43a39bf3f3c122aeac89981cbf666105eb20d88c4`, locked `libghostty-vt` and `libghostty-vt-sys` records, staged file sizes and hashes, native runtime-library artifacts, `otool -L` output, and locked dependency tree. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; same toolchain/source mode as packaging sample above | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-provenance-verify` | Passed. Regenerated `target/packaging-libghostty-vt/package/PROVENANCE.txt` and verified required toolchain, source-mode, `Cargo.lock`, locked `libghostty-vt` and `libghostty-vt-sys`, staged-file hash, runtime-library, per-binary `libghostty-vt` dynamic-dependency, and cargo-tree records. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; same toolchain/source mode as packaging sample above | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-archive-sample` | Passed. Wrote `target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz`, SHA-256 `1f824cdc7634e5f85e092d72fe20635cce8be834bbae33c000ac3a522fba8bdd`, extracted it under `target/packaging-libghostty-vt/archive/check`, and verified wrapped `nmux --version` and `nmuxd --version` from the extracted layout. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; same toolchain/source mode as packaging sample above; runtime smoke socket allocated under `/tmp` to keep Unix socket path below platform limits | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-archive-runtime-smoke` | Passed. Wrote and extracted `target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz`, SHA-256 `56dab0d0af12a744dff48f79108a804d768592b90e4dc7aa40ad9542edd227f0`, verified wrapped binary versions, started wrapped `nmuxd --terminal-engine libghostty-vt --one-shot`, attached wrapped `nmux`, and observed sentinel output `packaged-runtime-smoke` from the packaged daemon. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; same toolchain/source mode as packaging sample above; runtime smoke socket allocated under `/tmp`; working copy included package metadata plus relocated clean-env runtime smoke checks | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-archive-runtime-smoke` | Passed. Wrote archive SHA-256 `55e4a4de55858d5c90f35a8f1d257a4e169437b6740f37fc90fb86f956eebab0`, included `PACKAGE_METADATA.txt`, extracted the archive into fresh install root `/tmp/nmuxpkg.q6pGO8/install`, ran wrapped `nmuxd --terminal-engine libghostty-vt --one-shot` and wrapped `nmux` with `DYLD_LIBRARY_PATH`/`LD_LIBRARY_PATH` unset, and observed sentinel output `packaged-runtime-smoke`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; working copy included the no-rebuild archive verifier and evidence-bundle archive bytes | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-archive-verify` | Passed against the existing archive at `target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz`. Verified the sidecar SHA-256, extracted layout, `PACKAGE_METADATA.txt`, `PROVENANCE.txt`, `CARGO_TREE.txt`, staged file hashes against extracted files, bundled `libghostty-vt` runtime libraries, dynamic dependency records, and wrapped `nmux --version`/`nmuxd --version` with library-path environment variables unset. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; working copy included archive verifier reuse of `packaging-layout-verify` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make packaging-archive-verify` | Passed against the existing archive at `target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz`. Verified the sidecar SHA-256, extracted layout, package metadata, provenance file hashes, native runtime library, dynamic dependency records, and then ran `packaging-layout-verify` against the extracted layout under `/tmp/nmuxpkg-verify.*`; `make promotion-evidence-verify` also passed using the same extracted-layout verifier path against `target/promotion-evidence/PACKAGE_ARCHIVE.tar.gz`. |

## Local Combined Samples

These samples run source-fetch provenance, validation, and archive packaging
runtime evidence together. They are useful before updating separate provenance,
timing, and packaging rows, but they do not replace CI, cold-cache, or
multi-platform evidence.

| Date | Host | Command | Result |
| --- | --- | --- | --- |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout; existing Nix/Cargo/native build caches; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-local-sample` | Passed. Wrote `target/source-fetch-provenance/SOURCE_FETCH.txt` with pinned-fetch source mode and locked `libghostty-vt` package records; timed inner `make check-all`: `real 16.42`, `user 2.63`, `sys 3.17`; then wrote and verified `target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz` with SHA-256 `778306572191745af4da969657daf2e3c5ccf548098ecec18fff6e9b66e0cf56`, verified wrapped binary versions, started wrapped `nmuxd --terminal-engine libghostty-vt --one-shot`, attached wrapped `nmux`, and observed sentinel output `packaged-runtime-smoke` from the packaged daemon. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout with existing Nix/Cargo/native build caches; working copy included the new promotion-evidence-bundle target before commit; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. Wrote `target/promotion-evidence` containing `RUN.log`, `TOOLCHAIN.txt`, `SOURCE_FETCH.txt`, `PACKAGE_PROVENANCE.txt`, `CARGO_TREE.txt`, `ARCHIVE.sha256`, and `SUMMARY.txt`; timed inner `make check-all`: `real 16.16`, `user 2.65`, `sys 3.11`; package archive SHA-256 `1b2eaa2f2dda721c05aab3bc1405d5ab0abdf55eee92aa0c323c83c3f3bb943e`; packaged runtime smoke passed with sentinel `packaged-runtime-smoke`. Later `make promotion-evidence-verify` target work made this bundle shape self-checking for required fields and artifacts. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout with existing Nix/Cargo/native build caches; working copy included extracted timing fields in `SUMMARY.txt`; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. `make promotion-evidence-verify` accepted the generated bundle. `SUMMARY.txt` recorded `check_all_real_seconds=16.45`, `check_all_user_seconds=2.67`, and `check_all_sys_seconds=3.18`; package archive SHA-256 `56f603410b52c298eadf7ddabd3b58f936e5c03d178cf8452301138954beb109`; packaged runtime smoke passed with sentinel `packaged-runtime-smoke`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout with existing Nix/Cargo/native build caches; working copy included bundle start/completion timestamps plus elapsed duration; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. `make promotion-evidence-verify` accepted the generated bundle before and after final summary timing was written. `SUMMARY.txt` recorded `started_at_utc=2026-05-23T10:24:33Z`, `completed_at_utc=2026-05-23T10:24:54Z`, `bundle_elapsed_seconds=21`, `check_all_real_seconds=16.36`, `check_all_user_seconds=2.63`, and `check_all_sys_seconds=3.25`; package archive SHA-256 `ce499d19a2a2f095df8b3a39cea067a33f2e252a13e76eae61cfcabd2137e67d`; packaged runtime smoke passed with sentinel `packaged-runtime-smoke`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, warm checkout with existing Nix/Cargo/native build caches; working copy included relocation-safe `BUNDLE_MANIFEST.txt` hashes and bundle-relative summary artifact names; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. `make promotion-evidence-verify` checked exact bundle manifest hashes and accepted the generated bundle. `SUMMARY.txt` recorded `started_at_utc=2026-05-23T10:34:13Z`, `completed_at_utc=2026-05-23T10:34:33Z`, `bundle_elapsed_seconds=20`, `check_all_real_seconds=16.40`, `check_all_user_seconds=2.63`, and `check_all_sys_seconds=3.05`; package archive SHA-256 `893eb422b01085eb52c543b162ddfd4212efe6f5fcafcfe9efbd130d943341d9`; packaged runtime smoke passed with sentinel `packaged-runtime-smoke`. A copied bundle at `/tmp/nmux-promotion-evidence-copy` also passed `make PROMOTION_EVIDENCE_DIR=/tmp/nmux-promotion-evidence-copy promotion-evidence-verify`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, observed cache context in `CACHE_STATE.txt`: Nix store present, Cargo home/registry present, Cargo Git missing, default target dir present, promotion-cold target dir present, packaging default target dir present, packaging libghostty-vt target dir present; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. `make promotion-evidence-verify` checked exact bundle manifest hashes including `CACHE_STATE.txt` and accepted the generated bundle. `SUMMARY.txt` recorded `started_at_utc=2026-05-23T10:40:40Z`, `completed_at_utc=2026-05-23T10:41:01Z`, `bundle_elapsed_seconds=21`, `check_all_real_seconds=16.42`, `check_all_user_seconds=2.64`, and `check_all_sys_seconds=3.11`; package archive SHA-256 `2b7c956e76b4b492d86144be51d0c48cabcc5448baf9b5e9e1e342eece7238ab`; packaged runtime smoke passed with sentinel `packaged-runtime-smoke`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, observed cache context in `CACHE_STATE.txt`: Nix store present, Cargo home/registry present, Cargo Git missing, default target dir present, promotion-cold target dir present, packaging default target dir present, packaging libghostty-vt target dir present; working copy included package metadata plus relocated clean-env runtime smoke checks; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. `make promotion-evidence-verify` accepted the generated bundle and checked `RUN.log` for relocated package install root plus `packaged_runtime_smoke_library_env=unset`. `SUMMARY.txt` recorded `started_at_utc=2026-05-23T10:52:03Z`, `completed_at_utc=2026-05-23T10:52:25Z`, `bundle_elapsed_seconds=22`, `check_all_real_seconds=16.27`, `check_all_user_seconds=2.63`, and `check_all_sys_seconds=3.20`; package archive SHA-256 `9b9d04776901e9a99ea92e1415815eaff52dc0d4d56a694281596794b667b506`; packaged runtime smoke passed from `/tmp/nmuxpkg.U3JXnl/install` with sentinel `packaged-runtime-smoke`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, observed cache context in `CACHE_STATE.txt`: Nix store present, Cargo home/registry present, Cargo Git missing, default target dir present, promotion-cold target dir present, packaging default target dir present, packaging libghostty-vt target dir present; working copy included bundled package archive bytes plus no-rebuild archive verification; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. Bundle included `PACKAGE_ARCHIVE.tar.gz` and `ARCHIVE.sha256`; `make promotion-evidence-verify` invoked `make packaging-archive-verify` against the bundled archive bytes. `SUMMARY.txt` recorded `started_at_utc=2026-05-23T11:02:36Z`, `completed_at_utc=2026-05-23T11:03:00Z`, `bundle_elapsed_seconds=24`, `check_all_real_seconds=16.29`, `check_all_user_seconds=2.63`, and `check_all_sys_seconds=3.20`; package archive SHA-256 `3311324fb31461098e8382e11b84aa40c28f2b68ba15e99ffd35b09b9a313719`; packaged runtime smoke passed from `/tmp/nmuxpkg.uXsdko/install` with sentinel `packaged-runtime-smoke`. A copied bundle at `/tmp/nmux-promotion-evidence-copy` also passed `make PROMOTION_EVIDENCE_DIR=/tmp/nmux-promotion-evidence-copy promotion-evidence-verify`, validating the relocation/download path. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, observed cache context in `CACHE_STATE.txt`: Nix store present, Cargo home/registry present, Cargo Git missing, default target dir present, promotion-cold target dir present, packaging default target dir present, packaging libghostty-vt target dir present, source-fetch offline probe present; working copy included `OFFLINE_PROBE.txt` in the evidence bundle; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. Bundle included `OFFLINE_PROBE.txt`, `PACKAGE_ARCHIVE.tar.gz`, and `ARCHIVE.sha256`; `make promotion-evidence-verify` checked the offline probe result and archive bytes. `SUMMARY.txt` recorded `started_at_utc=2026-05-23T11:16:36Z`, `completed_at_utc=2026-05-23T11:17:17Z`, `bundle_elapsed_seconds=41`, `check_all_real_seconds=16.44`, `check_all_user_seconds=2.64`, and `check_all_sys_seconds=3.19`; `OFFLINE_PROBE.txt` recorded `source_fetch_offline_probe=passed` with `elapsed_seconds=17`; package archive SHA-256 `a7aac433d5f3ac0249d7de8cddac7b93397940952eb451d0c4a8c9985bdc350e`; packaged runtime smoke passed from `/tmp/nmuxpkg.yX0GCE/install` with sentinel `packaged-runtime-smoke`. A copied bundle at `target/downloaded-promotion-evidence` also passed `make PROMOTION_EVIDENCE_DIR=target/downloaded-promotion-evidence promotion-evidence-verify`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, observed cache context in `CACHE_STATE.txt`: Nix store present, Cargo home/registry present, Cargo Git missing, default target dir present, promotion-cold target dir present, packaging default target dir present, packaging libghostty-vt target dir present, source-fetch offline probe present; working copy included bundle-relative `ARCHIVE.sha256` content naming `PACKAGE_ARCHIVE.tar.gz`; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. Bundle included `OFFLINE_PROBE.txt`, `PACKAGE_ARCHIVE.tar.gz`, and `ARCHIVE.sha256`; `ARCHIVE.sha256` recorded `1e99f44fbbc829a73a4d7b3cf2063909186ada390b93f327061c7922792f116b  PACKAGE_ARCHIVE.tar.gz`. `SUMMARY.txt` recorded `started_at_utc=2026-05-23T11:22:58Z`, `completed_at_utc=2026-05-23T11:23:39Z`, `bundle_elapsed_seconds=41`, `check_all_real_seconds=16.38`, `check_all_user_seconds=2.64`, and `check_all_sys_seconds=3.17`; `OFFLINE_PROBE.txt` recorded `source_fetch_offline_probe=passed` with `elapsed_seconds=17`; packaged runtime smoke passed with sentinel `packaged-runtime-smoke`. A copied bundle at `target/downloaded-promotion-evidence` passed `make PROMOTION_EVIDENCE_DIR=target/downloaded-promotion-evidence promotion-evidence-verify`, and direct copied archive verification passed with `PACKAGING_ARCHIVE=target/downloaded-promotion-evidence/PACKAGE_ARCHIVE.tar.gz PACKAGING_ARCHIVE_SHA256=target/downloaded-promotion-evidence/ARCHIVE.sha256 make packaging-archive-verify`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, observed cache context in `CACHE_STATE.txt`: Nix store present, Cargo home/registry present, Cargo Git missing, default target dir present, promotion-cold target dir present, packaging default target dir present, packaging libghostty-vt target dir present, source-fetch offline probe present; working copy included VCS status artifact support under test, so `VCS_STATUS.txt` intentionally recorded `git_status_porcelain=dirty`; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. Bundle included `VCS_STATUS.txt`, and `make promotion-evidence-verify` checked that `SUMMARY.txt` recorded `vcs_status=VCS_STATUS.txt` and `working_tree_status=dirty` matching `git_status_porcelain=dirty`. `SUMMARY.txt` recorded `started_at_utc=2026-05-23T11:32:00Z`, `completed_at_utc=2026-05-23T11:32:41Z`, `bundle_elapsed_seconds=41`, `check_all_real_seconds=16.48`, `check_all_user_seconds=2.63`, and `check_all_sys_seconds=3.09`; `OFFLINE_PROBE.txt` recorded `source_fetch_offline_probe=passed` with `elapsed_seconds=17`; `ARCHIVE.sha256` recorded `0959120f8c89776898ebc302403848bba99dff2262534838d135102382db837b  PACKAGE_ARCHIVE.tar.gz`; packaged runtime smoke passed from `/tmp/nmuxpkg.ZVhXmt/install` with sentinel `packaged-runtime-smoke`. A copied bundle at `target/downloaded-promotion-evidence` also passed `make PROMOTION_EVIDENCE_DIR=target/downloaded-promotion-evidence promotion-evidence-verify`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, observed cache context in `CACHE_STATE.txt`: Nix store present, Cargo home/registry present, Cargo Git missing, default target dir present, promotion-cold target dir present, packaging default target dir present, packaging libghostty-vt target dir present, source-fetch offline probe present; clean committed working copy at `git_revision=9d3caba4174e6f2221a73efa9ae7c9dbe8e666f9`; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. `VCS_STATUS.txt` recorded `git_status_porcelain=clean`, `SUMMARY.txt` recorded `working_tree_status=clean`, and `make promotion-evidence-verify` checked the match. `SUMMARY.txt` recorded `started_at_utc=2026-05-23T11:34:02Z`, `completed_at_utc=2026-05-23T11:34:42Z`, `bundle_elapsed_seconds=40`, `check_all_real_seconds=16.37`, `check_all_user_seconds=2.63`, and `check_all_sys_seconds=3.22`; `OFFLINE_PROBE.txt` recorded `source_fetch_offline_probe=passed` with `elapsed_seconds=16`; `ARCHIVE.sha256` recorded `c606663f4fe363369e74c58f1cf9c948301cfc823a82a9d9b69d94b11d02c930  PACKAGE_ARCHIVE.tar.gz`; packaged runtime smoke passed from `/tmp/nmuxpkg.5awrQA/install` with sentinel `packaged-runtime-smoke`. A copied bundle at `target/downloaded-promotion-evidence` also passed `make PROMOTION_EVIDENCE_DIR=target/downloaded-promotion-evidence promotion-evidence-verify`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, observed cache context in `CACHE_STATE.txt`: Nix store present, Cargo home/registry present, Cargo Git missing, default target dir present, promotion-cold target dir present, packaging default target dir present, packaging libghostty-vt target dir present, source-fetch offline probe present; working copy included open-work snapshot artifact support under test, so `VCS_STATUS.txt` recorded `git_status_porcelain=dirty`; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. Bundle included `PROMOTION_OPEN_WORK.txt`, and `make promotion-evidence-verify` checked `promotion_decision=not-promoted`, `default_terminal_engine=interim-text`, `libghostty_vt_status=opt-in`, and the recorded CI, platform, non-Nix, source-policy, packaging, and frontend-hydration blockers. `SUMMARY.txt` recorded `started_at_utc=2026-05-23T11:43:58Z`, `completed_at_utc=2026-05-23T11:44:40Z`, `bundle_elapsed_seconds=42`, `check_all_real_seconds=16.50`, `check_all_user_seconds=2.64`, and `check_all_sys_seconds=3.14`; `OFFLINE_PROBE.txt` recorded `source_fetch_offline_probe=passed` with `elapsed_seconds=18`; `ARCHIVE.sha256` recorded `60c53019abab5fcc35f335e0dd0e4fa084ecb9ad354ae49e373c43ebdf8ecf18  PACKAGE_ARCHIVE.tar.gz`; packaged runtime smoke passed from `/tmp/nmuxpkg.9kX76E/install` with sentinel `packaged-runtime-smoke`. A copied bundle at `target/downloaded-promotion-evidence` also passed `make PROMOTION_EVIDENCE_DIR=target/downloaded-promotion-evidence promotion-evidence-verify`. |
| 2026-05-23 | Darwin arm64, Apple M5 Max, 128 GiB RAM, observed cache context in `CACHE_STATE.txt`: Nix store present, Cargo home/registry present, Cargo Git missing, default target dir present, promotion-cold target dir present, packaging default target dir present, packaging libghostty-vt target dir present, source-fetch offline probe present; working copy included exact open-work snapshot verification under test, so `VCS_STATUS.txt` recorded `git_status_porcelain=dirty`; `toolchain-info`: cargo 1.94.0, rustc 1.94.1, flatc 25.12.19, Zig 0.15.2, `GHOSTTY_SOURCE_DIR=unset`, source mode pinned fetch, `GIT_CONFIG_GLOBAL=unset` | `nix --extra-experimental-features 'nix-command flakes' develop . -c make promotion-evidence-bundle` | Passed. Bundle included `PROMOTION_OPEN_WORK.txt`, and `make promotion-evidence-verify` checked the exact current CI, full cold-checkout/cold-machine, non-Nix, source-policy, packaging, and frontend-hydration blocker lines. `SUMMARY.txt` recorded `check_all_real_seconds=16.32`, `check_all_user_seconds=2.66`, and `check_all_sys_seconds=3.20`; `OFFLINE_PROBE.txt` recorded `source_fetch_offline_probe=passed` with `elapsed_seconds=17`; packaged runtime smoke passed from `/tmp/nmuxpkg.lFMtGm/install` with sentinel `packaged-runtime-smoke`. |

## CI Promotion Samples

No manual GitHub Actions promotion evidence bundle run has been recorded yet. When
one is run, record it here with the field set in [CI notes](ci.md): workflow
run, git revision, runner, toolchain, source mode, VCS status, open-work
snapshot, cache state, timings, source-fetch provenance, uploaded artifact,
downloaded-artifact verifier result, packaging/archive SHA-256, runtime smoke
result, outcome, and follow-up.

| Date | Workflow Run | Runner | Command | Result |
| --- | --- | --- | --- | --- |

## Open Work

- Measure and record `make check-all` timing on more supported local systems,
  including a full cold-checkout or cold-machine run. Current cold-target
  evidence clears only `target/promotion-cold`, and current cold-deps evidence
  isolates Cargo home and target dirs but not the Nix store, checkout, or
  network state.
- Exercise the manual promotion evidence bundle and downloaded-artifact verifier
  jobs in CI before making the opt-in VT gate required.
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
  `make packaging-layout-verify`, `make packaging-provenance-sample`,
  `make packaging-provenance-verify`, `make packaging-archive-sample`,
  `make packaging-archive-verify`, and `make packaging-archive-runtime-smoke`
  results. The current Darwin packaging samples show the opt-in release
  binaries can run with an explicit runtime library path and staged wrapper
  layout, and the staged layout now has a no-rebuild verifier, but packaged
  binaries still need signing and platform distribution strategy.
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
