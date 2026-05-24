# 0009: tmux Adapter Process Boundary

Status: Accepted

Date: 2026-05-21

## Context

M8 needs nmux to attach to tmux without making tmux the core model. That means the integration has to respect the existing nmux architecture:

- `nmuxd` owns normalized workspace, pane, terminal-state, attach, reconnect, presence, and permission semantics;
- clients receive nmux FlatBuffers state objects and should not parse host- or adapter-specific messages;
- process hosts and adapters sit behind boundaries so local, container, sandbox, and future external backends do not leak into the public protocol.

tmux has its own sessions, windows, panes, layout semantics, history, control mode, and lifecycle behavior. If those become the nmux public model, nmux turns into a tmux-compatible frontend instead of a backend-owned terminal state protocol.

The repository also needs a clear licensing posture for adapter work. Prior art or adapters with GPL/AGPL licensing may inform architecture at a high level, but their code must not be copied into this repository.

## Decision

tmux integration will be an external adapter target, not an nmux core model.

The tmux adapter should run as a separate `nmux-tmux-adapter` process. `nmuxd` may talk to it through an nmux-owned internal adapter boundary, but the core session model must not import tmux data structures or expose tmux control-mode events as the nmux wire protocol.

The adapter owns tmux-specific work:

- launching or attaching to an installed tmux command/server;
- observing tmux sessions, windows, panes, focus, output, history, and lifecycle signals;
- formatting tmux commands and targeting tmux panes;
- handling tmux version differences, process failures, detach behavior, and recovery quirks.

`nmuxd` owns normalized nmux behavior:

- workspace/session identity exposed to clients;
- tab and pane identity;
- terminal surface versions;
- attach and reconnect behavior;
- presence and permission policy;
- client-facing `WorkspaceTreeSnapshot`, `PaneSurfaceSnapshot`, `PaneSurfacePatch`, `ScrollbackChunk`, input, resize, and lifecycle messages.

The first mapping should be narrow:

- tmux session -> nmux imported workspace or session source;
- tmux window -> nmux tab;
- tmux pane -> nmux pane;
- tmux active window and pane -> focus hints, not global truth for every attached client;
- tmux pane dimensions -> initial pane size and layout input;
- tmux pane title or current command metadata -> pane title/metadata where available;
- tmux pane output -> bytes/events consumed by the nmux terminal-state path;
- nmux input and resize intents -> adapter-targeted tmux requests when policy allows;
- tmux history -> optional scrollback source normalized into `ScrollbackChunk` and pane surface state.

Clients must continue to speak nmux FlatBuffers only. They must not parse tmux output, depend on tmux control mode, use tmux IDs as their only durable pane identity, or treat tmux layout semantics as canonical nmux layout semantics.

## Consequences

No public FlatBuffers schema change is required for the first M8 implementation. The next implementation step should be a pure mapping layer and test that converts adapter-owned tmux inventory structs into the existing nmux workspace/session shape.

A crashing or incompatible tmux adapter should surface as an adapter failure or pane lifecycle/error update. It must not corrupt core session state or force clients into tmux-specific recovery paths.

tmux IDs may be recorded as external adapter identifiers, but nmux must still provide stable normalized IDs and versioned state for client attach/reconnect.

Live tmux process handling, control-mode parsing, pane output streaming, resize forwarding, and history import should land behind the adapter boundary after the pure mapping path is tested.

## Licensing

Do not copy GPL or AGPL code into this repository.

Do not copy tmux implementation code as source material. A live adapter may interact with an installed tmux binary through documented command-line or protocol behavior, but nmux-owned code must remain independently implemented and isolated behind the adapter process boundary.
