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

## Default-Engine Promotion Packaging Questions

Before a later ADR can make `libghostty-vt` the default engine or a regular CI
requirement, packaging work must answer:

- which target triples are supported for binaries that include the native
  Ghostty VT dependency;
- whether the native VT library is statically included, dynamically linked, or
  supplied by a platform package/artifact cache;
- how source-fetch policy, offline builds, cache provenance, and license review
  are handled for the native Ghostty source;
- whether `nmuxd --terminal-engine libghostty-vt` is enabled in shipped
  binaries or reserved for developer builds;
- how release checks map to `make check`, `make check-ghostty-vt`, and
  `make check-all`.

Until those answers are recorded, packaged/default builds should keep the
`interim` engine as the normal path and treat `libghostty-vt` as opt-in
correctness work.
