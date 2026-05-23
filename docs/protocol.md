# nmux Protocol

The nmux protocol synchronizes backend-owned terminal state. It is not a raw PTY byte stream and it is not an RPC surface for terminal commands.

The current schema lives at [schema/nmux.fbs](../schema/nmux.fbs).

## Current Envelope Scope

The current schema has grown beyond the original M0 skeleton. It defines these
state-sync envelope bodies:

- `Envelope` for versioning, sequencing, acknowledgements, and body dispatch.
- `WorkspaceTreeSnapshot` for sessions, tabs, panes, split layout, pane sizes, and resize policy.
- `PaneSurfaceSnapshot` for a full visible or alternate screen surface.
- `PaneSurfacePatch` for row, cursor, or terminal-mode updates against a known
  surface version.
- `ScrollbackFetch` and `ScrollbackChunk` for pane-scoped history ranges.
- `InputEvent` for key, raw byte, paste, focus, and mouse input from an actor
  to a pane.
- `ResizeIntent` for client-originated size requests.
- `PresenceUpdate` for actor join/leave-style presence events.
- `AttachRequest` for actor identity, attach mode, focused pane, and known pane surface versions at attach time.
- `AttachStatus` for the daemon-selected pane and whether a surface frame
  follows the attach response.
- `Error` for protocol-level failures.

Permissions, sandbox hosts, adapters, and image-specific payloads remain outside
the current envelope. They should be added as new envelope bodies when their
object model is clear.

## State Ownership

`nmuxd` owns terminal state for each pane. Clients attach with the versions they know, receive snapshots or patches, render that state, and send input or control intents back to the daemon.

This keeps frontend behavior consistent across native, web, mobile, and automation clients. It also makes reconnect a protocol feature: a client can resume from its last known object versions instead of replaying terminal output.

## Surface Model

Pane surfaces and scrollback chunks are encoded as rows of runs:

- `SurfaceRow` and `RowUpdate` identify rows by index and include both a
  stable text-only `dirty_hash`, a `row_state_hash` covering runs and row
  metadata, the backend row `dirty` flag, and Kitty virtual placeholder
  presence.
- `ScrollbackRow` identifies history rows by absolute scrollback line and
  includes both a stable text-only `dirty_hash`, a `row_state_hash` covering
  runs and row metadata, the backend row `dirty` flag, and Kitty virtual
  placeholder presence.
- `CellRun` stores UTF-8 text, per-cell widths, a style table reference,
  flags, optional hyperlink reference, and per-run semantic content. Bit 0 in
  `CellRun.flags` means backend hyperlink presence for the run. Nonzero
  `hyperlink_id` values resolve through the `Hyperlink` table carried by the
  same `PaneSurfaceSnapshot` or `ScrollbackChunk`; the current extractor still
  leaves IDs zero until URI identity is wired. `cell_widths` is one byte per
  rendered cell in the run, so wide characters carry a width of 2 at their
  rendered cell position.
- `Hyperlink` stores table-backed hyperlink identity for full snapshots and
  scrollback chunks: numeric ID, target URI, optional OSC 8 identifier, and
  optional raw parameter string. `PaneSurfacePatch` does not carry hyperlink
  table diffs yet, so patches must not introduce references to unknown IDs.
- `Style` is a compact table referenced by run IDs. Full `PaneSurfaceSnapshot`
  objects and `ScrollbackChunk` objects carry the style table needed by their
  rows. `fg_rgba`, `bg_rgba`, and `underline_rgba` use `0xRRGGBBAA` packing;
  zero means the backend did not publish an explicit color for that field.
  `Style.flags` currently reserves bits 0-7 for bold, italic, faint, blink,
  inverse, invisible, strikethrough, and overline, and bits 8-12 for single,
  double, curly, dotted, and dashed underline. Unknown bits must be preserved
  by clients that cache or forward style tables.
- `CursorState` stores cursor row, column, visibility, shape, and blinking.
- `TerminalMetadataState` stores pane terminal title and working-directory
  metadata. The libghostty-vt path tracks OSC 7 byte sequences and populates the
  working-directory value carried by snapshots, patches, and cached client
  state.
