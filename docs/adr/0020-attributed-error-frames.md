# ADR 0020: Attributed Error Frames

## Status

Accepted.

## Date

2026-05-22

## Context

M12 and M13 made more client intents asynchronous and pane-scoped: scrollback
fetches, resize intents, structured input, current-surface reattach input, and
mode-gated focus or mouse forwarding can all fail after a client has already
attached.

Before this decision, `Error` carried only a code, message, and retryable flag.
That was enough for a human CLI message, but not enough for a client maintaining
multiple pane states or multiple outstanding input events. A client had to parse
human-oriented reason strings to determine which pane or input event failed.

## Decision

Extend `Error` with optional structured attribution:

- `pane_id`, the pane-scoped request that failed when the daemon can attribute
  the error to a pane; and
- `input_seq`, the originating `InputEvent.input_seq` for input failures.

`input_seq` remains zero for non-input failures such as scrollback fetch errors,
resize failures, and generic host or protocol errors. Existing `code`,
`message`, and `retryable` semantics remain unchanged, and the CLI continues to
print the server-provided message for humans.

The local daemon sets `pane_id` for pane-scoped `PaneNotFound`,
`PermissionDenied`, stale scrollback, input encoding, host input, and host
resize errors. It sets `input_seq` when the failed request originated from an
`InputEvent`, including input events targeting an unknown pane.

## Consequences

Clients can correlate a failure with cached pane state or an input event without
parsing reason strings. Human-readable error messages remain the compatibility
fallback.

This does not add request IDs for all client control frames. Resize and
scrollback requests still rely on envelope sequence plus pane/range context
until a broader request/response correlation model is needed.
