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
nix develop . -c make local-smoke
```

This validates the schema with `flatc` and runs `cargo test --workspace`
against the default engine, then runs a real local daemon/client smoke over a
temporary socket. Keep interim renderer limitations explicit in user-facing
docs; green default-engine tests and smoke checks are not a claim of VT
correctness. GitHub Actions runs this default gate on pull requests and pushes
to `main`; the manual promotion workflow also uploads `nmux-promotion-evidence`
and verifies the downloaded artifact in a dependent job. See [ci.md](ci.md) for
the workflow shape.

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
with `--features libghostty-vt` and `RUST_TEST_THREADS=1`. It is intentionally
broader than a filtered Ghostty smoke test. The serial harness setting is part
of the current native-VT evidence gate; do not replace it with filtered tests
when changing terminal engine behavior.

`make renderer-equivalence-smoke` is narrower: it runs the current
renderer-equivalence fixture corpus projection through the opt-in
`libghostty-vt` path. Use it while building renderer-equivalence evidence, but
do not treat a pass as a trusted renderer oracle comparison or as default-engine
promotion evidence.

## Promotion Evidence Work

Use this path when collecting evidence for making `libghostty-vt` the default
engine, a regular CI requirement, or a packaging baseline.

Start with the narrow evidence path that matches the question:

| Question | Command |
| --- | --- |
| Does the combined default plus opt-in gate pass here? | `nix develop . -c make promotion-sample` |
| Does the user-level default workflow still work? | `nix develop . -c make local-smoke` |
| What changes with a fresh Rust target dir? | `nix develop . -c make promotion-cold-target-sample` |
| Can isolated Cargo dependency/source fetches satisfy the opt-in path? | `nix develop . -c make promotion-cold-deps-sample` then `nix develop . -c make promotion-cold-deps-verify` |
| Do source-fetch, validation, workflow smoke, offline probe, and package runtime smoke pass together? | `nix develop . -c make promotion-local-sample` |
| Do we need a self-contained evidence artifact? | `nix develop . -c make promotion-evidence-bundle` then `nix develop . -c make promotion-evidence-verify` |
| Are source-fetch or packaging details under review? | Use the focused `source-fetch-*` and `packaging-*` targets described below, or the broader target inventory in [toolchain.md](toolchain.md). |

Record the host, command, result, timing, cache state, source-fetch mode, and
any CI or packaging context in
[docs/default-engine-promotion.md](default-engine-promotion.md). A passing local
`make check-all` sample is useful evidence, but it does not change the default
engine by itself.

`make local-smoke` is a quick user-level workflow check for the default engine:
it starts `nmuxd` and sequential `nmux` live clients over a temporary socket,
sends piped stdin input, verifies the echoed output survives read-only reattach
through a persisted state file, verifies nested context and JSON informational
flags plus daemon readiness JSON, then starts a new daemon on the same socket
path and verifies the old cached surface does not leak across socket
recreation. It exits without leaving the socket behind.

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

`make promotion-cold-deps-sample` clears `target/promotion-cold-deps` and times
`make check-all` with isolated repo-owned `CARGO_HOME` and `CARGO_TARGET_DIR`,
`GHOSTTY_SOURCE_DIR` unset, and `GIT_CONFIG_GLOBAL=/dev/null`. Use it for
dependency/source-fetch evidence, and record that it still does not prove cold
Nix store, source checkout, or network state.
`make promotion-cold-deps-verify` checks the existing `REPORT.txt` and
`RUN.log` for the expected isolation fields, timing fields, passing result, and
default plus opt-in test commands without rerunning the sample.

`make promotion-local-sample` runs `make source-fetch-provenance-sample`,
`make promotion-sample`, `make local-smoke`, `make source-fetch-offline-probe`,
and then `make packaging-archive-runtime-smoke` with clear section headers. Use
it for a local evidence pass before updating the promotion tracker with
source-provenance, default-engine workflow smoke, cache-present offline probe,
validation, and packaging/archive runtime results.
`make promotion-evidence-bundle` runs the local sample and gathers
`RUN.log`, `TOOLCHAIN.txt`, `SOURCE_FETCH.txt`, `OFFLINE_PROBE.txt`,
`PACKAGE_PROVENANCE.txt`, `CARGO_TREE.txt`, `PACKAGE_ARCHIVE.tar.gz`,
`ARCHIVE.sha256`, `CACHE_STATE.txt`, `VCS_STATUS.txt`,
`PROMOTION_OPEN_WORK.txt`, `SUMMARY.txt`, and `BUNDLE_MANIFEST.txt` under
`target/promotion-evidence` for easier
transcription into the promotion tracker or manual CI evidence records.
`ARCHIVE.sha256` names `PACKAGE_ARCHIVE.tar.gz`, not the original build-tree
archive path, so copied or downloaded bundles stay self-contained.
`SUMMARY.txt` includes the extracted `time -p make check-all` values as
`check_all_real_seconds`, `check_all_user_seconds`, and
`check_all_sys_seconds`, the `local_smoke=passed`,
`local_smoke_reattach=passed`, `local_smoke_print_context=passed`,
`local_smoke_json_info=passed`, `local_smoke_ready_json=passed`, and
`local_smoke_socket_recreation=passed` results, plus
`started_at_utc`, `completed_at_utc`, and `bundle_elapsed_seconds` for the
bundle artifact generation and verifier pass
before final console output. `CACHE_STATE.txt` records observed cache-related
environment values and directory presence for Nix, Cargo, target, packaging,
and source-fetch paths; classify cold, warm, restored, or unknown cache history
from that artifact plus CI/cache setup context. `VCS_STATUS.txt` records the
Git revision, Git working-tree status, and optional `jj status` output observed
when the bundle was generated; `make promotion-evidence-verify` checks that its
git revision agrees with `SUMMARY.txt`. `PROMOTION_OPEN_WORK.txt` records the
known blockers that still keep `libghostty-vt` opt-in. `BUNDLE_MANIFEST.txt`
records SHA-256 hashes for the evidence files in the bundle using stable
relative artifact names, and `SUMMARY.txt` records bundle artifacts by those
same relative names so copied or downloaded bundles remain verifiable.
It runs `make promotion-evidence-verify` before printing the artifact list.
Run `make promotion-evidence-verify` directly when reviewing an existing bundle
without regenerating the native build and packaging sample; it validates the
bundled package provenance through `make packaging-provenance-manifest-verify`
and bundled archive bytes through `make packaging-archive-verify`. For
GitHub-generated bundles, the verifier also requires concrete run, ref, SHA,
and runner fields and checks that `github_sha` matches the bundled revision. Use
`make PROMOTION_EVIDENCE_DIR=/path/to/artifact promotion-evidence-verify` when
checking a downloaded artifact outside `target/promotion-evidence`. The manual
CI workflow performs the same downloaded-artifact verification after upload.

`make source-fetch-provenance-sample` writes the active source mode and locked
`libghostty-vt` Cargo package records without inspecting Ghostty source. Use it
when updating source-fetch evidence or comparing pinned-fetch versus local
`GHOSTTY_SOURCE_DIR` samples.
`make source-fetch-provenance-verify` checks an existing `SOURCE_FETCH.txt`
against the current `Cargo.lock`, toolchain records, source-mode fields, and
policy note without regenerating the report. Override `SOURCE_FETCH_REPORT`
when checking a copied or bundled report.
`make source-fetch-offline-probe` checks whether the opt-in
`nmux-core --features libghostty-vt` build can compile from current caches with
`CARGO_NET_OFFLINE=true`. Treat this as cache-present evidence only, not
cold-checkout, CI cache-miss, or source-policy evidence.
`make source-fetch-offline-probe-verify` checks the existing `OFFLINE_PROBE.txt`
and `OFFLINE_PROBE.log` for the expected offline mode, target dir, command,
passing result, and generated `nmux-core` test binary without rerunning the
probe. Override `SOURCE_FETCH_OFFLINE_PROBE_REPORT` and
`SOURCE_FETCH_OFFLINE_PROBE_LOG` when checking copied or bundled evidence.

`make packaging-sample` prints the same toolchain context, builds default and
opt-in release binaries in separate target directories, and reports artifact
sizes plus binary versions for packaging evidence.
`make packaging-layout-sample` stages the opt-in binaries, wrapper scripts, and
`libghostty-vt` runtime libraries under `target/packaging-libghostty-vt/package`
and verifies the wrapped binaries can run from that local layout.
`make packaging-layout-verify` checks an existing staged layout without
rebuilding: required files, wrapper scripts, package metadata, native runtime
library presence, and wrapped binary version commands. Override
`PACKAGING_LAYOUT` when checking a copied or alternate staged layout.
`make packaging-provenance-sample` writes a manifest for that staged layout with
file hashes, package metadata, toolchain/source mode, dependency tree, native
runtime-library artifacts, and dynamic dependency output.
`make packaging-provenance-verify` regenerates that manifest and asserts the
required toolchain, source-mode, locked native-VT package, staged-file,
runtime-library, per-binary `libghostty-vt` dynamic-dependency, and cargo-tree
records are present.
`make packaging-provenance-manifest-verify` checks an existing manifest without
rebuilding; override `PACKAGING_PROVENANCE_MANIFEST` when checking copied or
bundled `PACKAGE_PROVENANCE.txt` evidence.
`make packaging-archive-sample` archives the staged layout, writes an archive
SHA-256 file, extracts it, verifies the wrapped binaries from the extracted
layout, and then runs the no-rebuild archive verifier.
`make packaging-archive-verify` validates an already-produced archive plus
sidecar hash without rebuilding. Override `PACKAGING_ARCHIVE` and
`PACKAGING_ARCHIVE_SHA256` when checking an artifact outside the default
`target/packaging-libghostty-vt/archive` path. It reuses
`make packaging-layout-verify` against the extracted layout before accepting
the archive.
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
