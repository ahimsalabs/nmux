---
status: accepted
date: 2026-05-26
---

# Session Inventory Control Command

## Context

The ratatui session menu needs to show daemon-owned sessions, not tabs inside
the currently attached session. The existing workspace snapshot is scoped to
one session, so clients cannot discover other named sessions behind the same
listener.

## Decision

Append `SessionList = 6` to `ControlCommandKind` and add
`SessionInventorySnapshot` as an envelope body. The inventory is a daemon
registry view: it carries the active/default session id and one item per known
session, with each item exposing a stable `session_id` and display `title`.

Single-session handlers may return a one-item inventory for compatibility.
Registry-aware daemons intercept the command before it reaches an actor so they
can report every session behind the listener.

## Consequences

Scriptable clients and UI clients can use one protocol command to populate a
real session menu. Switching live attachments still requires client-side
reattach to the selected session, but discovery no longer depends on guessing
or misusing tab state.
