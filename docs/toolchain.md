# Toolchain Notes

The supported development path is the Nix shell:

```sh
nix develop . -c make check
nix develop . -c make check-ghostty-vt
nix develop . -c make check-all
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

## Non-Nix Equivalents

The non-Nix path is documented as a requirements checklist, not as promotion
evidence. Before it can justify making `libghostty-vt` a default engine or
regular CI requirement, it still needs platform-specific setup commands,
timings, cache behavior, and packaging decisions recorded in
[the default-engine promotion tracker](default-engine-promotion.md).

A non-Nix environment must provide:

- a Rust toolchain new enough for Cargo workspace resolver 3 and edition 2024;
- `flatc` compatible with the pinned Rust `flatbuffers = "=25.12.19"` crate;
- `make`;
- Zig 0.15 when building or testing `--features libghostty-vt`;
- network or local-source policy for the `libghostty-vt-sys` Ghostty source
  fetch.

For the default engine, the command shape is:

```sh
make check
```

For the opt-in VT engine, the command shape is:

```sh
GIT_CONFIG_GLOBAL=/dev/null make check-ghostty-vt
```

`GIT_CONFIG_GLOBAL=/dev/null` is not a semantic nmux requirement. It keeps local
Git URL rewrite rules from changing the HTTPS source fetch used by
`libghostty-vt-sys`. If an environment uses a pre-fetched Ghostty checkout, set
`GHOSTTY_SOURCE_DIR` according to the `libghostty-vt-sys` build path and record
that source policy before using the result as promotion evidence.
See [source-fetch-policy.md](source-fetch-policy.md) for the current opt-in
policy and the remaining source-fetch decisions for packaged/default builds.
See [packaging.md](packaging.md) for binary distribution questions that remain
outside the supported development shell.
