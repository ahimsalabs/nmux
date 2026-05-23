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
see [ci.md](ci.md) for the workflow shape.

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
nix develop . -c make promotion-local-sample
nix develop . -c make packaging-sample
nix develop . -c make packaging-layout-sample
nix develop . -c make packaging-provenance-sample
nix develop . -c make packaging-archive-sample
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

`make promotion-local-sample` runs `make promotion-sample` and then
`make packaging-archive-sample` with clear section headers. Use it for a local
evidence pass before updating the promotion tracker with both validation and
packaging/archive results.

`make packaging-sample` prints the same toolchain context, builds default and
opt-in release binaries in separate target directories, and reports artifact
sizes plus binary versions for packaging evidence.
`make packaging-layout-sample` stages the opt-in binaries, wrapper scripts, and
`libghostty-vt` runtime libraries under `target/packaging-libghostty-vt/package`
and verifies the wrapped binaries can run from that local layout.
`make packaging-provenance-sample` writes a manifest for that staged layout with
file hashes, toolchain/source mode, dependency tree, native runtime-library
artifacts, and dynamic dependency output.
`make packaging-archive-sample` archives the staged layout, writes an archive
SHA-256 file, extracts it, and verifies the wrapped binaries from the extracted
layout.

Use [toolchain.md](toolchain.md), [source-fetch-policy.md](source-fetch-policy.md),
and [packaging.md](packaging.md) when the work touches non-Nix setup, Ghostty
source policy, or release binaries.
Use the manual CI promotion-sample job when collecting CI evidence; a normal
pull-request run remains default-engine-only.
