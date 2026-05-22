# 0004: Presence And Attach Modes

Status: Proposed

Date: 2026-05-21

## Context

nmux is intended to support multiple clients attached to the same backend-owned workspace. M5 in [docs/roadmap.md](../roadmap.md) requires presence, actor IDs, and read-only vs read-write attach modes.

The current local prototype already sends `InputEvent` messages with an actor ID, but attach mode is not modeled. Without attach mode, the daemon cannot distinguish a controller from a spectator, and read-only clients would rely on frontend restraint rather than server-side policy.

Presence is collaboration metadata. It should tell clients who is attached and what they can do, but it should not own terminal state or mutate pane surface versions.

## Decision

Model actors and attach modes explicitly in the protocol.

Use these attach modes:

- `ReadOnly`: the actor may receive workspace, surface, scrollback, and presence updates, and may request read-only data such as scrollback ranges. The actor may not send pane input or resize/control intents.
- `ReadWrite`: the actor may receive updates and send pane input/control events allowed by future policy.

Presence updates are session-scoped and may include a focused pane ID. They identify actors and their attach mode, but they do not change terminal state versions.

Authorization belongs to the attached actor/session, not to each individual `InputEvent`. The daemon must reject pane input and resize/control intents from read-only actors even if a client sends them.

## Consequences

The FlatBuffers schema should add an `AttachMode` enum and a `PresenceUpdate` body. A later public `Attach` body can replace the local attach prelude when the attach metadata shape is ready.

The local prototype can extend its temporary attach prelude with actor ID and attach mode while still treating the FlatBuffers schema as the durable contract for presence updates.

Read-only clients must still be able to fetch scrollback ranges. Read-only does not mean disconnected or unable to inspect history.

M5 should first prove actor/mode/presence semantics in the current single-client local skeleton before changing the accept loop for simultaneous clients.

## Compatibility

This extends the FlatBuffers envelope union append-only. Existing message fields and union variants must not be removed or reordered.
