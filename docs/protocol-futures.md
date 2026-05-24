# Future Protocol Tracks

Status: Tracking.

Last reviewed: 2026-05-23.

ADR 0023 keeps richer protocol objects separate from the post-M13
`libghostty-vt` extraction milestone. This file records the tracks that need
their own object model, compatibility plan, and ADR before fields are added to
[schema/nmux.fbs](../schema/nmux.fbs).

## Ground Rules

- Preserve backend-owned terminal state; do not ask clients to replay raw PTY
  bytes to recover missing objects.
- Keep schema changes append-friendly unless a new ADR explicitly accepts a
  compatibility break.
- Require decoder validation for new IDs, enum values, ranges, and cross-table
  references before clients can cache the object.
- Prefer full-object snapshots before incremental patch formats. Add patch
  deltas only after the snapshot semantics and invalidation rules are clear.
- Keep GPL and AGPL code out of nmux core. Treat incompatible projects as prior
  art or process-bound adapter targets only.

## Track Status

Use this table to decide whether a schema change belongs in current work or
needs a separate ADR first:

| Track | Current protocol surface | Withheld until ADR |
| --- | --- | --- |
| Hyperlinks | Hyperlink table on full surface snapshots and scrollback chunks; run-level presence flag. | Stable URI identity, ID lifetime, patch-table diffs, and nonzero run references from the backend. |
| Images and graphics | Kitty virtual placeholder presence on rows. | Placement objects, dimensions, pixel payload transfer, persistence, cache limits, and render security policy. |
| Damage | Row replacement, cursor/mode/color-only patches, row dirty flags, and row state hashes. | Cell/run/region/object damage shapes and compatibility recovery rules beyond `FullRefreshRequired`. |
| Command lifecycle | OSC 133 row prompt metadata and per-run semantic content. | Command IDs, prompt/input/output ranges, exit status, duration, and lifecycle ownership. |
| Physical key and text events | Text keys, raw bytes, paste, named keys, focus, mouse, and resize intents. | Layout-independent physical keys, IME/composition ownership, repeat/dead-key semantics, and richer frontend text events. |

## Hyperlink Identity

Current state: `PaneSurfaceSnapshot` and `ScrollbackChunk` carry a `Hyperlink`
table, and `CellRun.flags` records backend hyperlink presence. `hyperlink_id`
remains zero until the backend can provide stable URI identity.

Before schema or behavior changes:

- Decide hyperlink identity lifetime: pane-local, surface-version-local,
  scrollback-version-local, or session-global.
- Define whether OSC 8 ID and parameter strings are normalized, preserved raw,
  or both.
- Define patch behavior for new or removed identities. Current
  `PaneSurfacePatch` objects intentionally do not carry hyperlink-table diffs.
- Keep the upstream/API blocker in
  [libghostty-vt hyperlink identity access](upstream/libghostty-vt-hyperlink-identity.md)
  current until backend identity access is available.

## Images And Graphics

Current state: nmux carries Kitty placeholder presence on rows, but not image
placements, dimensions, pixel data, persistence metadata, or fallback payloads.

Before schema or behavior changes:

- Separate placement metadata from binary image payload transfer.
- Define image lifetime across visible surface, alternate screen, and
  scrollback.
- Decide whether payloads are inline, content-addressed, streamed as separate
  objects, or delegated to a side channel.
- Define cache invalidation, memory limits, and security policy before clients
  render remote-supplied pixels.

## Damage Objects

Current state: row replacement, no-row cursor/mode/color patches, row dirty
flags, and row state hashes are enough for the current prototype.

Before schema or behavior changes:

- Identify the client work that row-level patches cannot support efficiently.
- Define whether damage is cell-range, run-range, region, layer, or object
  based.
- Keep `FullRefreshRequired` as the recovery marker when an older client cannot
  apply a richer patch.
- Prove clients reject damage that references missing rows, runs, style IDs,
  hyperlink IDs, or future object tables.

## Command Lifecycle Metadata

Current state: nmux carries OSC 133 row semantic prompt metadata and per-run
semantic content. It does not carry command IDs, prompt/input/output ranges,
exit status, duration, or shell lifecycle events.

Before schema or behavior changes:

- Define command identity lifetime and whether it is pane-local or
  session-wide.
- Separate observed terminal metadata from shell-integration claims that may be
  spoofed by the process.
- Define range ownership across surface rows, scrollback rows, alternate
  screen, and history truncation.
- Decide how command metadata should survive reconnect and persisted client
  state without making clients infer it from text.

## Physical Key And Text Events

Current state: clients send text keys, raw bytes, paste, named keys, focus, and
mouse input. The daemon uses backend-owned terminal modes to encode what can be
encoded safely.

Before schema or behavior changes:

- Define a physical-key event object that can carry layout-independent keys,
  text input, modifiers, repeat, and compose/dead-key behavior without losing
  IME semantics.
- Preserve the existing `InputEvent.input_seq` attribution and protocol
  `Error.input_seq` reporting for failed input.
- Decide which frontends are responsible for text composition versus physical
  key identity.
- Keep raw bytes as an explicit escape hatch, not the default path for new
  structured input.

## Acceptance Rule

A future protocol track may add schema fields or envelope bodies only after an
ADR records:

- the object lifetime and owner;
- snapshot shape and any patch shape;
- compatibility and decoder validation rules;
- interaction with attach, reconnect, scrollback, cached state, and live
  streaming;
- test coverage required in `just check` and, when `libghostty-vt` state is
  involved, `just check-ghostty-vt`.
