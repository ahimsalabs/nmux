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
```

The target prints `make toolchain-info`, validates the optional native-VT
toolchain preflight, builds default release `nmux` and `nmuxd` binaries into
`target/packaging-default`, builds opt-in `--features libghostty-vt` release
binaries into `target/packaging-libghostty-vt`, then prints artifact paths,
byte sizes, discovered `libghostty-vt` dynamic-library artifacts, and
`--version` output. The opt-in build uses `GIT_CONFIG_GLOBAL=/dev/null` and the
same `GHOSTTY_SOURCE_DIR` validation as the terminal-correctness gate. If a
built binary cannot run its version check, the target exits nonzero after
reporting all version-check failures.

Record successful or failed samples in
[default-engine-promotion.md](default-engine-promotion.md) with host, source
mode, cache state, command, artifact sizes, and result. A passing local
packaging sample proves the release binaries build in that environment; it does
not answer install paths, signing/notarization, update channels, target support,
or native-library provenance by itself.

Current local Darwin evidence shows the default release binaries run, but the
opt-in `libghostty-vt` release binaries fail at runtime because
`@rpath/libghostty-vt.dylib` is not discoverable and the binaries have no
`LC_RPATH`. This must be resolved before native-VT binaries are treated as
shippable.

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
