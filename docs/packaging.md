# Packaging Notes

nmux does not yet publish release binaries or package definitions. The current
distribution path is source checkout plus the supported Nix development shell.

## Current Shape

- Default-engine builds use `libghostty-vt` and statically link the native
  Ghostty VT library.
- No-default-features builds keep the `interim` terminal engine available for
  legacy/debug coverage.
- `nmux` is the user-facing binary; `nmux daemon` is the daemon entrypoint.
- Nightly artifacts contain one `nmux` binary per target plus checksums and
  provenance. The repository does not currently define install paths, service
  units, shell completions, notarization/signing, or update channels beyond the
  `nightly` release tag.

## Nightly Artifacts

The nightly workflow builds target-specific archives named
`nmux-<target>.tar.gz`. Each archive contains:

- `nmux`;
- `VERSION.json`;
- `DYNAMIC_DEPENDENCIES.txt`;
- `PROVENANCE.txt`;
- `SHA256SUMS`.

`PROVENANCE.txt` records the GitHub repository, exact commit, run id, target,
build channel, build date, version JSON, and dynamic dependency inspection. The
workflow fails if dependency inspection shows a dynamic `libghostty-vt`
dependency. The release job also publishes a top-level `SHA256SUMS` for the
target archives.

## Local Packaging Sample

Use the narrowest packaging target that matches the evidence question:

| Question | Command |
| --- | --- |
| Do default and opt-in release binaries build and report versions locally? | `nix develop . -c just packaging-sample` |
| Does the local opt-in layout stage wrappers and runtime libraries correctly? | `nix develop . -c just packaging-layout-sample` then `nix develop . -c just packaging-layout-verify` |
| Does the staged layout have the required local provenance records? | `nix develop . -c just packaging-provenance-sample` then `nix develop . -c just packaging-provenance-verify` |
| Can an existing copied or bundled provenance manifest be checked without rebuilding? | `nix develop . -c just packaging-provenance-manifest-verify` |
| Does the staged layout archive and verify as a self-contained artifact? | `nix develop . -c just packaging-archive-sample` then `nix develop . -c just packaging-archive-verify` |
| Can the relocated archive serve a real opt-in native-VT pane? | `nix develop . -c just packaging-archive-runtime-smoke` |

The target prints `just toolchain-info`, validates the optional native-VT
toolchain preflight, builds the default release `nmux` binary into
`target/packaging-default`, builds opt-in `--features libghostty-vt` release
`nmux` into `target/packaging-libghostty-vt`, then prints artifact paths,
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

`just packaging-layout-sample` builds on `just packaging-sample` and stages an
opt-in local package layout at `target/packaging-libghostty-vt/package`:

- `bin/nmux` wrapper script;
- `libexec/nmux` release binary;
- `lib/libghostty-vt*` runtime-library artifacts.

The wrappers resolve their own directory, set `DYLD_LIBRARY_PATH` and
`LD_LIBRARY_PATH` to `../lib`, and then execute the matching binary from
`../libexec`. This proves a relocatable local layout shape for the opt-in
native VT build, but it is still not a signed, installed, notarized, or
platform-native package.

`just packaging-layout-verify` is the no-rebuild staged-layout verifier. By
default it checks `target/packaging-libghostty-vt/package`, but
`PACKAGING_LAYOUT=/path/to/package` can point it at an existing or copied staged
layout. The verifier requires the wrapper scripts, libexec binaries, package
metadata, and at least one bundled `libghostty-vt` runtime library; checks the
wrapper-managed `../lib` and `../libexec` shape; validates layout-level
metadata; and runs wrapped `nmux --version` with
library-path environment variables unset. It does not validate provenance
hashes, archive bytes, or daemon/client runtime behavior.