- `TerminalColorState` stores backend-observed default foreground/background,
  optional explicit cursor color, and the active palette. Snapshots and
  scrollback chunks carry full palettes. Color-only patches may carry a
  palette diff using `palette_diff_start` and `palette_diff_rgba` against the
  patch base version instead of a full `palette_rgba` vector. Row,
  style-table, or surface-kind changes that also affect colors still require a
  full surface refresh. All terminal color fields use the same `0xRRGGBBAA`
  packing as style colors; `cursor_rgba_set` distinguishes an unset cursor
  color from an explicit transparent/zero value.
- `RowSemanticPrompt` stores OSC 133 prompt-line metadata on surface rows,
  row updates, and scrollback rows. `CellSemanticContent` stores the
  backend-observed OSC 133 content class for each run: output, input, or
  prompt. Broader shell command IDs, ranges, lifecycle, and exit metadata remain
  withheld until that object model is explicit.
- `TerminalModeState` stores terminal modes that clients need for input and
  rendering decisions: bracketed paste, mouse tracking, focus reporting,
  application keypad, application cursor, origin, and wraparound. Mouse
  tracking keeps a compatibility boolean and also carries the backend-observed
  tracking mode (`None`, `X10`, `Normal`, `Button`, or `Any`) plus mouse
  encoding format (`X10`, `Utf8`, `Sgr`, `Urxvt`, or `SgrPixels`).

This avoids freezing a simplistic per-cell ABI before the project has enough implementation feedback about graphemes, combining marks, double-width characters, terminal modes, hyperlinks, and image protocols.

Clients should maintain a pane-surface render state keyed by pane ID and version. A snapshot initializes the local surface buffer and style table, and a patch is applied only when its `base_version` matches the client's current version. Clients reject unknown terminal-state enum values in snapshots and patches, including `SurfaceKind`, `PatchKind`, cursor shape, mouse tracking mode, and mouse encoding format, instead of treating them as compatible fallback states. Control-plane frames likewise reject unknown `PaneKind`, `SplitAxis`, `AttachMode`, `PresenceKind`, `AttachSurfaceState`, `ResizePolicy`, `ResizeReason`, and `ErrorCode` values instead of coercing them to defaults. Row patches are applied by encoded row index, not by vector order. Cursor-only patches update cursor state without row updates.

Attach responses send `WorkspaceTreeSnapshot`, `PresenceUpdate`, and then
`AttachStatus`. `AttachStatus.pane_id` is the daemon-selected attached pane.
`AttachStatus.surface_state` is `Current` when the client already has the
current surface and no surface frame follows; otherwise it is `Snapshot` or
`Patch` and the next frame is the corresponding pane surface object. Clients
should use the status pane ID, not a guessed default, for post-attach input,
resize, and scrollback requests. If the daemon cannot resolve its active tab or
active pane, it sends a `PaneNotFound` `Error` instead of publishing an
`AttachStatus` for a guessed pane.

Cursor-only, mode-only, and color-only patches can also update
`TerminalMetadataState` without row updates. Metadata-only updates are therefore
ordinary `PatchKind::CursorOnly` patches with no row payload; there is no
separate metadata-only patch kind. Mode-only patches update `TerminalModeState`
without row updates, and color-only patches update `TerminalColorState` without
row updates. If a color-only patch carries a palette diff, clients apply it to
their cached palette for the matching `base_version`; the diff is not an
absolute palette. These are versioned surface changes because future input
encoding, renderer behavior, and pane chrome can depend on terminal state even
when visible text does not change.

`PaneSurfacePatch` intentionally does not carry a style table or hyperlink table. If the daemon's style table changes, if a row update would reference a new hyperlink ID that is absent from the client's cached table, if color changes are coupled to row/style changes, or if terminal state changes in a way the current patch schema cannot express, the daemon must use `PatchKind::FullRefreshRequired`. During attach, `AttachStatus.surface_state = Snapshot` is the recovery signal and the daemon immediately follows it with a full `PaneSurfaceSnapshot`. Clients must reject unsupported patch kinds instead of treating them as cursor-only updates. They must not recover by replaying raw PTY bytes.

