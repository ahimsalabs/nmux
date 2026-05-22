# Terminal State Extraction Checklist

M13 replaces the interim text engine with daemon-owned backend `libghostty-vt`
state extraction. This file records the minimum mapping work needed before that
import so the protocol changes stay deliberate.

`libghostty-vt` is now present as an optional Cargo feature and compiles through
the vendored native Ghostty VT build. ADR 0018 keeps the default `nmuxd` engine
as `interim` while requiring the full feature-enabled `make check-ghostty-vt`
gate for related changes.

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
- Versions: bump surface versions when the nmux-visible surface, cursor,
  terminal title, or surface dimensions change; scrollback-only updates should
  not force a surface version bump. Keep workspace versions for tree metadata
  changes.
- Patch kind: cursor-only changes should use `PatchKind::CursorOnly`; terminal
  mode-only changes should use `PatchKind::ModeOnly`; row text or row-run-only
  changes should use `PatchKind::ReplaceRows` with only changed rows when pane
  geometry is stable; changes that cannot be expressed by the current patch
  schema, including style-table changes, should force a full snapshot. Clients
  must reject unsupported patch kinds rather than applying them as cursor-only
  updates.

## Known Schema Gaps

Do not freeze these into ad hoc string fields. Add protocol fields or objects
only after the backend extraction proves the exact shape needed.

- Cell style runs: `libghostty-vt` now supplies foreground/background colors,
  underline color, basic SGR flags, style identity, and cell widths for visible
  and scrollback rows. Surface snapshots and scrollback chunks carry a pane
  style table. The extractor preserves style-bearing trailing blank cells so
  background-colored terminal regions do not disappear from row runs, while
  default trailing blanks are still trimmed; richer style semantics still need
  protocol decisions.
- Grapheme and cell width: double-width cells, combining marks, and emoji ZWJ
  clusters are covered by `libghostty-vt` extraction tests and represented as
  per-cell run widths. Ambiguous-width policy and broader grapheme cases still
  need protocol guidance.
- Terminal modes: nmux snapshots and patches now carry bracketed paste, mouse
  tracking, detailed mouse tracking mode, mouse encoding format, focus
  reporting, application keypad, application cursor, origin, and wraparound
  state. `libghostty-vt` tracks those modes through its safe API,
  can derive key encoder behavior from terminal modes such as application
  cursor keys, and emits mode-only patches when only the mode payload changes.
  Its key encoder can also emit application-keypad sequences when the option is
  explicit, its focus helper can encode focus gained/lost events, and its paste
  validator rejects newline and bracketed paste terminator injection sequences.
  Local clients now forward UTF-8 paste input through `PasteInput`; the daemon
  rejects embedded bracketed-paste terminators, then wraps with bracketed-paste
  delimiters only when daemon-owned pane mode advertises bracketed paste.
  Local clients also forward focus gained/lost input through `FocusInput` only
  when the daemon-owned pane mode reports focus reporting enabled.
  Local clients forward common named keys for Enter, Tab, Backspace, Escape,
  Insert/Delete, Home/End, PageUp/PageDown, and F1-F12 through the live pane
  terminal engine. The interim engine preserves existing unmodified
  keypad/application-cursor behavior, and the libghostty-vt engine uses its key
  encoder from daemon-owned terminal state while preserving protocol modifiers.
  Public CLI modifier syntax maps `shift`, `ctrl`, `alt`, and `super` to the
  protocol modifier bits for named keys and mouse input. Local clients forward
  explicit mouse press/release/motion input only when the daemon-owned pane mode
  reports mouse tracking enabled, with bytes encoded by the live pane terminal
  engine from its current terminal mouse mode, format, and modifiers. Broader
  physical-key/text-event forwarding and frontend
  pointer integration still need protocol decisions.
- Terminal metadata: nmux carries terminal title and OSC 7 working-directory
  metadata through pane surface snapshot and patch metadata. Title/OSC 7-only
  changes use no-row surface patches so live clients can update pane metadata
  without reprinting unchanged row text. The libghostty-vt path has coverage for
  BEL-terminated, ST-terminated, split, cleared, and malformed OSC 7 input.
- Shell integration metadata: nmux carries OSC 133 row semantic prompt state on
  surface snapshots, surface patches, and scrollback chunks. `CellRun` also
  carries backend-observed OSC 133 semantic content for output, input, and
  prompt spans. Semantic command IDs, command ranges, lifecycle, and exit
  metadata remain withheld until that protocol shape is explicit.
- Alternate screen: `libghostty-vt` extraction tests cover entry into the
  alternate buffer, restoration of the primary buffer, and preservation of main
  scrollback while alternate-screen output is active. Alternate-screen
  scrollback remains intentionally withheld until nmux has protocol guidance for
  whether and how clients should request it.
- Palette and theme state: indexed SGR colors resolve into RGBA style-table
  entries during `libghostty-vt` extraction. `TerminalColorState` carries
  backend-observed default foreground/background colors, the active palette,
  palette overrides, and explicit cursor color state through surface snapshots,
  surface patches, scrollback chunks, and cached client state. Dynamic color
  changes currently require a full surface refresh; incremental palette diffs
  remain withheld until renderer requirements are clearer.
- Hyperlinks: OSC 8 link text is preserved by `libghostty-vt` extraction, and
  backend row/cell hyperlink presence is carried as bit 0 in `CellRun.flags`.
  nmux intentionally leaves `hyperlink_id` unset until URI, identifier, range
  ownership, and lifetime have a protocol object.
- Images and graphics protocols: nmux carries Kitty virtual placeholder
  presence on surface snapshots, surface patches, and scrollback chunks. Image
  placement, dimensions, persistence, pixel-data, and fallback protocol objects
  remain withheld.
- Damage granularity: row replacement is enough for the prototype, and nmux now
  carries backend row dirty flags on surface snapshots, sparse row-replacement
  patches, and scrollback chunks. Rows also carry a `row_state_hash` covering
  runs, semantic metadata, dirty state, and Kitty placeholder state, while the
  older `dirty_hash` remains a text-only compatibility fingerprint. Rich cells
  may still need run-level or region-level patches beyond cursor-only updates
  before nmux exposes a richer damage protocol.

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
render-state default colors/palette, palette overrides, explicit cursor color,
terminal color state, alternate-screen entry/restoration with alternate
scrollback omission, terminal title and OSC 7 working-directory metadata
extraction, OSC 133 row semantic prompt state, per-run semantic content,
resize/reflow, styled backend-owned scrollback extraction, row-level dirty
state, Kitty placeholder metadata, hyperlink presence, application-keypad and
application-cursor encoder support, modified named-key encoding, focus event
encoding, mouse event encoding, paste safety validation, safe-API mode tracking
for bracketed paste, mouse tracking, focus reporting, application keypad mode,
application cursor mode, origin, and wraparound.

The nmux state-sync path now has coverage for snapshot/patch cursor blink, title
and working-directory metadata, terminal color state, row semantic prompt
metadata, per-run semantic content, row dirty metadata, row state hashes, Kitty
placeholder row metadata, mode payloads, mode-only patch application, sparse row
replacement, metadata-only no-row live updates, cached client-state
compatibility, cached terminal metadata reattach, and feature-gated live CLI
smoke paths. `make check-ghostty-vt` runs the full `nmux-core` and `nmux-cli`
test suites with `--features libghostty-vt`, so ordinary feature-sensitive
tests are part of the opt-in gate. ADR 0018 keeps the default engine `interim`
until a later decision explicitly accepts the native Ghostty/Zig build cost in
normal development, CI, and packaging.
