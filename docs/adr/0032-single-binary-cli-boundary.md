# 0032: Single Binary CLI Boundary

Status: Accepted

Date: 2026-05-24

## Context

nmux has grown as two binaries in `nmux-cli`: `nmuxd` for the daemon and
`nmux` for attach/control clients. That split made early implementation
straightforward, but it gives users two command surfaces for one tool and makes
`nix run github:ahimsalabs/nmux` pick only one default binary.

The client also accumulated low-level attach flags such as `--live`,
`--stdin-bytes`, `--redraw`, and TCP transport flags next to pane and tab
control commands. Issue #6 defines the user-facing direction: `nmux` is the
primary command, `nmux daemon` owns server startup, and remote attach should
look like ssh or mosh (`nmux host`).

## Decision

Make `nmux` the primary user-facing binary and move daemon implementation into
a shared `nmux-cli` module. `nmux daemon ...` and the compatibility `nmuxd`
binary call the same daemon implementation; `nmuxd` stays as a thin shim during
the transition.

The CLI grows noun-verb commands around the protocol that already exists:

- bare interactive `nmux` attaches to the default socket when it exists and
  otherwise starts a private live shell;
- `--session NAME`, `nmux attach NAME`, and `nmux new NAME` target the one
  named session currently owned by a daemon;
- `nmux daemon ...` runs the daemon in the foreground;
- `nmux version [--json]` mirrors version flags without binding a socket;
- `nmux pane read` is the preferred spelling for `pane snapshot`;
- `nmux pane ls`, `nmux tab ls`, and `nmux ls` expose the currently visible
  daemon state without adding a new protocol object;
- `nmux send-keys [-t PANE] KEYS...` provides a tmux-familiar text-input alias.

Direct TCP remote attach is exposed as `nmux [user@]HOST[:PORT]`. A host with
no port uses the current direct TCP default port. Token authentication uses
`--token`, the existing `--tcp-token` compatibility spelling, or `NMUX_TOKEN`.
The older explicit `--tcp HOST:PORT` path remains for scripts.

SSH bootstrap is not implemented by this ADR. It remains the intended default
remote bootstrap because it can use existing SSH config and agents, but it
requires a separate design for remote daemon discovery, socket tunneling or
direct transport handoff, token lifetime, and failure cleanup.

## Consequences

Package and Nix defaults can point users at `nmux` without hiding daemon
functionality. Existing scripts that call `nmuxd`, `--tcp-listen`, or
`--tcp-token` keep working during the transition.

The current list commands are intentionally scoped to state already present in
the attach snapshot. They do not claim multi-session discovery or full tab
enumeration until the protocol and daemon state grow those concepts.

Remote positional attach is direct TCP for now, not a hardened production
remote story. It inherits ADR 0029's token-authenticated transport limits and
does not supersede ADR 0014's longer-term transport identity work.

## Licensing Notes

The CLI consolidation is an internal Rust refactor plus command parsing around
existing nmux protocol paths. It does not copy or depend on GPL or AGPL
implementation code.
