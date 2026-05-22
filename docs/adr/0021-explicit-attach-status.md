# ADR 0021: Explicit Attach Status

## Status

Accepted.

## Date

2026-05-22

## Context

Reconnect attach can legitimately have no pane surface snapshot or patch to
send: the client may already hold the current surface version. Previously the
local client discovered this by setting a short socket read timeout after
`WorkspaceTreeSnapshot` and `PresenceUpdate` and treating a timeout as "no
surface update."

That made attach completion implicit and timing-dependent. It also meant a
one-shot or follow client had to choose cached scrollback preconditions before
the daemon confirmed which pane was actually attached.

## Decision

Add an explicit `AttachStatus` envelope body after `PresenceUpdate` and before
any optional surface frame. It carries:

- `pane_id`, the pane the daemon attached;
- `surface_version`, the daemon's current version for that pane; and
- `surface_state`, one of `Current`, `Snapshot`, or `Patch`.

When `surface_state` is `Current`, no surface frame follows and the client can
immediately proceed to post-attach input or scrollback requests. When it is
`Snapshot` or `Patch`, the next frame is the corresponding
`PaneSurfaceSnapshot` or `PaneSurfacePatch`.

Clients use the status pane ID as the post-attach control target and as the key
for cached scrollback range/version preconditions.

## Consequences

Attach no longer depends on a timeout to distinguish "current surface" from a
slow server. One-shot, follow, and live attach share the same explicit barrier
before post-attach controls.

This does not add simultaneous multi-pane attach. The status describes the
single pane selected by the current local attach flow. Broader multi-pane
subscription semantics remain a later protocol decision.
