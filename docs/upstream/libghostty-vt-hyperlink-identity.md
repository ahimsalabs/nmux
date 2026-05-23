# libghostty-vt Hyperlink Identity Access

Status: Open upstream/API gap.

nmux can now carry table-backed hyperlink identities in full
`PaneSurfaceSnapshot` and `ScrollbackChunk` objects, but the current
`libghostty-vt` safe API only exposes hyperlink presence on rows/cells. It does
not expose the URI, OSC 8 id, raw parameter string, stable link object, or a
per-cell link reference that nmux can safely map into `CellRun.hyperlink_id`.

The required shape is a structured accessor, not formatted terminal output:

- render-state and scrollback/history cells expose a stable hyperlink reference
  or table index;
- the referenced object exposes target URI, optional OSC 8 id, and optional raw
  parameter string;
- references are valid for the same render/snapshot extraction pass, so nmux
  can build object-local tables for snapshots and scrollback chunks;
- missing identity remains distinguishable from known absence, so nmux can keep
  `CellRun.flags` hyperlink presence without inventing an ID.

Parsing formatted output or tracking OSC 8 in nmux from raw PTY bytes is not the
right replacement. That would duplicate terminal-state ownership across cursor
movement, wrapping, scrolling, erasure, overwrites, alternate screen, and
scrollback lifetime. The M13 boundary is daemon-owned terminal state extracted
from `libghostty-vt`, not a parallel client or nmux-side terminal emulator.

When this API exists, nmux can populate `Hyperlink` tables during row extraction,
assign nonzero `CellRun.hyperlink_id` values, and keep ADR 0022's conservative
patch rule: patches may only reference IDs already known by the client's cached
surface version unless the protocol grows patch-scoped table diffs.
