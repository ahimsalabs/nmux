# 0046: Attach Session Target Envelope

Status: Accepted

Date: 2026-05-26

## Context

`AttachRequest` describes actor identity, attach mode, focused pane, and known
surface versions. It does not own session selection. The envelope already has a
`session_id` field, but local attach frames previously filled it with the
placeholder `local`, making it impossible for a daemon to distinguish "attach to
the default session" from "attach to this named session."

GUI session switching needs an attach-level session target before the daemon can
route clients across multiple daemon-owned sessions.

## Decision

Client attach frames use `Envelope.session_id` as an optional target session.
An absent session ID means attach to the daemon's default/current session. A
present non-empty session ID must match the daemon-owned session in the current
single-session server path, or the daemon returns `ErrorCode::SessionNotFound`
before selecting a pane or sending workspace state.

The `AttachRequest` table remains unchanged.

## Consequences

Existing clients that omit `Envelope.session_id` continue to attach to the
daemon's current session. CLI clients now populate the envelope from
`--session`, so a mismatch is rejected by protocol error instead of being
detected later by comparing the returned workspace session.

This does not add multi-session daemon routing by itself. It establishes the
wire contract that a future session registry router can use when accepting a
client.
