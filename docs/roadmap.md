# Implementation Roadmap

This roadmap promotes the build targets from [WORK.md](../WORK.md) into a tracked implementation sequence. Keep it current as commits land.

## Current Target

M6 is the current target: isolate process hosting behind a local/container/sandbox host abstraction.

Done:

- Initial architecture notes in [WORK.md](../WORK.md).
- ADR 0001 for backend-owned terminal state.
- ADR 0002 for Rust as the core runtime.
- Initial M0 schema in [schema/nmux.fbs](../schema/nmux.fbs).
- M0 protocol notes in [docs/protocol.md](protocol.md).
- Reproducible schema validation through `nix develop path:$PWD -c make check-schema`.
- Rust workspace and generated protocol crate for core implementation.
- Bounded FlatBuffers envelope framing.
- Initial Rust session model that encodes a `WorkspaceTreeSnapshot`.
- Local `nmuxd` and `nmux` skeletons that exchange the initial workspace snapshot over a Unix socket.
- Static server-owned `PaneSurfaceSnapshot` and dumb client text rendering.
- Basic key `InputEvent` sent from `nmux` back to `nmuxd`.
- Local-only attach prelude with known pane surface versions.
- Current-version reconnect test and stale-version full snapshot test.
- Patchable reconnect response using `PaneSurfacePatch`.
- ADR 0003 for scrollback as a separate synchronized object.
- `ScrollbackFetch` and `ScrollbackChunk` schema bodies.
- Static scrollback object with range-fetch tests and local client rendering.
- ADR 0004 for presence and attach modes.
- `PresenceUpdate`, `AttachMode`, and `PresenceKind` schema objects.
- Local attach prelude actor mode, presence update, and read-only input policy tests.
- Sequential two-client attach test against one local session.
- ADR 0005 for the process host boundary.
- Internal `nmux-core` process host abstraction with local/container/sandbox host choices and lifecycle tests.
- Concrete local command host that starts, writes to, and stops local child processes behind the host interface.
- Local PTY host behind the same process host boundary, using `portable-pty` for start, write, resize, and stop lifecycle coverage.

Next:

- Promote attach/request metadata into the public schema once the local reconnect behavior settles.
- Feed local PTY output into backend-owned pane state instead of the current static surface.

## Milestones

### M0: Protocol Contract

Goal: define the smallest state-sync schema that can carry workspace tree snapshots, pane surface snapshots, pane surface patches, input events, resize intents, and errors.

Exit evidence:

- `schema/nmux.fbs` exists.
- Schema validation runs from a documented command.
- `docs/protocol.md` explains M0 scope and exclusions.

### M1: Local nmuxd Skeleton

Goal: start a local daemon that can own a single session, tab, and pane, accept a local client connection, and exchange M0 envelopes.

Exit evidence:

- A local server command starts.
- A local client command attaches.
- The client receives a `WorkspaceTreeSnapshot`.
- Tests cover envelope encode/decode and initial session creation.

Status: Done for the static local skeleton. See [docs/running.md](running.md).

### M2: Dumb Viewer

Goal: render a pane surface from server-owned state and send basic keyboard input back to the daemon.

Exit evidence:

- A simple TUI or web viewer can attach locally.
- The viewer renders a surface snapshot.
- Input events reach the daemon with actor and pane IDs.

Status: Done for the static local skeleton. The local client renders a server-owned `PaneSurfaceSnapshot` and sends a basic key `InputEvent` back to the daemon.

### M3: Reconnect

Goal: allow a client to reconnect with known object versions and receive either patches or a fresh snapshot.

Exit evidence:

- A test demonstrates reconnect from a current version.
- A test demonstrates stale reconnect falling back to a full snapshot.

Status: Done for the local skeleton. The local Unix-socket prelude proves current-version, patchable-version, and stale-version behavior for pane surfaces. Public schema promotion remains a later protocol hardening step.

### M4: Scrollback Object

Goal: introduce scrollback as a separate synchronized object with lazy range fetches.

Exit evidence:

- Scrollback chunks are versioned separately from the visible surface.
- Tests cover consistency between visible rows and scrollback ranges.

Status: Done for the static local skeleton. The daemon serves a range-addressed `ScrollbackChunk`, and tests cover visible-surface consistency with the scrollback tail.

### M5: Multi-Player

Goal: support multiple actors with presence and read-only vs read-write attach modes.

Exit evidence:

- Two clients can attach to one session.
- Presence updates identify actors and capabilities.
- Read-only clients cannot send pane input.

Status: Done for the local skeleton. Two clients can attach sequentially to one session, presence identifies actor capabilities, and read-only actors do not send pane input.

### M6: Sandbox Host

Goal: isolate process hosting behind a local/container/sandbox host abstraction.

Exit evidence:

- Pane process lifecycle is mediated by a host interface.
- Local and sandbox host choices are represented without changing the protocol core.

Status: Done for the internal boundary and first local PTY host. `nmux-core` now models host specs, local/container/sandbox host kinds, a `ProcessHost` lifecycle interface, lifecycle tests, a concrete local command host, and a local PTY host without changing the FlatBuffers protocol. Feeding PTY output into pane state remains a follow-up.

### M7: Ghostty Frontend

Goal: prove a native Ghostty/libghostty-facing frontend path.

Exit evidence:

- The project has a documented answer for external surface rendering.
- A prototype frontend can render server-owned state or the needed upstream/API work is documented.

### M8: tmux Adapter

Goal: attach nmux to tmux as an adapter without making tmux the core model.

Exit evidence:

- Adapter process boundary is documented.
- Basic session/tree mapping is tested.

### M9: herdr Adapter

Goal: keep any herdr integration outside the core repository and behind the nmux protocol.

Exit evidence:

- Integration boundary is documented.
- No AGPL code is copied into the nmux core.
