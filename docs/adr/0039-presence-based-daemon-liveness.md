# ADR 0039: Presence-Based Daemon Liveness

## Status

Accepted.

## Date

2026-05-25

## Context

Bare `nmux` should behave like a normal multiplexer entrypoint: start the
default daemon when none exists, otherwise attach to the existing daemon. The
client also needs to detect stale sockets and default daemons whose interactive
target is no longer usable.

ADR 0038 used an attach-plus-resize probe for this. That kept the decision local
to the launcher, but it coupled daemon health to terminal side effects. Resize
intents should mean "resize this pane", not "tell me whether this daemon is
safe to reuse".

Presence is already nmux's control-plane actor mechanism. A liveness check is a
control-plane question: can the daemon respond, and does it have a usable active
session target for an interactive attach?

## Decision

Use `PresenceUpdate` for side-effect-free daemon liveness.

Two presence kinds are added:

- `HealthProbe`: a short-lived client request that asks whether the daemon and
  selected pane are usable for default interactive attach.
- `Heartbeat`: a daemon response confirming the session target is usable.

Health probes do not create visible joined/left presence, do not participate in
smallest-client resize policy, do not change focus, and do not require surface or
scrollback state. A daemon that is reachable but cannot satisfy the requested
target returns a structured `Error` frame instead of accepting an attach or
triggering a terminal side effect.

Bare default mode uses this control-plane probe before reusing an existing
default socket. Connect failures, invalid frames, closed sockets, and retryable
health errors cause the launcher to replace the default daemon socket and start a
fresh persistent daemon. Explicit sockets, TCP, scripted commands, and explicit
live attach do not silently replace user-selected daemons.

## Consequences

Default daemon reuse no longer depends on resize behavior, shell signal
handling, PTY timing, or a pane process that exits when probed.

Presence now carries both visible multiplayer membership and invisible liveness
traffic. Implementations must keep `HealthProbe` and `Heartbeat` out of visible
presence membership and resize calculations.

This supersedes ADR 0038's resize-probe mechanism. Resize remains a terminal
control intent only.
