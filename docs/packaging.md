# Packaging Notes

nmux does not yet publish release binaries or package definitions. The current
distribution path is source checkout plus the supported Nix development shell.

## Current Shape

- Default-engine builds use the `interim` terminal engine and avoid the native
  Ghostty/Zig build.
- Opt-in correctness builds use `--features libghostty-vt` and may build the
  native Ghostty VT library through `libghostty-vt-sys`.
- `nmux` and `nmuxd` are the user-facing binaries today.
- The repository does not currently define install paths, service units,
  shell completions, notarization/signing, update channels, or binary artifact
  provenance.

## Local Packaging Sample

Use this command to collect local binary-build evidence without changing the
default engine:

```sh
nix develop . -c make packaging-sample
nix develop . -c make packaging-layout-sample
nix develop . -c make packaging-provenance-sample
nix develop . -c make packaging-archive-sample
nix develop . -c make packaging-archive-runtime-smoke
```

The target prints `make toolchain-info`, validates the optional native-VT
toolchain preflight, builds default release `nmux` and `nmuxd` binaries into
`target/packaging-default`, builds opt-in `--features libghostty-vt` release
binaries into `target/packaging-libghostty-vt`, then prints artifact paths,
byte sizes, discovered `libghostty-vt` dynamic-library artifacts, and
`--version` output. The opt-in build uses `GIT_CONFIG_GLOBAL=/dev/null` and the
same `GHOSTTY_SOURCE_DIR` validation as the terminal-correctness gate. For the
version checks, it discovers the `ghostty-install/lib` directory produced by
`libghostty-vt-sys` and sets `DYLD_LIBRARY_PATH` and `LD_LIBRARY_PATH` for the
opt-in binaries. If a built binary cannot run its version check, the target
exits nonzero after reporting all version-check failures.

Record successful or failed samples in
[default-engine-promotion.md](default-engine-promotion.md) with host, source
mode, cache state, command, artifact sizes, and result. A passing local
packaging sample proves the release binaries build in that environment; it does
not answer install paths, signing/notarization, update channels, target support,
or native-library provenance by itself.

`make packaging-layout-sample` builds on `make packaging-sample` and stages an
opt-in local package layout at `target/packaging-libghostty-vt/package`:

- `bin/nmux` and `bin/nmuxd` wrapper scripts;
- `libexec/nmux` and `libexec/nmuxd` release binaries;
- `lib/libghostty-vt*` runtime-library artifacts.

The wrappers resolve their own directory, set `DYLD_LIBRARY_PATH` and
`LD_LIBRARY_PATH` to `../lib`, and then execute the matching binary from
`../libexec`. This proves a relocatable local layout shape for the opt-in
native VT build, but it is still not a signed, installed, notarized, or
platform-native package.

`make packaging-provenance-sample` writes
`target/packaging-libghostty-vt/package/PROVENANCE.txt` for that staged layout.
The manifest includes toolchain and source-mode fields, `Cargo.lock` hash,
staged file byte sizes and SHA-256 hashes, native runtime-library artifacts,
best-effort dynamic dependency output from `otool -L` or `ldd`, and a locked
`cargo tree` for `nmux-cli --features libghostty-vt`. This is local provenance
evidence, not a release signing or supply-chain attestation format.

`make packaging-archive-sample` writes
`target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz` and
a matching `.sha256` file, extracts the archive under
`target/packaging-libghostty-vt/archive/check`, and verifies the wrapped
`nmux --version` and `nmuxd --version` commands from the extracted layout. This
is a local release-artifact smoke check; it is still not a signed, notarized,
published, or platform-native package.

`make packaging-archive-runtime-smoke` builds on the extracted archive and runs
the wrapped `nmuxd --terminal-engine libghostty-vt --one-shot` plus wrapped
`nmux` client against a temporary socket. It asserts that the client observes a
known PTY sentinel from the packaged daemon. This proves the extracted package
layout can serve a real opt-in native-VT pane locally; it is still not a
multi-platform or installed-package guarantee.

Current local Darwin evidence shows the default release binaries run directly
and the opt-in `libghostty-vt` release binaries run when the produced
`ghostty-install/lib` directory is supplied as a runtime library path. A real
package still needs an explicit runtime-library strategy, such as rpath,
bundling, platform package dependency, or another artifact layout, before
native-VT binaries are treated as shippable.

## Default-Engine Promotion Packaging Criteria

Before a later ADR can make `libghostty-vt` the default engine or a regular CI
requirement, packaging work must satisfy
[ADR 0025](adr/0025-native-vt-packaging-criteria.md) and answer:

- which target triples are supported for binaries that include the native
  Ghostty VT dependency;
- whether the native VT library is statically included, dynamically linked, or
  supplied by a platform package/artifact cache;
- how source-fetch policy, offline builds, cache provenance, and license review
  are handled for the native Ghostty source;
- which provenance and checksum manifest is required for release artifacts;
- what archive/package format is published per supported target;
- whether `nmuxd --terminal-engine libghostty-vt` is enabled in shipped
  binaries or reserved for developer builds;
- how release checks map to `make check`, `make check-ghostty-vt`, and
  `make check-all`.

Until those answers are recorded, packaged/default builds should keep the
`interim` engine as the normal path and treat `libghostty-vt` as opt-in
correctness work.
