# ADR 0035: Restore Portable Default Engine

## Status

Accepted.

## Date

2026-05-24

Supersedes ADR 0034.

## Context

ADR 0034 promoted `libghostty-vt` to the default Cargo feature and daemon
engine. That made normal development exercise the terminal-correct backend, but
it also made `nix run github:ahimsalabs/nmux` build the native Ghostty VT
dependency inside the flake package derivation.

The `libghostty-vt-sys` build script fetches a pinned Ghostty checkout with
`git` unless `GHOSTTY_SOURCE_DIR` is supplied. The flake app/package derivation
is stricter than the GitHub `nix develop . -c just check` path and did not
provide `git`, so public `nix run` failed before producing a runnable binary.

## Decision

Restore `interim` as the default Cargo and daemon engine. Keep
`libghostty-vt` available only through the explicit Cargo feature and
`--terminal-engine libghostty-vt`.

The GitHub check workflow must build the actual flake package and run the
packaged binary, not only run commands inside the Nix development shell.

## Consequences

`nix run github:ahimsalabs/nmux` returns to a portable default package that does
not require the Ghostty source-fetching build script.

The VT-correct engine remains available for targeted validation and development
through `just check-ghostty-vt`. Promoting it again requires a packaging design
that works for the public flake package, not only for a Nix dev shell or cached
CI workspace.
