# Implementation Roadmap

## Current target

See [WORK.md](../WORK.md) for the active priority list. The next major features
are multi-pane, multi-tab, simultaneous multi-client, scriptable CLI, and
remote transport — in that order.

## Milestones

### M0: Protocol Contract — done

FlatBuffers schema (`schema/nmux.fbs`) with envelope, workspace tree, pane
surface, scrollback, input, resize, presence, attach, and error bodies.

### M1: Local nmuxd Skeleton — done

Local daemon serves initial workspace snapshot over Unix socket.

### M2: Dumb Viewer — done

CLI text renderer attaches, renders server-owned surface, sends key input.

### M3: Reconnect — done

Client reconnects with known versions, receives patch or fresh snapshot.

### M4: Scrollback Object — done

Scrollback as a separate versioned object with lazy range fetches.

### M5: Multi-Player — done

Sequential two-client attach with presence, actor IDs, read-only enforcement.

### M6: Process Host Boundary — done

`ProcessHost` trait with local PTY host via `portable-pty`.
Container/sandbox variants exist in type system but aren't functional.

### M7: Ghostty Frontend Boundary — done

ADR 0007 documents the boundary. Backend libghostty-vt is the correctness
path; frontend hydration is a separate upstream question tracked in
[upstream/ghostty-surface-hydration.md](upstream/ghostty-surface-hydration.md).

### M8: tmux Adapter — done

Pure mapping structs translate tmux inventory to nmux session model. No actual
tmux process integration.

### M9: herdr Adapter — done

ADR 0010 documents the AGPL membrane boundary. No code in this repo.

### M10: Live Interactive Attach — done

Long-lived interactive PTY session over Unix socket. Live streaming, stdin
(line and byte mode), redraw, resize intents, detach key, read-only observers.

### M11: Terminal Frontend Polish — done

Alternate screen for redraw, flush after updates, detach/EOF reporting,
interim renderer limitations called out, initial scrollback in live mode.

### M12: Live Workspace Usability — done

Default socket path, managed `--start`/`--shell`, persisted state reattach,
bounded/unbounded sequential clients, JSON output, `NMUX_*` context, connect
timeout, `--ready-json`, input/resize validation, `make local-smoke`.

### M13: Backend libghostty-vt Extraction — done (opt-in)

Opt-in `--features libghostty-vt` engine. Extracts cursor, modes, styles,
colors, scrollback, cell widths, graphemes, semantic metadata, hyperlink
presence, mouse/focus/paste/key input with daemon-owned gating. Default
engine remains `interim`.

### M14: Post-M13 Product Split — done

ADR 0023 keeps libghostty-vt opt-in. Promotion requires build/CI/packaging
evidence tracked in [default-engine-promotion.md](default-engine-promotion.md).
Renderer equivalence, frontend hydration, and protocol extensions are separate
tracks.

## Not yet started

- Multi-pane (splitting, focus routing, multi-pane rendering)
- Multi-tab (tab creation, switching, closing)
- Simultaneous multi-client (concurrent broadcast, not sequential)
- Scriptable CLI workspace management
- Remote network transport (QUIC/TCP/SSH bootstrap)
- Container/sandbox hosts (functional, not just type stubs)
- Proxy daemon (aggregate multiple upstream nmuxd instances)
- Web/mobile frontends
- Permissions system beyond read-only/read-write
