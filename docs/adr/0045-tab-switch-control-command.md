# 0045: Tab Switch Control Command

Status: Accepted

Date: 2026-05-26

## Context

The ratatui top menu can create tabs, but it cannot switch to another existing
tab through the same backend-owned workspace mutation path. The workspace tree
snapshot already carries all tab nodes, so switching only needs a command kind
that targets an existing tab ID.

## Decision

Append `TabSwitch = 4` to `ControlCommandKind`. The command uses the existing
`tab_id` field, mutates the active tab through the session actor when present,
and replies with the existing `WorkspaceTreeSnapshot`.

## Consequences

GUI and TUI clients can implement clickable tab/session menus without adding a
new request table or bypassing daemon-owned state. This remains tab switching
inside the connected daemon session; cross-daemon session inventory is still a
separate transport/session-management problem.

## Licensing Notes

This is an nmux protocol extension and does not copy code from external
terminal multiplexers.
