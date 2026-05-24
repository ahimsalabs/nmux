# Source Fetch Policy

This repository uses backend `libghostty-vt` extraction as the default engine.
Local development can use the `libghostty-vt-sys` pinned fetch path or an
explicit `GHOSTTY_SOURCE_DIR`; the Nix package uses a Nix-fetched Ghostty source
derivation so package builds do not clone Ghostty from inside the Cargo build.

## Current Behavior

- `nmux-core` enables `libghostty-vt` through its default Cargo feature.
- `libghostty-vt-sys` is pinned through `Cargo.lock` and fetches a pinned
  Ghostty source tree for the native VT library unless `GHOSTTY_SOURCE_DIR`
  points at an existing local Ghostty checkout.
- `just check` sets `RUST_TEST_THREADS=1` for the current FFI-backed native-VT
  gate and `GIT_CONFIG_GLOBAL=/dev/null` so local Git URL rewrite rules do not
  alter that HTTPS fetch.
- `just check` also validates `GHOSTTY_SOURCE_DIR` before running the
  native build: when the variable is set, it must point at an existing readable
  source directory; when it is unset, the build uses the pinned
  `libghostty-vt-sys` fetch path.
- `packages.default` sets `GHOSTTY_SOURCE_DIR` to a pinned Nix
  `ghostty-org/ghostty` source fetch, matching the commit expected by
  `libghostty-vt-sys`.
- `just toolchain-info` and `just promotion-sample` report the source mode as
  `ghostty_source_mode=pinned-fetch` or `ghostty_source_mode=local`, along with
  whether the local directory is present.
- Generated Ghostty build output under `target/` is build output, not nmux
  source material. Do not inspect it as implementation guidance or copy it into
  this repository.

For local validation, either allow the pinned upstream fetch or provide a local
Ghostty checkout with `GHOSTTY_SOURCE_DIR`. The Nix package path owns its source
input explicitly through `flake.nix`.

Use the narrowest source-fetch evidence target that matches the question:

| Question | Command |
| --- | --- |
| Which source mode and locked packages are active? | `nix develop . -c just source-fetch-provenance-sample` |
| Can an existing copied or bundled provenance report be checked without regenerating it? | `nix develop . -c just SOURCE_FETCH_REPORT=/path/to/SOURCE_FETCH.txt source-fetch-provenance-verify` |
| Can current caches compile native VT offline? | `nix develop . -c just source-fetch-offline-probe` |
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
env CARGO_NET_OFFLINE=true GIT_CONFIG_GLOBAL=/dev/null CARGO_TARGET_DIR=target/source-fetch-offline cargo test -p nmux-core --no-run
```

That probe compiles the default `nmux-core` test binary from existing local
Cargo/Ghostty caches. It is useful evidence that the current pinned-fetch path
can reuse cache state after a normal build has populated it, but it does not
prove cold-checkout behavior, CI cache-miss behavior, or network-failure
behavior.

## Policy Notes

ADR 0034 chooses a split source policy:

- local development uses the `libghostty-vt-sys` pinned fetch unless
  `GHOSTTY_SOURCE_DIR` is supplied;
- public Nix packages use a Nix-fetched Ghostty source derivation;
- generated Ghostty build output stays under `target/` or the Nix store and is
  not nmux source material.

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
