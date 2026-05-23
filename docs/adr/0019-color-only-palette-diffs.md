# ADR 0019: Color-Only Palette Diffs

Status: Accepted
Date: 2026-05-22

## Context

ADR 0016 added `TerminalColorState` so backend-observed default colors, cursor
color, and palette state travel with terminal surfaces. M13 then introduced
`PatchKind::ColorOnly` for terminal color changes that do not repaint rows.

Sending the full active palette on every color-only patch works, but it is a
poor long-term protocol habit. Palette OSC updates often affect one index, and
clients already have a versioned cached palette when a patch is valid.

At the same time, nmux must not imply that palette-indexed style-table entries
can be remapped without repainting. Current `Style` entries carry resolved RGBA
values, so palette changes that alter existing styled rows still need a full
surface refresh unless the protocol later models style references differently.

## Decision

`TerminalColorState` keeps the full `palette_rgba` vector for snapshots and
scrollback chunks. `PaneSurfacePatch` may instead encode a color-only palette
change as:

- scalar terminal colors in `TerminalColorState`;
- no full `palette_rgba` vector;
- `palette_diff_start`, the first palette index replaced; and
- `palette_diff_rgba`, replacement palette entries from that index onward.

The diff is valid only when applied to the patch base surface version. Clients
materialize the new palette by truncating their cached palette at
`palette_diff_start` and appending `palette_diff_rgba`.
Clients reject palette diffs on snapshots, scrollback chunks, and non-color
patches; those objects must carry self-contained full palettes or avoid terminal
color changes entirely.

Color-only patches with no palette change may omit both the full palette and the
diff vector. Clients preserve their cached palette while applying scalar color
fields.

Palette changes coupled to row text, row runs, style-table changes, or
surface-kind transitions still require `PatchKind::FullRefreshRequired`.

## Consequences

Color-only palette updates become smaller without weakening backend-owned state
sync. Snapshots and scrollback remain self-contained.

Clients must treat a palette diff as patch-state data, not as an independent
absolute palette. A stale or missing cached surface cannot apply it and must
fall back to a snapshot.

This does not add hyperlink tables, image placement data, command lifecycle
metadata, or richer run/region damage. Those remain separate protocol
decisions.
