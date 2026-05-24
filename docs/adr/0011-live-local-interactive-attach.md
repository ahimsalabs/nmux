# 0011: Live Local Interactive Attach

Status: Accepted

Date: 2026-05-21

## Context

The local prototype can start a PTY, attach a client, render server-owned state, send one input event, fetch scrollback, and exit. `--follow` can poll through repeated reconnects, but it is read-only and still treats the protocol as request/response.

That is enough to prove the first state-sync objects, but not enough to be a usable terminal workspace. Live adapters such as tmux or herdr would immediately run into the same gap: nmux needs a long-lived attach loop where the daemon keeps polling backend output, forwarding permitted client input, and sending updated pane state on the same connection.

The project should make the native local PTY path usable before expanding live adapter integrations. This keeps correctness pressure on the nmux-owned protocol and runtime instead of hiding missing behavior behind adapter-specific code.

## Decision

M10 should be live local interactive attach.

The local attach path should evolve from one request/response exchange into a long-lived session loop:

- the client sends one `AttachRequest` and stays connected;
- the daemon sends the initial workspace, presence, and surface state;
- permitted read-write clients can send repeated `InputEvent` messages;
- the daemon forwards input through the process host, polls backend output, updates backend-owned pane state, and sends `PaneSurfacePatch` or `PaneSurfaceSnapshot` updates;
- read-only clients can stay attached and observe updates without sending input
  or resize control intents;
- resize handling should use `ResizeIntent` and the existing process-host resize boundary when policy allows.

The first implementation should stay library-level. It should prove repeated input/output cycles over one connection before adding raw terminal mode, screen clearing, keyboard decoding, or richer CLI UX.

## Consequences

This milestone should come before live tmux or herdr adapter scaffolding. A live adapter is useful only after the nmux daemon/client loop can sustain an interactive workspace.

The current reconnect behavior remains valuable. A reconnecting client should still be able to use known pane surface versions, but reconnect polling is no longer the primary live interaction mechanism.

The interim text surface remains acceptable for this milestone. M10 tests should assert state-sync behavior and permission policy, not VT correctness.

## Exit Evidence

M10 is done when:

- `nmuxd` can keep serving one local PTY while at least one client stays attached;
- a long-lived read-write client can send repeated input events over one connection;
- process output updates backend-owned pane state and reaches the client as patches or snapshots;
- a read-only client can observe updates without forwarding input or resize
  control intents;
- tests cover repeated input/output, current-version no-update behavior, and
  read-only input/resize permission enforcement.

## Compatibility

No public schema change is required for the first live local attach loop. `AttachRequest`, `InputEvent`, `ResizeIntent`, `PaneSurfaceSnapshot`, and `PaneSurfacePatch` already exist.

If an explicit attach acknowledgement, heartbeat, detach message, or subscription object becomes necessary, add it append-only in a later ADR and implementation step.
