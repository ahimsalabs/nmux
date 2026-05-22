# Terminal State Extraction Checklist

M13 replaces the interim text engine with daemon-owned backend `libghostty-vt`
state extraction. This file records the minimum mapping work needed before that
import so the protocol changes stay deliberate.

`libghostty-vt` is now present as an optional Cargo feature and compiles through
the vendored native Ghostty VT build. The default `nmuxd` engine remains
`interim` until the acceptance gate below is complete.

## Current nmux Surface

`nmux-core` currently publishes:

- `WorkspaceTreeSnapshot`: session, tab, root pane identity, size, resize policy,
  and current pane surface version.
- `PaneSurfaceSnapshot`: pane ID, surface version, size, cursor, style table,
  and rendered row runs.
- `PaneSurfacePatch`: base/version pair, replacement row runs, and cursor.
- `ScrollbackChunk`: scrollback row runs by range.

The current terminal engine boundary owns:

- pane identity;
- old and new pane size;
- cursor state;
- visible surface rows;
- scrollback rows;
- row runs, cell widths, and the pane style table;
- PTY output bytes;
- resize events.

## First Backend Mapping

The first `libghostty-vt` engine should keep the existing client contract and map
backend terminal state into the same nmux objects:

- Cursor: row, column, visibility, and shape must come from the VT engine, not
  from row-count heuristics or serializer defaults.
- Surface kind: active main versus alternate screen state must come from the
  terminal engine and be present on full surface snapshots.
- Visible rows: extract the active screen viewport as row runs compatible with
  the current `PaneSurfaceSnapshot` and `PaneSurfacePatch` fields. Snapshot and
  patch serialization must stay pane-scoped.
- Cell runs: preserve style identity and cell-width metadata from the VT engine
  while keeping rendered row text available as a client fallback.
- Scrollback: expose historical rows through the existing `ScrollbackChunk`
  range model and advance scrollback versions when backend-owned history
  changes. Fetch handling must stay pane-scoped.
- Resize: feed resize events into the VT engine and publish the resulting pane
  size, cursor, visible rows, and scrollback state.
- Versions: bump surface versions when the nmux-visible surface, cursor, or
  surface dimensions change; scrollback-only updates should not force a surface
  version bump. Keep workspace versions for tree metadata changes.
- Patch kind: cursor-only changes should use `PatchKind::CursorOnly`; row text
  or row-run-only changes should use `PatchKind::ReplaceRows`; changes that
  cannot be expressed by the current patch schema, including style-table changes
  and mode-only updates before mode fields exist, should force a full snapshot.
  Clients must reject unsupported patch kinds rather than applying them as
  cursor-only updates.

## Known Schema Gaps

Do not freeze these into ad hoc string fields. Add protocol fields or objects
only after the backend extraction proves the exact shape needed.

- Cell style runs: `libghostty-vt` now supplies foreground/background colors,
  underline color, basic SGR flags, style identity, and cell widths for visible
  and scrollback rows. Surface snapshots and scrollback chunks carry a pane
  style table; richer style semantics still need protocol decisions.
- Grapheme and cell width: double-width cells, combining marks, and emoji ZWJ
  clusters are covered by `libghostty-vt` extraction tests and represented as
  per-cell run widths. Ambiguous-width policy and broader grapheme cases still
  need protocol guidance.
- Terminal modes: `libghostty-vt` tracks bracketed paste, mouse tracking,
  application keypad, origin, and wraparound modes through its safe API and can
  derive key encoder behavior from terminal modes such as application cursor
  keys. Its render state also exposes cursor blink state, but nmux has no mode
  or cursor-metadata fields yet. Keypad input forwarding, mouse input
  forwarding, and the client-visible shape of mode updates still need protocol
  decisions.
- Alternate screen: `libghostty-vt` extraction tests cover entry into the
  alternate buffer and restoration of the primary buffer. Alternate scrollback
  behavior still needs protocol guidance.
- Palette and theme state: indexed SGR colors resolve into RGBA style-table
  entries during `libghostty-vt` extraction. Palette overrides, default
  foreground/background ownership, and dynamic color changes still need protocol
  decisions.
- Hyperlinks: OSC 8 link text is preserved by `libghostty-vt` extraction, but
  nmux intentionally leaves `hyperlink_id` unset until URI, identifier, range
  ownership, and lifetime have a protocol object.
- Images and graphics protocols: placement, dimensions, persistence, and
  fallback behavior for clients without image support.
- Damage granularity: row replacement is enough for the prototype, but rich
  cells may need run-level or region-level patches beyond cursor-only updates.

## Acceptance Gate

Before enabling `nmuxd --terminal-engine libghostty-vt`, the repository should
have tests proving:

- the engine is stateful per pane across multiple PTY output reads;
- cursor movement without printable text updates `PaneSurfacePatch.cursor` using
  `PatchKind::CursorOnly`;
- resize events are handled by the engine instance used for that pane;
- scrollback fetches return backend-owned history after viewport changes;
- alternate-screen behavior is either correctly modeled or explicitly withheld
  behind the interim engine flag;
- visible rows preserve style-separated `CellRun` objects and wide-cell widths;
- clients preserve decoded surface and scrollback row runs instead of collapsing
  them to text-only state;
- unsupported VT features fail by omission with documented limitations, not by
  corrupting the existing nmux state objects.

## Current libghostty-vt Default-Enable Gate

The opt-in engine now proves dependency wiring, VT byte ingestion, visible-row
extraction, style-separated cell runs, basic SGR style flags, underline color,
wide-cell widths, cursor-only updates, cursor visibility/shape extraction,
backend-observable cursor blink state, alternate-screen entry/restoration,
resize/reflow, styled backend-owned scrollback extraction, and safe-API mode
tracking for bracketed paste, mouse tracking, application keypad mode, and
origin/wraparound modes through unit, session, and live CLI smoke coverage. It
is not the default until the project deliberately accepts the native
Zig/Ghostty build cost in normal development and CI.
