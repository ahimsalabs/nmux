# ADR 0018: libghostty-vt Default And CI Gate

## Status

Accepted.

## Date

2026-05-22

## Context

ADR 0013 imported `libghostty-vt` as an optional backend terminal engine. M13
has since expanded that engine from a smoke path into the main terminal-state
correctness path: it extracts cursor state, modes, style-separated rows,
scrollback, color state, semantic metadata, dirty state, Kitty placeholders,
hyperlink presence, and mode-aware input encoding while preserving nmux state
sync semantics.

The implementation is now broad enough that a narrow name-filtered smoke check
is not a sufficient opt-in gate. Feature-sensitive behavior can live in ordinary
tests: CLI argument parsing, local attach behavior, cached client state, protocol
decoding, and shared session invariants all need to compile and pass under
`--features libghostty-vt`.

The native build is still materially different from the default path.
`libghostty-vt-sys` fetches a pinned Ghostty source tree unless
`GHOSTTY_SOURCE_DIR` is provided, and the native build currently requires the
Zig version pinned by the Nix development shell. That build cost and packaging
shape should not enter the default developer loop by accident.

## Decision

Keep `libghostty-vt` opt-in for default development, regular `make check`, and
the default `nmuxd --terminal-engine interim` path.

Strengthen the opt-in verification gate instead:

- `make check-ghostty-vt` runs the full `nmux-core` and `nmux-cli` package test
  suites with `--features libghostty-vt` and `RUST_TEST_THREADS=1`.
- The gate keeps `GIT_CONFIG_GLOBAL=/dev/null` so local Git URL rewrites do not
  break the pinned HTTPS Ghostty fetch.
- M13 changes that touch terminal extraction, terminal modes, structured input
  encoding, live attach behavior, cached client state, or feature-sensitive CLI
  parsing should run `make check-ghostty-vt` before commit.

Promoting `libghostty-vt` into regular CI or making it the documented/default
engine requires a later decision. That decision should have evidence for:

- acceptable native build time in CI and local development;
- stable Zig/toolchain provisioning outside the current Nix shell;
- a packaging story for binaries that include the native Ghostty VT library;
- reliable source-fetch or vendoring policy for `libghostty-vt-sys`;
- passing full default and feature-enabled gates without broad flakes.

## Consequences

The repository now has a stronger opt-in correctness gate without forcing the
native Ghostty/Zig build onto every contributor or every default test run.

The default engine remaining `interim` is a build and packaging decision, not a
claim that the interim text surface is terminal-correct. User-facing docs should
continue to label the interim renderer as temporary and non-VT-correct.

Because the opt-in gate is full-suite rather than name-filtered, feature-only
regressions in ordinary code paths are more likely to fail before a commit.

## Licensing And Compatibility

This decision does not change licensing posture. The optional path may use the
MIT/Apache Rust crates and MIT Ghostty source fetched by `libghostty-vt-sys` or
provided through `GHOSTTY_SOURCE_DIR`. Do not copy GPL or AGPL terminal emulator
code into nmux, and do not hand-copy generated Ghostty bindings into the core.
