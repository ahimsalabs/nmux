# ADR 0016: Terminal Color State

## Status

Accepted.

## Date

2026-05-22

## Context

`libghostty-vt` exposes terminal render colors through the safe render-state
API: default foreground, default background, optional explicit cursor color, and
the active palette. nmux already resolves styled cell runs into RGBA style-table
entries, but default colors and palette changes affect rendering even when no
cell text changes.

Hyperlinks, image placement, and richer damage are separate protocol decisions.
Their current safe APIs expose presence or row-level state, not enough payload
or lifetime information for stable nmux objects.

## Decision

Add `TerminalColorState` to the protocol and carry it on
`PaneSurfaceSnapshot`, `PaneSurfacePatch`, and `ScrollbackChunk`.

`TerminalColorState` stores default foreground/background RGBA, optional cursor
RGBA via a boolean presence flag, and the active palette as RGBA entries.
Backend engines populate this state when it is available. Old client cache files
default missing color state to zero/empty values.

Color changes require `PatchKind::FullRefreshRequired`. This avoids freezing a
partial palette-diff protocol before renderer requirements are clearer.

## Consequences

Clients can preserve backend-observed default colors and palettes alongside
style runs without replaying PTY bytes. Incremental color patching remains
withheld until nmux has a concrete renderer-driven need for it.
