---
status: accepted
date: 2026-05-26
---

# Session New Control Command

## Context

The ratatui session menu needs a daemon-visible operation for creating a named
session behind an existing listener. Reusing `TabNew` keeps the UI usable as an
interim behavior, but it prevents clients from distinguishing a new window/tab
inside the current session from a new daemon-owned session.

## Decision

Append `SessionNew = 5` to `ControlCommandKind`. The command uses the existing
`session_id` field as the required new session id and the existing `title`
field as an optional initial title. Daemon registry routing is responsible for
intercepting this command before it reaches a single-session actor.

Single-session control handlers reject `SessionNew`; they do not silently turn
it into `TabNew` because that would hide missing registry support from clients.

## Consequences

Clients can learn and send one stable command shape for session creation. The
daemon can add registry-backed creation without another protocol change, and
older single-session paths fail explicitly until they are routed through a
registry-aware handler.
