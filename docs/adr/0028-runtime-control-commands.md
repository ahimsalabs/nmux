# 0028: Runtime Control Commands

Status: Accepted

Date: 2026-05-24

## Context

nmux needs scriptable user commands such as pane splitting and tab creation
after the daemon is already running. These are backend workspace mutations, not
terminal input bytes and not attach options.

## Decision

Add a small `ControlCommand` FlatBuffers envelope body for local client requests
that mutate workspace structure. The initial command kinds are pane split, tab
new, and tab close; the daemon replies with the existing
`WorkspaceTreeSnapshot` on success and existing `Error` frames on failure.

## Consequences

The protocol keeps terminal input and workspace control separate while reusing
the state-sync response object clients already understand. Future command kinds
can extend this table without turning attach into an RPC surface.

## Licensing Notes

This decision is nmux-specific protocol shape. It does not copy or depend on
GPL or AGPL implementation code.
