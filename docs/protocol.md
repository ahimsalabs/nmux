# nmux Protocol

The nmux protocol synchronizes backend-owned terminal state. It is not a raw PTY byte stream and it is not an RPC surface for terminal commands.

The current schema lives at [schema/nmux.fbs](../schema/nmux.fbs).

## M0 Scope

M0 defines the smallest useful state-sync envelope:

- `Envelope` for versioning, sequencing, acknowledgements, and body dispatch.
- `WorkspaceTreeSnapshot` for sessions, tabs, panes, split layout, pane sizes, and resize policy.
- `PaneSurfaceSnapshot` for a full visible or alternate screen surface.
- `PaneSurfacePatch` for row, cursor, or terminal-mode updates against a known
  surface version.
- `ScrollbackFetch` and `ScrollbackChunk` for pane-scoped history ranges.
- `InputEvent` for key, raw byte, mouse, and paste input from an actor to a pane.
- `ResizeIntent` for client-originated size requests.
- `AttachRequest` for actor identity, attach mode, focused pane, and known pane surface versions at attach time.
- `Error` for protocol-level failures.

Permissions, sandbox hosts, adapters, and image-specific payloads are intentionally outside M0. They should be added as new envelope bodies when their object model is clear.

## State Ownership

`nmuxd` owns terminal state for each pane. Clients attach with the versions they know, receive snapshots or patches, render that state, and send input or control intents back to the daemon.

This keeps frontend behavior consistent across native, web, mobile, and automation clients. It also makes reconnect a protocol feature: a client can resume from its last known object versions instead of replaying terminal output.

## Surface Model

Pane surfaces and scrollback chunks are encoded as rows of runs:

- `SurfaceRow` and `RowUpdate` identify rows by index and include both a
  stable `dirty_hash`, the backend row `dirty` flag, and Kitty virtual
  placeholder presence.
- `ScrollbackRow` identifies history rows by absolute scrollback line and
  includes both a stable `dirty_hash`, the backend row `dirty` flag, and Kitty
  virtual placeholder presence.
- `CellRun` stores UTF-8 text, per-cell widths, a style table reference, flags, and an optional hyperlink reference.
- `Style` is a compact table referenced by run IDs. Full `PaneSurfaceSnapshot` objects and `ScrollbackChunk` objects carry the style table needed by their rows.
- `CursorState` stores cursor row, column, visibility, shape, and blinking.
- `TerminalMetadataState` stores pane terminal title metadata. OSC 7 working
  directory state is not modeled yet.
- `RowSemanticPrompt` stores OSC 133 prompt-line metadata on surface rows,
  row updates, and scrollback rows.
- `TerminalModeState` stores terminal modes that clients need for input and
  rendering decisions: bracketed paste, mouse tracking, focus reporting,
  application keypad, application cursor, origin, and wraparound.

This avoids freezing a simplistic per-cell ABI before the project has enough implementation feedback about graphemes, combining marks, double-width characters, terminal modes, hyperlinks, and image protocols.

Clients should maintain a pane-surface render state keyed by pane ID and version. A snapshot initializes the local surface buffer and style table, and a patch is applied only when its `base_version` matches the client's current version. Row patches are applied by encoded row index, not by vector order. Cursor-only patches update cursor state without row updates.

Cursor-only and mode-only patches can also update `TerminalMetadataState`
without row updates. Mode-only patches update `TerminalModeState` without row
updates. These are versioned surface changes because future input encoding,
renderer behavior, and pane chrome can depend on terminal state even when
visible text does not change.

`PaneSurfacePatch` intentionally does not carry a style table or hyperlink table. If the daemon's style table changes, or if terminal state changes in a way the current patch schema cannot express, the daemon must use `PatchKind::FullRefreshRequired` and the client must request or wait for a full `PaneSurfaceSnapshot`. Clients must reject unsupported patch kinds instead of treating them as cursor-only updates. They must not recover by replaying raw PTY bytes.

Scrollback is a separate versioned object. Clients request ranges with `ScrollbackFetch`; the daemon replies with `ScrollbackChunk` rows and the corresponding style table for that chunk.

## Resize Model

Clients do not directly resize PTYs. They send `ResizeIntent` with desired columns, rows, actor, pane, and reason. The daemon applies policy and later publishes committed size through workspace or pane state.

The policy is part of `PaneNode` so a pane can be fixed-size, leader-controlled, active-client-controlled, or manual.

## Input Model

`InputEvent` is still a client-to-daemon intent, not authoritative terminal state. Text-oriented commands can use `InputKind.Key` with `KeyInput.text_utf8`; byte-oriented live clients should use `InputKind.RawBytes` with `RawInput.bytes` so control bytes and non-UTF-8 input do not get lossy string conversion before they reach the process host. Paste-oriented commands use `InputKind.Paste` with `PasteInput.text_utf8`; local clients wrap the paste in bracketed-paste delimiters only when the current pane mode reports bracketed paste enabled, and reject embedded bracketed-paste terminators before forwarding.

## Validation

Validate the schema with:

```sh
nix develop path:$PWD -c make check-schema
```

The `check-schema` target runs `flatc` against [schema/nmux.fbs](../schema/nmux.fbs). Schema edits should stay append-friendly unless an ADR explicitly changes the compatibility posture.

Generate Rust protocol bindings with:

```sh
nix develop path:$PWD -c make generate-schema
```

The Rust core consumes the generated bindings through the `nmux-proto` crate.
