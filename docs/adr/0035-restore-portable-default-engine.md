# ADR 0035: Restore Portable Default Engine

## Status

Superseded by ADR 0034.

## Date

2026-05-24

## Context

ADR 0034 promoted `libghostty-vt` to the default Cargo feature and daemon
engine. That made normal development exercise the terminal-correct backend, but
it also made `nix run github:ahimsalabs/nmux` build the native Ghostty VT
dependency inside the flake package derivation.

The `libghostty-vt-sys` build script fetches a pinned Ghostty checkout unless
`GHOSTTY_SOURCE_DIR` is supplied. The flake app/package derivation is stricter
than the GitHub `nix develop . -c just check` path and did not supply a
Nix-fetched source, so public `nix run` failed before producing a runnable
binary.

## Decision

This reversal is no longer active. ADR 0034 is restored as the accepted
decision: `libghostty-vt` is the default Cargo and daemon engine, and the flake
package supplies `GHOSTTY_SOURCE_DIR` from a pinned Nix source derivation.

The GitHub check workflow must build the actual flake package and run the
packaged binary, not only run commands inside the Nix development shell.

## Consequences

The short-lived interim package fallback is retained only as historical context
for why the package derivation now owns the Ghostty source input explicitly.
