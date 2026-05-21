# nmux Protocol

The nmux protocol synchronizes backend-owned terminal state. It is not a raw PTY byte stream and it is not an RPC surface for terminal commands.

The current schema lives at [schema/nmux.fbs](../schema/nmux.fbs).

## M0 Scope

M0 defines the smallest useful state-sync envelope:

- `Envelope` for versioning, sequencing, acknowledgements, and body dispatch.
- `WorkspaceTreeSnapshot` for sessions, tabs, panes, split layout, pane sizes, and resize policy.
- `PaneSurfaceSnapshot` for a full visible or alternate screen surface.
- `PaneSurfacePatch` for row, cursor, or mode updates against a known surface version.
- `InputEvent` for key, mouse, and paste input from an actor to a pane.
- `ResizeIntent` for client-originated size requests.
- `Error` for protocol-level failures.

Scrollback, presence, permissions, sandbox hosts, adapters, and image-specific payloads are intentionally outside M0. They should be added as new envelope bodies when their object model is clear.

## State Ownership

`nmuxd` owns terminal state for each pane. Clients attach with the versions they know, receive snapshots or patches, render that state, and send input or control intents back to the daemon.

This keeps frontend behavior consistent across native, web, mobile, and automation clients. It also makes reconnect a protocol feature: a client can resume from its last known object versions instead of replaying terminal output.

## Surface Model

Pane surfaces are encoded as rows of runs:

- `SurfaceRow` and `RowUpdate` identify rows by index and include a `dirty_hash`.
- `CellRun` stores UTF-8 text, per-cell widths, a style table reference, flags, and an optional hyperlink reference.
- `Style` is a compact table referenced by run IDs.

This avoids freezing a simplistic per-cell ABI before the project has enough implementation feedback about graphemes, combining marks, double-width characters, terminal modes, hyperlinks, and image protocols.

## Resize Model

Clients do not directly resize PTYs. They send `ResizeIntent` with desired columns, rows, actor, pane, and reason. The daemon applies policy and later publishes committed size through workspace or pane state.

The policy is part of `PaneNode` so a pane can be fixed-size, leader-controlled, active-client-controlled, or manual.

## Validation

The schema should be validated with `flatc` once FlatBuffers tooling is added to the repo. Until then, edits to [schema/nmux.fbs](../schema/nmux.fbs) should be reviewed as schema changes and kept append-friendly.
