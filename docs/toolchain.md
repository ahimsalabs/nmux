# Toolchain Notes

The supported development path is the Nix shell:

```sh
nix develop . -c make check
nix develop . -c make check-ghostty-vt
nix develop . -c make check-all
nix develop . -c make promotion-sample
nix develop . -c make promotion-cold-target-sample
nix develop . -c make promotion-local-sample
nix develop . -c make promotion-evidence-bundle
nix develop . -c make promotion-evidence-verify
nix develop . -c make source-fetch-provenance-sample
nix develop . -c make packaging-sample
nix develop . -c make packaging-layout-sample
nix develop . -c make packaging-provenance-sample
nix develop . -c make packaging-provenance-verify
nix develop . -c make packaging-archive-sample
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
For a combined local source-provenance, validation, and packaging/archive
evidence pass, use:

```sh
nix develop . -c make promotion-local-sample
nix develop . -c make promotion-evidence-bundle
nix develop . -c make promotion-evidence-verify
```

The local sample target runs `make source-fetch-provenance-sample`,
`make promotion-sample`, and then `make packaging-archive-runtime-smoke`. The
bundle target runs the same local sample and gathers the run log, toolchain
output, bundle start/completion timestamps plus elapsed duration, extracted
`time -p make check-all` values, source-fetch report, package provenance, cargo
tree, archive checksum, observed cache-state report, and bundle artifact manifest under
`target/promotion-evidence`, then runs
`make promotion-evidence-verify`. Run the verifier directly to check an
existing bundle without rebuilding the native VT package, or pass
`PROMOTION_EVIDENCE_DIR=/path/to/artifact` for a downloaded bundle outside the
default `target/promotion-evidence` path.
For source-fetch provenance evidence, use:

```sh
nix develop . -c make source-fetch-provenance-sample
```

That target writes `target/source-fetch-provenance/SOURCE_FETCH.txt` with the
active source mode, `GHOSTTY_SOURCE_DIR`, `GIT_CONFIG_GLOBAL`, `Cargo.lock`
SHA-256, toolchain info, and locked `Cargo.lock` records for `libghostty-vt`
and `libghostty-vt-sys`.
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
```

That target stages the opt-in binaries, wrapper scripts, and `libghostty-vt`
runtime libraries under `target/packaging-libghostty-vt/package`, then verifies
the wrapped binaries can run from that layout.
For local package provenance evidence, use:

```sh
nix develop . -c make packaging-provenance-sample
nix develop . -c make packaging-provenance-verify
```

The sample target writes `target/packaging-libghostty-vt/package/PROVENANCE.txt`
with staged file sizes and SHA-256 hashes, package metadata, `Cargo.lock` hash,
toolchain/source mode, dependency tree, native runtime-library artifacts, and
best-effort dynamic dependency output. The verifier regenerates that manifest
and asserts the required toolchain, source-mode, locked native-VT package,
staged-file, package metadata, runtime-library, per-binary `libghostty-vt`
dynamic-dependency, and cargo-tree records are present before archive packaging
continues.
For local archive evidence, use:

```sh
nix develop . -c make packaging-archive-sample
```

That target writes `target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz`,
writes a matching `.sha256` file, extracts the archive, and verifies the wrapped
`nmux` and `nmuxd` binaries from the extracted layout.
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
