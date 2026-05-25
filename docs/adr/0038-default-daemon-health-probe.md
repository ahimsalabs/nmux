# ADR 0038: Default Daemon Health Probe

## Status

Accepted.

## Date

2026-05-25

## Context

Bare `nmux` starts or reuses a shared default daemon for the normal interactive
terminal experience. Reuse is important because a multiplexer should preserve
the workspace across client exits, but it also means a later client can connect
to a daemon whose active pane process has already exited.

An exited pane should remain attachable so users can inspect its last surface
and scrollback. Returning a structured server error during output polling makes
the default path look broken: the shared daemon is reachable, but attach fails
with `pane process is not running`.

## Decision

Local PTY output reads drain any buffered output and then return quiet for an
exited pane. Missing pane process state remains an error, but a known pane whose
child process has exited is a valid terminal-history object.

Bare default mode also performs a cheap health probe before reusing an existing
default socket. The probe opens the socket, sends a read-only attach request,
and reads the attach response. If the daemon returns the structured server error
whose message reports that the pane process is not running, default mode treats
that daemon as unhealthy for interactive reuse, sends a best-effort session kill,
unlinks the shared socket, and starts a fresh persistent daemon.

This probe is limited to bare default mode. Explicit `--socket`, `--live`, TCP,
and scripted commands do not silently replace a daemon selected by the user.

## Consequences

The default `nmux` experience recovers from stale shared daemons without asking
users to know where the socket lives or run a cleanup command first.

The health probe intentionally relies on the existing FlatBuffers attach and
error-frame contract rather than adding a new health-check protocol. That keeps
the decision local to default launcher policy and avoids a schema change.

An exited pane in a healthy daemon can still be displayed. The restart path is a
compatibility fallback for older or inconsistent daemons that report exited
panes as attach-time output polling failures.
