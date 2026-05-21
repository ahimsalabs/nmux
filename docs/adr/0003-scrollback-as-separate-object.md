# 0003: Scrollback As A Separate Object

Status: Proposed

Date: 2026-05-21

## Context

ADR 0001 says `nmuxd` owns authoritative terminal state and clients synchronize versioned state objects instead of replaying raw PTY bytes.

The current prototype synchronizes a workspace tree and a pane surface. The pane surface represents the current visible terminal state. That is not enough for a portable workspace: reconnecting clients also need historical output, and clients may request different historical ranges without forcing every attached frontend to keep the same viewport.

[WORK.md](../../WORK.md) already separates:

- `PaneSurface` for the current visible or alternate screen
- `PaneScrollback` for durable historical line storage
- `PaneViewport` for a client-specific visible range
- `PanePatch` for current-screen updates
- `ScrollbackChunk` for lazy historical range transfer

## Decision

Model scrollback as a separate synchronized object from the pane surface.

The pane surface remains the object for the current visible or alternate screen. Scrollback is a range-addressed object owned by `nmuxd`, versioned independently enough that clients can request historical ranges lazily.

The protocol should add request/response bodies for scrollback ranges rather than folding all historical rows into `PaneSurfaceSnapshot`.

Initial shape:

- `ScrollbackFetch`: client asks for a pane ID and line range.
- `ScrollbackChunk`: daemon replies with pane ID, scrollback version, range start, and rows.

## Consequences

Clients can reconnect and fetch only the historical ranges they need.

Mobile or small-screen clients can keep narrow local viewports without changing the PTY size or forcing other clients to fetch the same history.

The daemon must maintain consistency between visible surface rows and scrollback rows when terminal output advances.

M4 should start with a small static model and tests before introducing real PTY output or libghostty-backed history extraction.

## Compatibility

This extends the FlatBuffers envelope union. It must be done append-only: add new tables and new union variants; do not reorder or remove existing fields.
