# 0006: Interim PTY Text Surface

Status: Proposed

Date: 2026-05-21

## Context

The local host boundary can now start commands and PTYs, but the pane surface sent to clients is still static. nmux needs to start feeding process output into backend-owned pane state before it can become usable.

[WORK.md](../../WORK.md) says libghostty should eventually be the canonical terminal-state engine. That remains the right destination for VT parsing, cursor behavior, grapheme width, style runs, images, alternate screen handling, and scrollback correctness. Blocking all local progress on that integration would leave the process-host boundary unexercised.

## Decision

Use a deliberately small interim text surface for local PTY output.

The core may convert UTF-8-ish PTY output bytes into plain visible lines and scrollback lines so the daemon can serve backend-owned state derived from a real process. This model is not a terminal emulator and must not claim VT correctness. It may normalize carriage returns and line feeds, drop escape bytes conservatively, and keep a bounded tail for visible rows.

The FlatBuffers protocol remains unchanged. Clients continue receiving `PaneSurfaceSnapshot`, `PaneSurfacePatch`, and `ScrollbackChunk`; only the server-owned source of the rows changes from static fixture text to process-derived text.

## Consequences

This creates an end-to-end path for local process output while keeping the libghostty integration point clear. The interim text surface should be easy to replace with a libghostty-backed surface object later.

The interim surface is a sequencing device, not a competing terminal engine. It should be retired by backend `libghostty-vt` extraction once the local state-sync path has enough live, reconnect, and scrollback behavior to validate the replacement.

Tests should cover deterministic byte-to-surface behavior without using blocking PTY reads. PTY lifecycle tests remain in the host layer; terminal-state tests belong in the session/surface layer.

The implementation must avoid copying terminal parser behavior or code from GPL or AGPL projects. Prior art can inform architecture only.

## Compatibility

No wire schema changes are required. Future libghostty-backed state should preserve the existing snapshot/patch object shape unless a new protocol requirement is explicitly documented.
