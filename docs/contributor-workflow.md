# Contributor Workflow

The normal contributor path stays on the default `interim` terminal engine. The
optional backend `libghostty-vt` path is available for terminal-correctness work,
but it is not part of every local edit, regular check, or release baseline.

## Default-Engine Work

Use this path for changes to:

- CLI argument parsing, help text, and local socket behavior;
- attach, reconnect, follow, live streaming, scrollback fetches, cached client
  state, and daemon-owned input/control semantics;
- FlatBuffers schema validation, generated bindings, and default-engine tests;
- docs, ADRs, and roadmap updates that do not change the optional native VT
  path.

Run focused tests while iterating, then run:

```sh
nix develop . -c make check
```

This validates the schema with `flatc` and runs `cargo test --workspace`
against the default engine. Keep interim renderer limitations explicit in
user-facing docs; green default-engine tests are not a claim of VT correctness.
GitHub Actions runs this default gate on pull requests and pushes to `main`;
the manual promotion workflow also uploads `nmux-promotion-evidence` and
verifies the downloaded artifact in a dependent job. See [ci.md](ci.md) for the
workflow shape.

## Terminal-Correctness Work

Use this path for changes that touch:

- `libghostty-vt` extraction, feature-gated terminal engine behavior, or
  `libghostty-vt-sys` source/build behavior;
- terminal modes, cursor state, styles, colors, hyperlinks, Kitty placeholders,
  row metadata, scrollback extraction, terminal-generated replies, or
  mode-aware key/paste/focus/mouse encoding;
- feature-sensitive attach/reconnect behavior, cached state, or daemon-owned
  structured input semantics.

Run the default gate plus the full opt-in gate:

```sh
nix develop . -c make check
nix develop . -c make check-ghostty-vt
```

`make check-ghostty-vt` runs the full `nmux-core` and `nmux-cli` package suites
with `--features libghostty-vt`. It is intentionally broader than a filtered
Ghostty smoke test.

## Promotion Evidence Work

Use this path when collecting evidence for making `libghostty-vt` the default
engine, a regular CI requirement, or a packaging baseline.

Run:

```sh
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
nix develop . -c make packaging-archive-verify
nix develop . -c make packaging-archive-runtime-smoke
```

Record the host, command, result, timing, cache state, source-fetch mode, and
any CI or packaging context in
[docs/default-engine-promotion.md](default-engine-promotion.md). A passing local
`make check-all` sample is useful evidence, but it does not change the default
engine by itself.

`make promotion-sample` prints `make toolchain-info` output and then times
`make check-all` with `time -p`. Outside the Nix shell, run the same target for
non-Nix setup attempts. The Makefile checks for `flatc` 25.12.19, `cargo`, and
optional native-VT Zig 0.15.x before running the full gate. If
`GHOSTTY_SOURCE_DIR` is set, it must point at an existing readable source
directory; otherwise the evidence sample records the pinned-fetch source mode.
Record missing-tool, wrong-version, or invalid-source-directory failures too;
they are setup evidence for the non-Nix checklist, not passing promotion
evidence.

`make promotion-cold-target-sample` clears `target/promotion-cold` and times
`make check-all` with that fresh Rust target directory. Use it for local
cold-target evidence, and record that it does not clear Cargo registry, Git
source, or Nix store caches.

`make promotion-local-sample` runs `make source-fetch-provenance-sample`,
`make promotion-sample`, and then `make packaging-archive-runtime-smoke` with
clear section headers. Use it for a local evidence pass before updating the
promotion tracker with source-provenance, validation, and packaging/archive
runtime results.
`make promotion-evidence-bundle` runs the local sample and gathers
`RUN.log`, `TOOLCHAIN.txt`, `SOURCE_FETCH.txt`, `PACKAGE_PROVENANCE.txt`,
`CARGO_TREE.txt`, `PACKAGE_ARCHIVE.tar.gz`, `ARCHIVE.sha256`,
`CACHE_STATE.txt`, `SUMMARY.txt`, and `BUNDLE_MANIFEST.txt` under
`target/promotion-evidence` for easier transcription into the promotion tracker
or manual CI evidence records.
`SUMMARY.txt` includes the extracted `time -p make check-all` values as
`check_all_real_seconds`, `check_all_user_seconds`, and
`check_all_sys_seconds`, plus `started_at_utc`, `completed_at_utc`, and
`bundle_elapsed_seconds` for the bundle artifact generation and verifier pass
before final console output. `CACHE_STATE.txt` records observed cache-related
environment values and directory presence for Nix, Cargo, target, packaging,
and source-fetch paths; classify cold, warm, restored, or unknown cache history
from that artifact plus CI/cache setup context. `BUNDLE_MANIFEST.txt` records
SHA-256 hashes for the evidence files in the bundle using stable relative
artifact names, and `SUMMARY.txt` records bundle artifacts by those same
relative names so copied or downloaded bundles remain verifiable.
It runs `make promotion-evidence-verify` before printing the artifact list.
Run `make promotion-evidence-verify` directly when reviewing an existing bundle
without regenerating the native build and packaging sample; it validates the
bundled archive bytes through `make packaging-archive-verify`. Use
`make PROMOTION_EVIDENCE_DIR=/path/to/artifact promotion-evidence-verify` when
checking a downloaded artifact outside `target/promotion-evidence`. The manual
CI workflow performs the same downloaded-artifact verification after upload.

`make source-fetch-provenance-sample` writes the active source mode and locked
`libghostty-vt` Cargo package records without inspecting Ghostty source. Use it
when updating source-fetch evidence or comparing pinned-fetch versus local
`GHOSTTY_SOURCE_DIR` samples.

`make packaging-sample` prints the same toolchain context, builds default and
opt-in release binaries in separate target directories, and reports artifact
sizes plus binary versions for packaging evidence.
`make packaging-layout-sample` stages the opt-in binaries, wrapper scripts, and
`libghostty-vt` runtime libraries under `target/packaging-libghostty-vt/package`
and verifies the wrapped binaries can run from that local layout.
`make packaging-provenance-sample` writes a manifest for that staged layout with
file hashes, package metadata, toolchain/source mode, dependency tree, native
runtime-library artifacts, and dynamic dependency output.
`make packaging-provenance-verify` regenerates that manifest and asserts the
required toolchain, source-mode, locked native-VT package, staged-file,
runtime-library, per-binary `libghostty-vt` dynamic-dependency, and cargo-tree
records are present.
`make packaging-archive-sample` archives the staged layout, writes an archive
SHA-256 file, extracts it, verifies the wrapped binaries from the extracted
layout, and then runs the no-rebuild archive verifier.
`make packaging-archive-verify` validates an already-produced archive plus
sidecar hash without rebuilding. Override `PACKAGING_ARCHIVE` and
`PACKAGING_ARCHIVE_SHA256` when checking an artifact outside the default
`target/packaging-libghostty-vt/archive` path.
`make packaging-archive-runtime-smoke` extracts the archive into a fresh `/tmp`
install root with `DYLD_LIBRARY_PATH` and `LD_LIBRARY_PATH` unset, then starts
the wrapped opt-in `libghostty-vt` daemon and attaches the wrapped client to
prove the relocated package layout can serve a real pane.

Use [toolchain.md](toolchain.md), [source-fetch-policy.md](source-fetch-policy.md),
and [packaging.md](packaging.md) when the work touches non-Nix setup, Ghostty
source policy, or release binaries. Non-Nix promotion attempts should use the
field template in [toolchain.md](toolchain.md) before being copied into the
promotion tracker.
Use the manual CI promotion evidence bundle job when collecting CI evidence; a
normal pull-request run remains default-engine-only.
