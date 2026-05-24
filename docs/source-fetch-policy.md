# Source Fetch Policy

This repository keeps backend `libghostty-vt` extraction opt-in. The current
source-fetch policy is acceptable for local correctness work, but it is not yet
enough to make `libghostty-vt` the default engine, a regular CI requirement, or
a packaging baseline.

## Current Opt-In Behavior

- `nmux-core` depends on `libghostty-vt = "=0.1.1"` only through the optional
  `libghostty-vt` Cargo feature.
- `libghostty-vt-sys` is pinned through `Cargo.lock` and fetches a pinned
  Ghostty source tree for the native VT library unless `GHOSTTY_SOURCE_DIR`
  points at an existing local Ghostty checkout.
- `just check-ghostty-vt` sets `RUST_TEST_THREADS=1` for the current
  FFI-backed native-VT evidence gate and `GIT_CONFIG_GLOBAL=/dev/null` so
  local Git URL rewrite rules do not alter that HTTPS fetch.
- `just check-ghostty-vt` also validates `GHOSTTY_SOURCE_DIR` before running the
  native build: when the variable is set, it must point at an existing readable
  source directory; when it is unset, the build uses the pinned
  `libghostty-vt-sys` fetch path.
- `just toolchain-info` and `just promotion-sample` report the source mode as
  `ghostty_source_mode=pinned-fetch` or `ghostty_source_mode=local`, along with
  whether the local directory is present.
- Generated Ghostty build output under `target/` is build output, not nmux
  source material. Do not inspect it as implementation guidance or copy it into
  this repository.

For local opt-in validation, either allow the pinned upstream fetch or provide a
local Ghostty checkout with `GHOSTTY_SOURCE_DIR`. Record which path was used
when adding default-engine promotion evidence.

Use the narrowest source-fetch evidence target that matches the question:

| Question | Command |
| --- | --- |
| Which source mode and locked packages are active? | `nix develop . -c just source-fetch-provenance-sample` |
| Can an existing copied or bundled provenance report be checked without regenerating it? | `nix develop . -c just SOURCE_FETCH_REPORT=/path/to/SOURCE_FETCH.txt source-fetch-provenance-verify` |
| Can current caches compile opt-in native VT offline? | `nix develop . -c just source-fetch-offline-probe` |
| Can an existing copied or bundled offline probe be checked without rerunning it? | `nix develop . -c just SOURCE_FETCH_OFFLINE_PROBE_REPORT=/path/to/OFFLINE_PROBE.txt SOURCE_FETCH_OFFLINE_PROBE_LOG=/path/to/OFFLINE_PROBE.log source-fetch-offline-probe-verify` |

The provenance report is written to
`target/source-fetch-provenance/SOURCE_FETCH.txt` and includes the active source
mode, `GHOSTTY_SOURCE_DIR`, `GIT_CONFIG_GLOBAL`, `Cargo.lock` SHA-256,
toolchain info, and the locked `Cargo.lock` records for `libghostty-vt` and
`libghostty-vt-sys`. It is evidence for the current local source-fetch path,
not a default or packaged-build source-policy decision.

The offline probe report is written to
`target/source-fetch-offline/OFFLINE_PROBE.txt` and the command log to
`target/source-fetch-offline/OFFLINE_PROBE.log`. The target clears that Rust
target directory, then runs:

```sh
env CARGO_NET_OFFLINE=true GIT_CONFIG_GLOBAL=/dev/null CARGO_TARGET_DIR=target/source-fetch-offline cargo test -p nmux-core --features libghostty-vt --no-run
```

That probe compiles the opt-in `nmux-core` test binary from existing local
Cargo/Ghostty caches. It is useful evidence that the current pinned-fetch path
can reuse cache state after a normal opt-in build has populated it, but it does
not prove cold-checkout behavior, CI cache-miss behavior, network-failure
behavior, or a packaged/default source policy.

## Promotion Blockers

Before `libghostty-vt` can become the documented default engine, a regular CI
requirement, or a packaged binary dependency, a later ADR must satisfy
[ADR 0024](adr/0024-native-vt-source-policy-criteria.md), choose one of these
source policies, and record the consequences:

- pinned network fetch with CI/cache controls;
- vendored or mirrored source with update and license-review rules;
- pre-fetched local source via `GHOSTTY_SOURCE_DIR`;
- platform package or artifact cache for the native Ghostty VT library.

That decision also needs offline-build behavior, cache invalidation, provenance,
license-review scope, and binary packaging expectations. Until then, the
default engine remains `interim`, and `just check` remains independent of the
native Ghostty/Zig build.
See [packaging.md](packaging.md) for the matching binary distribution questions.

Local package provenance samples record the active source mode,
`GHOSTTY_SOURCE_DIR` value, and locked `libghostty-vt`/`libghostty-vt-sys`
package records. `just packaging-provenance-verify` asserts those records are
present before archive packaging continues, and
`just packaging-provenance-manifest-verify` can check copied or bundled
manifest evidence without rebuilding. That record is not enough to settle the
source policy for default or packaged builds. A later promotion decision still
needs to choose how the pinned Ghostty source, local-source overrides, offline
builds, cache provenance, and license review are represented in release
artifacts.
