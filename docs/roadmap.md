# Implementation Roadmap

## Current target

See [WORK.md](../WORK.md) for the active priority list. The local multiplexer
MVP is implemented: panes, tabs, concurrent clients, scriptable commands, and
token-authenticated TCP transport all have runnable CLI coverage.

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
Container and sandbox host choices lower to explicit runtime process
boundaries.

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
timeout, `--ready-json`, input/resize validation, `just local-smoke`.

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

### M15: Usable Local Multiplexer MVP — done

Local daemon/client workflows support pane splitting, tab creation/switching,
concurrent attached clients, scriptable pane/tab commands, pane send/snapshot,
record/replay, token-authenticated TCP attach, and container/sandbox host
selection. ADR 0030 records the experimental recording-format boundary, and
ADR 0031 records the container/sandbox execution boundary.

## Not yet started

- Proxy daemon (aggregate multiple upstream nmux daemon instances)
- QUIC or SSH-bootstrap remote transport beyond the current token TCP listener
- Web/mobile frontends
- Permissions system beyond read-only/read-write
