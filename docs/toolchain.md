# Toolchain Notes

The supported development path is the Nix shell:

```sh
nix develop . -c make check
nix develop . -c make check-ghostty-vt
nix develop . -c make check-all
nix develop . -c make promotion-sample
nix develop . -c make packaging-sample
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
See [source-fetch-policy.md](source-fetch-policy.md) for the current opt-in
policy and the remaining source-fetch decisions for packaged/default builds.
See [packaging.md](packaging.md) for binary distribution questions that remain
outside the supported development shell.