Scrollback is a separate versioned object. Clients request ranges with
`ScrollbackFetch`; the daemon replies with `ScrollbackChunk` rows and the
corresponding style and hyperlink tables for that chunk.
`ScrollbackFetch.start_line`, `ScrollbackChunk.start_line`, and
`ScrollbackRow.line` are 1-based public line numbers, so line 1 is the oldest
retained row in the chunk's pane history.
`ScrollbackFetch.known_scrollback_version = 0` means the client is not asserting
a cached scrollback version. Nonzero known versions must match the daemon's
current pane scrollback version; mismatches return `ErrorCode::StaleVersion`
instead of serving a range from a different history version.

## Resize Model

Clients do not directly resize PTYs. They send `ResizeIntent` with desired
columns, rows, actor, pane, and reason. `ResizeReason::UserCommand` represents
an explicit user request such as live `--cols`/`--rows`.
`ResizeReason::FrontendViewport` represents automatic frontend viewport changes
such as interactive `SIGWINCH` handling. The daemon applies actor permissions
and pane resize policy, then later publishes committed size through workspace
or pane state. Read-only actors receive `PermissionDenied` if they send a live
resize intent.

The policy is part of `PaneNode` so a pane can be fixed-size,
leader-controlled, active-client-controlled, or manual. Manual resize policy
rejects automatic frontend viewport changes while still allowing explicit
user-command resize intents.

## Input Model

`InputEvent` is still a client-to-daemon intent, not authoritative terminal state. Text-oriented commands can use `InputKind.Key` with `KeyInput.text_utf8`; named-key commands can use `InputKind.Key` with `KeyInput.key_name`, currently for keypad Enter/digits, arrow keys, Enter, Tab, Backspace, Escape, Insert, Delete, Home, End, PageUp/PageDown, and F1-F12. The local daemon asks the live pane terminal engine to encode named keys from daemon-owned terminal state; the interim engine preserves existing unmodified keypad/application-cursor behavior, while the `libghostty-vt` engine uses its key encoder and preserves `KeyInput.modifiers`. Public CLI named-key and mouse modifiers use the low four protocol bits: `shift=1`, `ctrl=2`, `alt=4`, and `super=8`; decoded client input rejects unsupported modifier bits outside that mask. Byte-oriented live clients should use `InputKind.RawBytes` with `RawInput.bytes` so control bytes and non-UTF-8 input do not get lossy string conversion before they reach the process host. Paste-oriented commands use `InputKind.Paste` with `PasteInput.text_utf8`; the local daemon rejects embedded bracketed-paste terminators, ignores the historical client-provided `PasteInput.bracketed` preference, and wraps the paste in bracketed-paste delimiters only when daemon-owned pane mode reports bracketed paste enabled. Focus commands use `InputKind.Focus` with `FocusInput.focused`; the local daemon forwards focus gained/lost bytes only when the daemon-owned pane mode reports focus reporting enabled and otherwise returns a protocol `Error`. Mouse commands use `InputKind.Mouse` with zero-based cell coordinates, optional pixel coordinates, button, modifiers, and action; clients reject unknown mouse button/action enum values instead of coercing them to defaults. Attach requests also reject unknown attach modes rather than treating them as read-only. The local daemon gates mouse input by daemon-owned pane size and mouse tracking mode (`None`, `X10`, `Normal`, `Button`, or `Any`) before asking the live pane terminal engine to encode bytes from the current terminal mode and mouse format. Pixel coordinates are used for `MouseFormat::SgrPixels` when present; otherwise the event is encoded at the referenced cell origin. Local clients maintain monotonic `Envelope.seq` values across post-attach client frames and monotonic `InputEvent.input_seq` values across input events on the same connection. ADR 0017 records the input-mode policy and the remaining physical-key/text-event boundary.

When a structured input event cannot be encoded or safely forwarded, or a live
control intent such as resize cannot be applied by the process host, the daemon
sends an `Error` frame instead of terminating the connection with an opaque
socket close. `Error.pane_id` identifies the pane-scoped request that failed
when the daemon can attribute the failure to a pane. `Error.input_seq` carries
the originating `InputEvent.input_seq` for input failures and remains zero for
non-input failures. The local CLI reports the frame with the server-provided
reason. Pane-scoped client intents for unknown panes, missing active-tab or
active-pane metadata during attach, and unauthorized control intents return
protocol `Error` frames instead of hanging, silently omitting a response, or
falling through to process host behavior.

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
