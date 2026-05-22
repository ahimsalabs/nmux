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
- `PaneSurfaceSnapshot`: pane ID, surface version, size, cursor, terminal modes,
  style table, and rendered row runs.
- `PaneSurfacePatch`: base/version pair, replacement row runs, cursor, and
  terminal modes.
- `ScrollbackChunk`: scrollback row runs by range.

The current terminal engine boundary owns:

- pane identity;
- old and new pane size;
- cursor state, including visibility, shape, and blinking;
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
- Patch kind: cursor-only changes should use `PatchKind::CursorOnly`; terminal
  mode-only changes should use `PatchKind::ModeOnly`; row text or row-run-only
  changes should use `PatchKind::ReplaceRows`; changes that cannot be expressed
  by the current patch schema, including style-table changes, should force a
  full snapshot. Clients must reject unsupported patch kinds rather than
  applying them as cursor-only updates.

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
- Terminal modes: nmux snapshots and patches now carry bracketed paste, mouse
  tracking, focus reporting, application keypad, application cursor, origin,
  and wraparound state. `libghostty-vt` tracks those modes through its safe API,
  can derive key encoder behavior from terminal modes such as application
  cursor keys, and emits mode-only patches when only the mode payload changes.
  Its key encoder can also emit application-keypad sequences when the option is
  explicit, its focus helper can encode focus gained/lost events, and its paste
  validator rejects newline and bracketed paste terminator injection sequences.
  Terminal-derived keypad input forwarding, paste forwarding, mouse input
  forwarding, and focus forwarding still need protocol decisions.
- Terminal metadata: `libghostty-vt` exposes OSC 2 title state through the safe
  API, but nmux has no title metadata field yet. OSC 7 working-directory state
  remains unproven in the current backend path and should stay withheld until
  the expected upstream behavior is clear.
- Shell integration metadata: `libghostty-vt` exposes OSC 133 prompt semantics
  on rows, but nmux has no semantic prompt/input/output protocol fields yet.
  Keep this backend-observable state withheld until the row metadata shape is
  explicit.
- Alternate screen: `libghostty-vt` extraction tests cover entry into the
  alternate buffer, restoration of the primary buffer, and preservation of main
  scrollback while alternate-screen output is active. Alternate-screen
  scrollback remains intentionally withheld until nmux has protocol guidance for
  whether and how clients should request it.
- Palette and theme state: indexed SGR colors resolve into RGBA style-table
  entries during `libghostty-vt` extraction, and render-state tests prove the
  safe API exposes default foreground/background colors, the active palette,
  palette overrides, and explicit cursor color changes. Default
  foreground/background ownership in nmux protocol objects and the
  client-visible shape of dynamic color changes still need protocol decisions.
- Hyperlinks: OSC 8 link text is preserved by `libghostty-vt` extraction, and
  the safe API exposes row/cell hyperlink presence. nmux intentionally leaves
  `hyperlink_id` unset until URI, identifier, range ownership, and lifetime have
  a protocol object.
- Images and graphics protocols: `libghostty-vt` build info and row metadata
  can prove Kitty graphics support and virtual placeholder rows, but nmux has no
  image placement, dimensions, persistence, pixel-data, or fallback protocol
  objects yet.
- Damage granularity: row replacement is enough for the prototype, and
  `libghostty-vt` render-state tests prove row-level dirty state is
  backend-observable. Rich cells may still need run-level or region-level
  patches beyond cursor-only updates before nmux exposes a damage protocol.

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
wide-cell widths, cursor-only updates, cursor visibility/shape/blink extraction,
render-state default colors/palette,
palette overrides, and explicit cursor color, alternate-screen entry/restoration
with alternate scrollback omission, title metadata with OSC 7 working-directory
omission, OSC 133 semantic prompt state, resize/reflow, styled backend-owned
scrollback extraction, row-level dirty state, Kitty graphics placeholder
detection, hyperlink presence, application-keypad encoder support, focus event
encoding, paste safety validation, safe-API mode tracking for bracketed paste,
mouse tracking, focus reporting, application keypad mode, and origin/wraparound
modes, plus nmux snapshot/patch cursor blink and mode payloads, mode-only patch
application, and cursor cache compatibility through unit, session, and live CLI
smoke coverage. It is not the default until
the project deliberately accepts the native Zig/Ghostty build cost in normal
development and CI.
