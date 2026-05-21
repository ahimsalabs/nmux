# 0007: Ghostty/libghostty Frontend Boundary

Status: Accepted

## Context

ADR 0001 makes `nmuxd` the owner of terminal state. ADR 0006 adds an interim PTY text surface so local process hosting can be exercised before the real terminal engine is integrated.

[WORK.md](../../WORK.md) names Ghostty/libghostty as the desired terminal substrate. There are two separate integration points:

- backend libghostty, where `nmuxd` feeds PTY bytes into a terminal engine and extracts nmux snapshots and patches;
- frontend libghostty, where a native client might use Ghostty rendering, input, theme, and widget pieces to present server-owned state.

Current public Ghostty material points at `libghostty-vt` as the available embeddable terminal-emulation layer for parsing terminal sequences and maintaining terminal state. The Ghostling example also demonstrates that consumers provide their own drawing/windowing code on top of libghostty-vt render state. Ghostty's public VT documentation is for applications running inside Ghostty, not for hydrating a Ghostty renderer with externally supplied terminal grid state from another server.

The M7 question is therefore not "can the frontend run Ghostty?" It is: can a frontend Ghostty/libghostty renderer be hydrated from nmux's authoritative server-owned `PaneSurfaceSnapshot` and `PaneSurfacePatch` objects without re-parsing PTY bytes?

## Decision

M7 treats backend libghostty/libghostty-vt integration as the primary correctness path and frontend libghostty rendering as an integration milestone, not as a prerequisite for nmux state sync.

The native frontend must render nmux's server-owned surface objects. It must not consume raw PTY bytes as its source of truth, even if a Ghostty-derived renderer is embedded. Raw PTY bytes belong at the process-host/backend terminal-engine boundary.

Until Ghostty/libghostty exposes a clear external-state hydration API for rendering a supplied grid/surface, nmux should use a temporary nmux renderer for the native frontend prototype. That renderer should target the current `PaneSurfaceSnapshot` and `PaneSurfacePatch` object model first, then evolve with richer terminal state once backend libghostty extraction replaces the interim text parser.

If the project needs full Ghostty-native rendering of externally supplied state, the preferred path is an upstream Ghostty/libghostty API proposal. A fork is allowed only as a short-lived proving ground for that API shape. Copied GPL/AGPL code remains out of scope for the MIT/Apache core.

## Consequences

This preserves backend-owned terminal state and avoids a split-brain model where each client reconstructs terminal state from raw output.

The current `CellRun` and `Style` schema are not frozen as the complete Ghostty model. They are sufficient for the interim local path, but future libghostty-backed extraction may require explicit protocol additions for cursor/mode fidelity, alternate screen, palette state, hyperlinks, images, grapheme details, and renderer metadata.

The M7 prototype can proceed without waiting for Ghostty renderer hydration support: it should prove that a native client can render server-owned nmux surface state. Separately, M7 should track the upstream/API work needed to replace that renderer with a Ghostty/libghostty-backed rendering path.

ADR 0006 remains history for the interim PTY text surface. This ADR supersedes any interpretation that frontend Ghostty should parse process output directly.
