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
- `PaneSurfaceSnapshot`: pane ID, surface version, size, cursor, and rendered row
  text.
- `PaneSurfacePatch`: base/version pair, replacement rows, and cursor.
- `ScrollbackChunk`: scrollback lines by range.

The current terminal engine boundary owns:

- pane identity;
- old and new pane size;
- cursor state;
- visible surface rows;
- scrollback rows;
- PTY output bytes;
- resize events.

## First Backend Mapping

The first `libghostty-vt` engine should keep the existing client contract and map
backend terminal state into the same nmux objects:

- Cursor: row, column, visibility, and shape must come from the VT engine, not
  from row-count heuristics or serializer defaults.
- Surface kind: active main versus alternate screen state must come from the
  terminal engine and be present on full surface snapshots.
- Visible rows: extract the active screen viewport as row text compatible with
  the current `PaneSurfaceSnapshot` and `PaneSurfacePatch` fields. Snapshot and
  patch serialization must stay pane-scoped.
- Scrollback: expose historical rows through the existing `ScrollbackChunk`
  range model and advance scrollback versions when backend-owned history
  changes. Fetch handling must stay pane-scoped.
- Resize: feed resize events into the VT engine and publish the resulting pane
  size, cursor, visible rows, and scrollback state.
- Versions: bump surface versions when the nmux-visible surface, cursor, or
  surface dimensions change; scrollback-only updates should not force a surface
  version bump. Keep workspace versions for tree metadata changes.
- Patch kind: cursor-only changes should use `PatchKind::CursorOnly`; row
  changes should use `PatchKind::ReplaceRows`; changes that cannot be expressed
  by the current patch schema, including mode-only updates before mode fields
  exist, should force a full snapshot. Clients must reject unsupported patch
  kinds rather than applying them as cursor-only updates.

## Known Schema Gaps

Do not freeze these into ad hoc string fields. Add protocol fields or objects
only after the backend extraction proves the exact shape needed.

- Cell style runs: color, bold, italic, underline variants, reverse video,
  faint, blink, strike, and style identity.
- Grapheme and cell width: combining marks, emoji clusters, double-width cells,
  zero-width continuations, and ambiguous-width policy.
- Terminal modes: origin mode, wrap mode, bracketed paste, application cursor
  keys, keypad mode, and cursor blink.
- Alternate screen: alternate scrollback behavior and transitions between
  primary and alternate buffers.
- Palette and theme state: indexed palette overrides, default foreground and
  background, and dynamic color changes.
- Hyperlinks: URI, identifier, range ownership, and lifetime.
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
- unsupported VT features fail by omission with documented limitations, not by
  corrupting the existing nmux state objects.

## Current libghostty-vt Feature Gaps

The opt-in engine currently proves dependency and lifecycle wiring. It is not
the default because the first extraction pass still needs:

- backend-owned scrollback extraction instead of mirroring only the visible
  render rows into `ScrollbackChunk`;
- focused tests for cursor-only VT updates, alternate-screen transitions, and
  resize after scrollback reflow;
- documentation for the local GitHub HTTPS-to-SSH rewrite trap during the
  vendored build, or a repo-local build strategy that avoids relying on user
  Git config.
