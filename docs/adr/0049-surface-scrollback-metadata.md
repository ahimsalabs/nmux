---
status: accepted
date: 2026-05-27
---

# Surface Scrollback Metadata

## Context

Interactive clients draw scrollbars even when they are showing the live pane
surface rather than a fetched scrollback range. Before this change, the only
protocol object with scrollback length was `ScrollbackChunk`, so a default
redraw client that skipped the initial scrollback fetch could not size the
thumb until the user first scrolled.

## Decision

Add `scrollback_version` and `scrollback_total_lines` to
`PaneSurfaceSnapshot` and `PaneSurfacePatch`. The daemon already computes this
metadata from the pane transcript, and including it on surface updates lets
clients keep scrollbar proportions current without fetching history rows.

`ScrollbackChunk` remains the authority for row contents. Surface scrollback
metadata is only a lightweight index signal for UI state, cache invalidation,
and deciding which range to lazily fetch.

## Consequences

Live clients can render a proportional bottom-anchored scrollbar on initial
attach and keep it accurate as new output extends the transcript. Rapid
scrolling can coalesce local viewport changes and fetch only the newest desired
range while still using surface metadata to bound the scrollbar.
