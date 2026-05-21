# Implementation Roadmap

This roadmap promotes the build targets from [WORK.md](../WORK.md) into a tracked implementation sequence. Keep it current as commits land.

## Current Target

M10 is the current target: turn local attach from request/response smoke tests into a long-lived interactive PTY session.

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
- ADR 0006 for the interim PTY text surface.
- Core pane state can hydrate visible rows and scrollback from process output bytes.
- Local attach can serve process-derived pane state over the Unix socket.
- Nonblocking process output seam and local PTY output queue for polling already-pumped bytes.
- Local attach polls available process output before choosing snapshot, patch, or no surface response.
- `nmuxd` starts a local PTY host and can run a command-backed PTY smoke through the local client.
- Read-write local input is forwarded to the process host, while read-only attaches do not forward input.
- Local attach polls process output after forwarded input so echoed command output can update backend-owned scrollback.
- Client flags can drive an interaction smoke across attaches with custom input and scrollback ranges.
- ADR 0007 for the Ghostty/libghostty frontend boundary.
- Client-side pane surface render state can initialize from snapshots, apply row patches by row index, and reject patch base-version mismatches.
- The `nmux` CLI renders through client-side pane surface state and can persist that state with `--state` so a later attach sends known pane versions and can apply a server patch.
- The `nmux --follow` local loop keeps one client render state across repeated reconnects, applies snapshots/patches, and treats current-version reconnects as no render update.
- ADR 0008 for promoting the local attach prelude into a public FlatBuffers `AttachRequest`.
- `AttachRequest` and known pane surface versions are public schema objects, and the local attach handshake uses a FlatBuffers envelope instead of the text prelude.
- ADR 0009 for the tmux adapter process boundary, including licensing and ownership rules.
- Internal tmux adapter inventory structs can map a tmux session/window/active-pane view into the existing nmux session model without launching tmux or changing the public schema.
- ADR 0010 for the herdr integration boundary and AGPL membrane.
- ADR 0011 for making live local interactive attach the next milestone before live adapter work.

Next:

- Add a library-level long-lived attach loop test that keeps one connection open across repeated input/output updates.
- Keep [the Ghostty/libghostty surface hydration tracker](upstream/ghostty-surface-hydration.md) current as upstream APIs change.

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

Status: Done for the local skeleton. `AttachRequest` now carries known pane surface versions in the public FlatBuffers schema, and tests prove current-version, patchable-version, and stale-version behavior for pane surfaces.

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

Status: Done for the internal boundary and first local PTY host. `nmux-core` now models host specs, local/container/sandbox host kinds, a `ProcessHost` lifecycle interface, lifecycle tests, a concrete local command host, a local PTY host, an interim process-output text surface, and a nonblocking output polling seam without changing the FlatBuffers protocol. `nmuxd` starts the local PTY host, local attach can poll already-pumped output before responding, read-write input is forwarded to the hosted process, and output is polled again after forwarded input.

### M7: Ghostty Frontend

Goal: prove a native Ghostty/libghostty-facing frontend path.

Exit evidence:

- The project has a documented answer for external surface rendering.
- A prototype frontend can render server-owned state or the needed upstream/API work is documented.

Status: Done. ADR 0007 documents the boundary: backend libghostty/libghostty-vt is the primary correctness path, frontend libghostty rendering must not re-parse PTY bytes, and a temporary nmux renderer is the practical prototype path unless Ghostty/libghostty exposes external surface hydration. The local client library has a small pane-surface render state that applies server-owned snapshots and patches by version, the CLI uses the same state path for output rendering, persisted reconnect state, and a read-only follow loop, and [the upstream tracker](upstream/ghostty-surface-hydration.md) records the external hydration API gap.

### M8: tmux Adapter

Goal: attach nmux to tmux as an adapter without making tmux the core model.

Exit evidence:

- Adapter process boundary is documented.
- Basic session/tree mapping is tested.

Status: Done. ADR 0009 documents the tmux adapter process boundary: tmux remains an external adapter target, clients continue to speak nmux FlatBuffers, and `nmuxd` keeps ownership of normalized workspace, pane, terminal-state, attach, reconnect, presence, and permission semantics. The core has a pure tmux inventory-to-nmux session mapping with tests for active window/pane focus hints, stable adapter-derived IDs, and invalid empty inventories, without launching tmux or changing the public schema.

### M9: herdr Adapter

Goal: keep any herdr integration outside the core repository and behind the nmux protocol.

Exit evidence:

- Integration boundary is documented.
- No AGPL code is copied into the nmux core.

Status: Done. ADR 0010 documents the herdr integration boundary: any herdr work stays behind an `nmux-herdr-adapter` process and, if needed, a separate repository; clients continue to speak nmux FlatBuffers; and no GPL or AGPL implementation material is copied into the nmux core.

### M10: Live Local Interactive Attach

Goal: turn local attach from request/response smoke tests into a long-lived interactive PTY session.

Exit evidence:

- `nmuxd` can keep serving one local PTY while at least one client stays attached.
- A long-lived read-write client can send repeated input events over one connection.
- Process output updates backend-owned pane state and reaches the client as patches or snapshots.
- A read-only client can observe updates without forwarding input.
- Tests cover repeated input/output, current-version no-update behavior, and read-only permission enforcement.

Status: In progress. ADR 0011 documents why this comes before live tmux or herdr adapter work and scopes the first implementation to a library-level long-lived attach loop before raw terminal UX.
