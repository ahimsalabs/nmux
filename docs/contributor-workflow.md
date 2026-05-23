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
nix develop . -c make check-all
```

Record the host, command, result, timing, cache state, source-fetch mode, and
any CI or packaging context in
[docs/default-engine-promotion.md](default-engine-promotion.md). A passing local
`make check-all` sample is useful evidence, but it does not change the default
engine by itself.

Use [toolchain.md](toolchain.md), [source-fetch-policy.md](source-fetch-policy.md),
and [packaging.md](packaging.md) when the work touches non-Nix setup, Ghostty
source policy, or release binaries.