`just packaging-provenance-sample` writes
`target/packaging-libghostty-vt/package/PROVENANCE.txt` for that staged layout.
The staged layout also includes `PACKAGE_METADATA.txt` with the local archive
format, host target, opt-in terminal-engine status, source mode, and wrapper
managed runtime-library strategy. The provenance manifest includes toolchain
and source-mode fields, `Cargo.lock` hash, locked `libghostty-vt` and
`libghostty-vt-sys` package records, staged file byte sizes and SHA-256 hashes,
native runtime-library artifacts, best-effort dynamic dependency output from
`otool -L` or `ldd`, and a locked `cargo tree` for `nmux-cli --features
libghostty-vt`. This is local provenance evidence, not a release signing or
supply-chain attestation format.

`just packaging-provenance-verify` regenerates that manifest and fails if the
required toolchain, source-mode, locked native-VT package, staged-file,
runtime-library, per-binary `libghostty-vt` dynamic-dependency, or cargo-tree
records are missing. Archive packaging depends on this verifier, so
`just packaging-archive-sample` and `just packaging-archive-runtime-smoke`
cannot pass with a structurally incomplete local provenance manifest.

`just packaging-provenance-manifest-verify` checks an existing manifest without
rebuilding. By default it checks
`target/packaging-libghostty-vt/package/PROVENANCE.txt`, but
`PACKAGING_PROVENANCE_MANIFEST=/path/to/PACKAGE_PROVENANCE.txt` can point it at
copied, bundled, or downloaded promotion evidence. `just
promotion-evidence-verify` uses this path for bundled `PACKAGE_PROVENANCE.txt`
so it validates the evidence under review rather than regenerating local
package provenance first.

`just packaging-archive-sample` writes
`target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz` and
a matching `.sha256` file, extracts the archive under
`target/packaging-libghostty-vt/archive/check`, and verifies the wrapped
`nmux --version` from the extracted layout. It
then runs `just packaging-archive-verify` against the produced archive and
sidecar hash. This is a local release-artifact smoke check; it is still not a
signed, notarized, published, or platform-native package.

`just packaging-archive-verify` is the no-rebuild archive verifier. By default
it checks the archive at
`target/packaging-libghostty-vt/archive/nmux-libghostty-vt-package.tar.gz` and
its `.sha256` file, but `PACKAGING_ARCHIVE=/path/to/archive.tar.gz` and
`PACKAGING_ARCHIVE_SHA256=/path/to/archive.sha256` can point it at an existing
or downloaded artifact. The verifier checks the archive bytes against the
sidecar hash, rejects unsafe archive paths, extracts into a temporary directory,
requires the package metadata/provenance/cargo-tree files, validates staged
file hashes against the extracted files, requires bundled `libghostty-vt`
runtime libraries and dynamic dependency records, and then runs
`just packaging-layout-verify` against the extracted layout so wrapper shape,
layout metadata, runtime-library presence, and wrapped binary version checks
are covered by the same staged-layout verifier.

`just packaging-archive-runtime-smoke` builds on the archive sample, extracts
the archive into a fresh `/tmp` install root outside `target/`, unsets
`DYLD_LIBRARY_PATH` and `LD_LIBRARY_PATH`, and runs the wrapped
`nmux daemon --terminal-engine libghostty-vt --one-shot` plus wrapped `nmux` client
against a temporary socket. It asserts that the client observes a known PTY
sentinel from the packaged daemon. This proves the relocated package layout can
serve a real opt-in native-VT pane locally through its own wrappers; it is still
not a multi-platform or installed-package guarantee.

Current local Darwin evidence shows the default release binaries run directly,
the opt-in `libghostty-vt` binaries can run with the build-produced runtime
library path, and the staged wrapper/archive layout bundles `libghostty-vt`
runtime libraries and passes the relocated archive runtime smoke locally. That
is package-layout evidence, not a published distribution decision: supported
target triples, signing/notarization, installer/update shape, and final
static/dynamic/runtime-library policy still need a later packaging ADR before
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
- whether `nmux daemon --terminal-engine libghostty-vt` is enabled in shipped
  binaries or reserved for developer builds;
- how release checks map to `just check`, `just check-ghostty-vt`, and
  `just check-all`.

Until those answers are recorded, packaged/default builds should keep the
`interim` engine as the normal path and treat `libghostty-vt` as opt-in
correctness work.
