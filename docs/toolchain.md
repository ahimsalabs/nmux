# Toolchain Notes

The supported development path is the Nix shell. Most contributors only need
the default-engine check and the local daemon/client smoke:

```sh
nix develop . -c make check
nix develop . -c make local-smoke
```

All Nix examples assume `nix-command` and `flakes` are enabled. If your Nix
install has not enabled them globally, run the same commands as:
`nix --extra-experimental-features 'nix-command flakes' develop . -c ...`.

Use a broader target only when the change needs the extra evidence:

| Work type | Target |
| --- | --- |
| Normal default-engine or docs work | `nix develop . -c make check` and `nix develop . -c make local-smoke` |
| Backend `libghostty-vt` correctness work | `nix develop . -c make check` and `nix develop . -c make check-ghostty-vt` |
| Release-style local validation | `nix develop . -c make check-all` |
| Self-contained promotion evidence bundle | `nix develop . -c make promotion-evidence-bundle` then `nix develop . -c make promotion-evidence-verify` |
| Source-fetch evidence | `nix develop . -c make source-fetch-provenance-sample` or `nix develop . -c make source-fetch-offline-probe` |
| Packaging evidence | Choose the matching target in [packaging.md](packaging.md#local-packaging-sample); use `nix develop . -c make packaging-sample` for binary-build evidence and `nix develop . -c make packaging-archive-runtime-smoke` only for relocated runtime evidence. |

The common check and evidence target inventory is:

```sh
nix develop . -c make check
nix develop . -c make check-ghostty-vt
nix develop . -c make check-all
nix develop . -c make local-smoke
nix develop . -c make promotion-sample
nix develop . -c make promotion-cold-target-sample
nix develop . -c make promotion-cold-deps-sample
nix develop . -c make promotion-cold-deps-verify
nix develop . -c make promotion-local-sample
nix develop . -c make promotion-evidence-bundle
nix develop . -c make promotion-evidence-verify
nix develop . -c make source-fetch-provenance-sample
nix develop . -c make source-fetch-provenance-verify
nix develop . -c make source-fetch-offline-probe
nix develop . -c make source-fetch-offline-probe-verify
nix develop . -c make packaging-sample
nix develop . -c make packaging-layout-sample
nix develop . -c make packaging-layout-verify
nix develop . -c make packaging-provenance-sample
nix develop . -c make packaging-provenance-verify
nix develop . -c make packaging-provenance-manifest-verify
nix develop . -c make packaging-archive-sample
nix develop . -c make packaging-archive-verify
nix develop . -c make packaging-archive-runtime-smoke
```

`flake.nix` currently provides:

- Rust `cargo` and `rustc` for the workspace crates;
- `flatc` through `pkgs.flatbuffers` for schema validation and generated Rust
  bindings;
- GNU Make for the repository check targets;
- Zig 0.15 for the optional native Ghostty VT build.

The regular contributor path is `make check`, which validates
[schema/nmux.fbs](../schema/nmux.fbs) and runs `cargo test --workspace` against
the default `interim` terminal engine. `make check-ghostty-vt` and
`make check-all` are explicit opt-in gates for changes that touch backend
`libghostty-vt` extraction, feature-sensitive attach/reconnect behavior, cached
state, daemon-owned structured input, or default-engine promotion evidence.
`make local-smoke` is a default-engine user workflow smoke for the local
daemon/client path; it does not enable the native VT feature or replace the
test suite.
See [contributor-workflow.md](contributor-workflow.md) for choosing between
those gates.

For setup and evidence records, print the active tools with:

```sh
nix develop . -c make toolchain-info
```

For a local default-engine-promotion evidence sample, use:

```sh
nix develop . -c make promotion-sample
```

That target prints `toolchain-info` and then runs `time -p make check-all`.
For a local cold-target-dir validation sample, use:

```sh
nix develop . -c make promotion-cold-target-sample
```

That target clears `target/promotion-cold` and times `make check-all` with that
fresh Rust target directory. It does not clear Cargo registry, Git source, or
Nix store caches, so record it as cold-target evidence rather than full
cold-checkout evidence.
For isolated Cargo dependency/source-fetch evidence, use:

```sh
nix develop . -c make promotion-cold-deps-sample
```

`make promotion-cold-deps-sample` clears `target/promotion-cold-deps` and
runs `make check-all` with isolated repo-owned `CARGO_HOME` and
`CARGO_TARGET_DIR`, `GHOSTTY_SOURCE_DIR` unset, and `GIT_CONFIG_GLOBAL=/dev/null`.
It records `target/promotion-cold-deps/REPORT.txt` plus `RUN.log`. Treat it as
dependency/source-fetch evidence only: the Nix store, source checkout, and
network state may still be warm.

For a combined local source-provenance, validation, workflow-smoke,
offline-probe, and package runtime evidence pass, use:

```sh
nix develop . -c make promotion-local-sample
```

The local sample target runs `make source-fetch-provenance-sample`,
`make promotion-sample`, `make local-smoke`, `make source-fetch-offline-probe`,
and then `make packaging-archive-runtime-smoke`.

Use the evidence bundle only when a self-contained artifact is needed:

```sh
nix develop . -c make promotion-evidence-bundle
nix develop . -c make promotion-evidence-verify
```

The bundle target runs the same local sample and gathers the run log,
toolchain output, bundle
start/completion timestamps plus elapsed duration, extracted `time -p make
check-all` values, `local_smoke` result, source-fetch report, offline probe
report, package provenance, cargo tree, package archive, archive checksum,
observed cache-state report, VCS status report, open-work snapshot, and bundle
artifact manifest under
`target/promotion-evidence`, then runs
`make promotion-evidence-verify`. Run the verifier directly to check an
existing bundle without rebuilding the native VT package; `ARCHIVE.sha256`
names `PACKAGE_ARCHIVE.tar.gz`, and the verifier validates those bundled
package provenance and archive bytes through `make
packaging-provenance-manifest-verify` and `make packaging-archive-verify`. The
verifier also checks that `SUMMARY.txt` matches the recorded `VCS_STATUS.txt`
working-tree status and names the bundled `PROMOTION_OPEN_WORK.txt` snapshot.
Pass
`PROMOTION_EVIDENCE_DIR=/path/to/artifact` for a downloaded bundle outside the
default `target/promotion-evidence` path.
For source-fetch provenance evidence, use:

```sh
nix develop . -c make source-fetch-provenance-sample
nix develop . -c make source-fetch-provenance-verify
nix develop . -c make source-fetch-offline-probe
nix develop . -c make source-fetch-offline-probe-verify
```

The provenance sample writes
`target/source-fetch-provenance/SOURCE_FETCH.txt` with the active source mode,
`GHOSTTY_SOURCE_DIR`, `GIT_CONFIG_GLOBAL`, `Cargo.lock` SHA-256, toolchain
info, and locked `Cargo.lock` records for `libghostty-vt` and
`libghostty-vt-sys`. The provenance verifier checks the existing report
without collecting a new sample.
The offline probe writes `target/source-fetch-offline/OFFLINE_PROBE.txt` and
checks whether `nmux-core --features libghostty-vt` can compile from current
caches with `CARGO_NET_OFFLINE=true` and `GIT_CONFIG_GLOBAL=/dev/null`. The
offline-probe verifier checks the existing probe report. Treat the probe as
cache-present evidence only, not cold-checkout, CI cache-miss, or source-policy
evidence.
For local release-binary evidence, use:

```sh
nix develop . -c make packaging-sample
```

That target builds default and opt-in `libghostty-vt` release binaries in
separate target directories, then prints artifact sizes, dynamic-library
artifacts, the discovered runtime library directory, and binary versions. A
failed opt-in binary version check is packaging evidence and should be recorded
in
[the default-engine promotion tracker](default-engine-promotion.md).
For a local staged package-layout smoke check, use:

```sh
nix develop . -c make packaging-layout-sample
nix develop . -c make packaging-layout-verify
```

The sample target stages the opt-in binaries, wrapper scripts, and
`libghostty-vt` runtime libraries under
`target/packaging-libghostty-vt/package`, then runs the layout verifier. Run
`make packaging-layout-verify` directly to check an already staged layout
without rebuilding it.
For local package provenance evidence, use:

```sh
nix develop . -c make packaging-provenance-sample
nix develop . -c make packaging-provenance-verify
nix develop . -c make packaging-provenance-manifest-verify
```

The sample target writes `target/packaging-libghostty-vt/package/PROVENANCE.txt`
with staged file sizes and SHA-256 hashes, package metadata, `Cargo.lock` hash,
toolchain/source mode, dependency tree, native runtime-library artifacts, and
best-effort dynamic dependency output. The verifier regenerates that manifest
and asserts the required toolchain, source-mode, locked native-VT package,
staged-file, package metadata, runtime-library, per-binary `libghostty-vt`
dynamic-dependency, and cargo-tree records are present before archive packaging
continues.
Run `make packaging-provenance-manifest-verify` directly to check an already
produced manifest without rebuilding. Override `PACKAGING_PROVENANCE_MANIFEST`
when verifying a copied or bundled `PACKAGE_PROVENANCE.txt`.
For local archive evidence, use:

```sh
nix develop . -c make packaging-archive-sample
nix develop . -c make packaging-archive-verify
```

That target writes `target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz`,
writes a matching `.sha256` file, extracts the archive, verifies the wrapped
`nmux` and `nmuxd` binaries from the extracted layout, and then runs the
no-rebuild archive verifier. Run `make packaging-archive-verify` directly to
check an already-produced archive. Override `PACKAGING_ARCHIVE` and
`PACKAGING_ARCHIVE_SHA256` when verifying a downloaded artifact outside the
default `target/packaging-libghostty-vt/archive` path.
For local packaged runtime evidence, use:

```sh
nix develop . -c make packaging-archive-runtime-smoke
```

That target extracts the archive into a fresh `/tmp` install root with
`DYLD_LIBRARY_PATH` and `LD_LIBRARY_PATH` unset, then starts the wrapped opt-in
`libghostty-vt` daemon and attaches the wrapped client to prove the relocated
package layout can serve a real pane.

## Non-Nix Equivalents

The non-Nix path is documented as a requirements checklist, not as promotion
evidence. Before it can justify making `libghostty-vt` a default engine or
regular CI requirement, it still needs platform-specific setup commands,
timings, cache behavior, and packaging decisions recorded in
[the default-engine promotion tracker](default-engine-promotion.md).

Current validation status: the Nix shell provisions `flatc` 25.12.19 and is the
validated local workflow for optional native Ghostty VT checks. No non-Nix
`make check-ghostty-vt` or `make check-all` run has passed as promotion
evidence. A local non-Nix `make check-all` attempt on 2026-05-23 failed during
Makefile tool preflight because `flatc` was not on the host PATH outside the
Nix shell. That is a non-Nix setup gap, not a Nix-shell blocker.

A non-Nix environment must provide:

- a Rust toolchain new enough for Cargo workspace resolver 3 and edition 2024;
- `flatc` 25.12.19, matching the pinned Rust `flatbuffers = "=25.12.19"`
  crate;
- `make`;
- Zig 0.15.x when building or testing `--features libghostty-vt`;
- network or local-source policy for the `libghostty-vt-sys` Ghostty source
  fetch.

For the default engine, the command shape is:

```sh
make check
```

The Makefile checks for required tools before running schema or Rust tests. A
missing `flatc`, `cargo`, or optional native-VT `zig` reports the missing tool
and the matching Nix command to use. `flatc` must report version 25.12.19, and
the optional native-VT `zig` must be in the 0.15.x line.

For the opt-in VT engine, the command shape is:

```sh
GIT_CONFIG_GLOBAL=/dev/null make check-ghostty-vt
```

`GIT_CONFIG_GLOBAL=/dev/null` is not a semantic nmux requirement. It keeps local
Git URL rewrite rules from changing the HTTPS source fetch used by
`libghostty-vt-sys`. If an environment uses a pre-fetched Ghostty checkout, set
`GHOSTTY_SOURCE_DIR` according to the `libghostty-vt-sys` build path and record
that source policy before using the result as promotion evidence.
Use `make promotion-sample` for non-Nix setup attempts or promotion samples so
the exact tool versions, source-fetch environment, and `check-all` timing are
visible together. When `GHOSTTY_SOURCE_DIR` is set, the optional VT preflight
requires it to point at an existing readable directory. When it is unset, the
source mode is recorded as the pinned `libghostty-vt-sys` fetch path.

Record non-Nix attempts with these fields before treating them as promotion
evidence:

| Field | Required Content |
| --- | --- |
| Host | OS version, architecture, CPU class, and memory if known. |
| Setup commands | Platform-specific commands used to install Rust, FlatBuffers, make, Zig, and any source-fetch prerequisites. |
| Toolchain | `make toolchain-info` output, including exact `flatc` and Zig versions. |
| Source mode | `ghostty_source_mode`, `GHOSTTY_SOURCE_DIR`, and `GIT_CONFIG_GLOBAL`. |
| Cache state | Whether Cargo registry, Cargo Git, Rust target, native Ghostty/Zig build, and source-fetch caches were cold, warm, restored, or unknown. |
| Command | Exact command, usually `/usr/bin/time -p env GIT_CONFIG_GLOBAL=/dev/null make promotion-sample` or `make check-all`. |
| Result | Passed or failed, including timed output and the first failing command. |
| Packaging follow-up | Whether matching `make packaging-*` targets were run in the same environment or left as open work. |
| Gaps | Missing tools, version mismatches, source-fetch failures, cache assumptions, or platform-specific packaging issues. |

See [source-fetch-policy.md](source-fetch-policy.md) for the current opt-in
policy and the remaining source-fetch decisions for packaged/default builds.
See [packaging.md](packaging.md) for binary distribution questions that remain
outside the supported development shell.
