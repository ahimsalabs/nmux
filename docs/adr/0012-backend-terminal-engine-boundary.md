# 0012: Backend Terminal Engine Boundary

Status: Accepted

Date: 2026-05-21

## Context

ADR 0001 makes `nmuxd` the owner of authoritative terminal state. ADR 0006 allowed a deliberately small interim PTY text surface so local process hosting, attach, reconnect, scrollback, and live workflows could be tested before integrating a real terminal engine.

M13 starts replacing that interim path with backend `libghostty-vt` extraction. A real VT engine is stateful per pane: PTY bytes are not independent chunks, and terminal state includes cursor, modes, alternate screen, styles, scrollback, grapheme width, hyperlinks, images, and other details that accumulate over time.

## Decision

Represent daemon-owned terminal interpretation behind a core terminal engine boundary.

The boundary accepts pane identity, size, current cursor, current nmux surface/scrollback state, and newly read PTY bytes. It returns updated nmux-owned cursor, surface, and scrollback objects. The current `InterimTextTerminalEngine` implements that boundary, while local daemon serving code keeps terminal engine instances alive per pane across output polls and sequential live clients.

Future backend `libghostty-vt` integration should implement this boundary inside `nmuxd`: PTY bytes enter the daemon-owned engine, and clients continue to receive nmux `PaneSurfaceSnapshot`, `PaneSurfacePatch`, and `ScrollbackChunk` objects.

## Consequences

The interim text surface remains replaceable without changing client attach, reconnect, live streaming, or scrollback fetch semantics.

The boundary keeps frontend rendering and backend terminal-state extraction separate. It does not authorize client-side raw PTY replay as a shortcut to Ghostty rendering.

The first `libghostty-vt` integration can focus on extraction mapping and schema gaps rather than daemon/client lifecycle plumbing.

The current boundary returns cursor state plus plain surface and scrollback lines. Backend `libghostty-vt` extraction may require explicit schema additions for mode fidelity, alternate screen, palette state, style runs, hyperlinks, images, grapheme details, and renderer metadata. Those additions should be documented before changing the public protocol.

## Licensing

Do not copy GPL or AGPL terminal parser code into this boundary. GPL/AGPL projects remain prior art or isolated adapter targets only.
