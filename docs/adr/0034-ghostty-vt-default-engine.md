# ADR 0034: Ghostty VT Default Engine

## Status

Accepted.

## Date

2026-05-24

## Context

ADR 0023 kept `interim` as the default after M13 because native build cost,
source-fetch policy, regular CI, and packaging were not yet accepted. The
promotion path now pins `libghostty-vt` and `libghostty-vt-sys` to an exact
upstream Git revision, enables static Ghostty VT linking, and makes the
default check path build and test with the native VT dependency.

`interim` still has value as a small fallback for debugging and for
no-default-features builds, but it is not terminal-correct enough to remain the
normal product path.

## Decision

Make `libghostty-vt` the default Cargo feature and the default daemon runtime
engine when that feature is built. Keep `nmux daemon --terminal-engine interim`
as an explicit legacy/debug choice, and keep `cargo test --no-default-features`
covered by `make check-interim`.

Resize-driven surface dimension changes require a full surface snapshot rather
than a patch, because `PaneSurfacePatch` does not carry new pane dimensions.
Patch row indices must also be clamped to the current pane height when a VT
backend reports transient extra rows.

## Consequences

Default builds require the Ghostty VT native toolchain, including the pinned
Zig version. Default CI and local smoke now exercise Ghostty behavior rather
than the interim text surface.

The interim backend remains available without changing the wire protocol. Any
future removal of interim should be a separate decision after the fallback no
longer provides useful isolation for tests or debugging.
