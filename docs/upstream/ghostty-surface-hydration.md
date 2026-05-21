# Ghostty/libghostty External Surface Hydration Tracker

Status: Tracking

Last reviewed: 2026-05-21

## nmux Requirement

nmux frontends render backend-owned terminal state. The specific M7 question is whether a Ghostty/libghostty-facing frontend can hydrate a renderer from nmux `PaneSurfaceSnapshot` and `PaneSurfacePatch` objects without parsing raw PTY bytes as its source of truth.

ADR 0007 decides that backend libghostty/libghostty-vt integration is the primary correctness path. Frontend Ghostty/libghostty rendering is an integration milestone, and nmux should use a temporary renderer until upstream exposes a clear external surface hydration API.

## Current Upstream Evidence

- Ghostty's public documentation describes `libghostty` as a cross-platform C-ABI library that provides terminal emulation, font handling, and rendering, while noting that the API is not yet a stable standalone library: <https://ghostty.org/docs/about>
- The Ghostty repository describes `libghostty-vt` as currently available for C and Zig, with terminal parsing and state maintenance available today but API signatures still in flux: <https://github.com/ghostty-org/ghostty>
- Ghostling demonstrates the practical current shape: `libghostty-vt` manages VT parsing, terminal state, scrollback, and renderer state, but consumers provide their own drawing and windowing code: <https://github.com/ghostty-org/ghostling>
- Ghostling also says `libghostty` has no opinion about the renderer or GUI framework and exposes render state that can be layered under any renderer: <https://github.com/ghostty-org/ghostling>

As of this review, this evidence supports using libghostty-vt/backend extraction plus an nmux renderer. It does not prove that Ghostty/libghostty can hydrate its renderer directly from externally supplied nmux grid state.

## Required API Shape

For nmux to replace its temporary renderer with a Ghostty/libghostty-backed frontend renderer, upstream needs an API shape that can:

- construct a renderable surface from externally supplied rows, runs, styles, cursor, and modes;
- apply incremental row, cursor, and mode updates against a known base version;
- represent or preserve palette state, hyperlinks, images, alternate screen, grapheme clusters, cell widths, and renderer metadata;
- render without replaying raw PTY bytes on the client;
- report when the supplied state is too stale or incomplete and a full nmux snapshot is required.

## Mapping To Current nmux Surface Objects

The current nmux prototype can supply:

- `PaneSurfaceSnapshot`: pane ID, version, surface kind, size, cursor, styles, and rows;
- `PaneSurfacePatch`: pane ID, base version, version, patch kind, row updates, and cursor;
- `SurfaceRow` / `RowUpdate`: row index, cell runs, and dirty hash;
- `CellRun`: UTF-8 text, cell widths, style ID, flags, and hyperlink ID;
- `Style`: foreground, background, underline color, and flags.

This schema is not frozen as the final Ghostty-compatible terminal model. ADR 0007 already calls out likely future additions for cursor/mode fidelity, alternate screen, palette state, hyperlinks, images, grapheme details, and renderer metadata.

## Gaps And Open Questions

- Is there an upstream API to build render state from externally supplied terminal grid data, rather than from VT-parser-owned terminal state?
- If the render state API is the right layer, can nmux map rows/runs/styles/cursor into it without copying Ghostty internals?
- How should nmux encode alternate screen state, terminal modes, palettes, hyperlinks, and image protocols before backend libghostty extraction is available?
- Would an upstream proposal be accepted as a public API, or would nmux need a short-lived fork to prove the shape first?
- What versioning or capability negotiation should a frontend use to declare support for Ghostty-backed rendering vs the temporary nmux renderer?

## M7 Acceptance Signal

This tracker can be considered satisfied for M7 when one of these is true:

- upstream Ghostty/libghostty documents and exposes an external surface hydration API that can render nmux-owned state without PTY replay;
- nmux has a documented upstream issue, discussion, or proposal for that API shape;
- nmux has a temporary native/frontend renderer that consumes `PaneSurfaceSnapshot` and `PaneSurfacePatch`, with the upstream API gap documented here.

## Non-Goals And Licensing

Do not use Ghostty/libghostty as a client-side raw PTY parser for nmux state sync. That violates backend-owned terminal state.

Do not treat the current `CellRun` / `Style` schema as final terminal fidelity.

Do not copy GPL or AGPL code into nmux. GPL/AGPL projects can remain prior art or isolated adapter targets only.
