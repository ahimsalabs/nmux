# 0050: Daemon-Owned Pane Viewports

Status: Accepted

Date: 2026-05-27

## Context

The previous scrollback design split terminal display into two protocol objects:
`PaneSurfaceSnapshot`/`PaneSurfacePatch` for the active visible surface and
`ScrollbackFetch`/`ScrollbackChunk` for historical ranges. Live scrolling then
required the client to combine surface metadata, local scroll offsets, pending
surface updates, and separately fetched history chunks.

That split made reconnect and live scroll fragile. A client could be scrolled
into history while new output arrived, receive a surface update for the active
tail, and then render a blank or stale historical view until another explicit
scrollback fetch caught up.

Ghostty's terminal model keeps one timeline and renders a selected viewport:
active bottom, top, pinned line, or a delta from the current viewport. nmux
should follow that ownership boundary.

## Decision

Protocol v2 adds daemon-owned pane viewport messages:

- `PaneViewportIntent` is the client request for an active, top, pinned, or
  delta viewport.
- `PaneViewportSnapshot` is a full renderable viewport over the pane timeline.
- `PaneViewportPatch` is reserved for incremental updates against a viewport
  version.
- `AttachRequest.known_viewports` lets reconnecting clients advertise cached
  viewport versions independently from older surface versions.

The daemon owns viewport selection, clamping, padding, cursor visibility, and
the mapping from absolute timeline lines to rendered rows. Live clients render
the viewport rows returned by the daemon instead of splicing active surface rows
with separately fetched scrollback chunks.

This is a breaking protocol change. nmux does not preserve compatibility with
protocol v1 while this branch is still pre-release.

## Consequences

The live scroll path has one authoritative row source per pane: the daemon's
viewport snapshot. This removes a class of blank or missing-scrollback bugs
caused by client-side range stitching.

The old `ScrollbackFetch` and `ScrollbackChunk` bodies can remain temporarily
for CLI/debug paths, but they are no longer the live viewport architecture.
They should be removed once all user-facing scrollback commands consume
viewport snapshots.

Viewport patches can be implemented after the snapshot path is stable. Until
then, snapshots are the correctness path and patch support is an optimization.
