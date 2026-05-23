# 0008: FlatBuffers Attach Request

Status: Accepted

Date: 2026-05-21

## Context

ADR 0004 introduced presence and attach modes, but intentionally left the first attach handshake as a local text prelude until the metadata shape was clearer.

That prelude now carries stable protocol concepts:

- actor identity;
- read-only vs read-write attach mode;
- focused pane intent;
- known pane surface versions used for snapshot, patch, or no-update reconnect decisions.

The local client also persists pane surface state and uses known versions across reconnects. Keeping that negotiation outside FlatBuffers would make non-local clients, adapters, and future native frontends implement a second protocol before they can receive server-owned terminal state.

## Decision

Attach negotiation should become a public FlatBuffers message. The local text prelude is deprecated scaffolding and should be replaced by an `AttachRequest` envelope body.

The first public request should be minimal and state-sync oriented:

- actor ID, user ID, display name, attach mode, and focused pane ID;
- known pane surface versions keyed by pane ID.

The server should continue to answer with the existing state objects:
`WorkspaceTreeSnapshot`, `PresenceUpdate`, `PaneSurfaceSnapshot` or
`PaneSurfacePatch`, and `ScrollbackChunk` when requested. ADR 0021 later adds
`AttachStatus` as the explicit attach barrier between presence and any optional
surface frame. `AttachStatus` covers the immediate need for accepted pane ID and
current/snapshot/patch surface state; broader connection capabilities still
need a separate decision if they become necessary.

## Consequences

The schema change should be append-only:

- add a `KnownPaneSurfaceVersion` table;
- add an `AttachRequest` table;
- append `AttachRequest` to `EnvelopeBody`.

Do not reorder existing enum values, union variants, tables, or fields.

The Rust local transport can keep its domain-level `AttachRequest` and `KnownSurfaceVersion` structs, but encode and decode them through FlatBuffers rather than a bespoke text payload. Once that migration lands, `docs/running.md` should stop describing the attach metadata as local-only.

This does not promote host specs, process launch metadata, auth tokens, transport capabilities, or scrollback subscriptions into the public attach request yet. Those need separate decisions when their shape is concrete.

## Compatibility

This supersedes the ADR 0004 note that a later public attach body can replace the local attach prelude. The replacement should happen in a separate implementation commit after this ADR.

ADR 0021 supersedes the earlier possibility of a generic `AttachAccepted`
acknowledgement for current local attach completion by adding `AttachStatus`.
