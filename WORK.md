# Work Plan

Current priority order. Build features that make nmux usable as a terminal
multiplexer before investing in promotion evidence, packaging, or documentation
infrastructure.

## Definition of usable

nmux is "usable" when a developer can: start a workspace, split panes, switch
tabs, run commands in each pane, detach, reattach from another terminal, and
see all panes restored.

## Next features (priority order)

- Complete ADR 0043 session actor runtime and ADR 0044 replay foundation
- Supply-chain pin-change review gate (issue #7)

## Test gaps to close

None queued.

## Deferred until usable

- Renderer equivalence oracle
- Protocol extensions (hyperlink IDs, images, command lifecycle, physical keys)
- Per-client cursor/mouse overlays; current multiplayer MVP uses actor identity,
  attach focused pane, and the shared daemon-owned terminal cursor (issue #2)
- Frontend Ghostty renderer hydration

## Architectural constraints

- Backend owns terminal state; no client-side PTY replay
- FlatBuffers state-sync protocol, not RPC
- No GPL/AGPL code in the core
- libghostty-vt is the default engine; interim stays as an explicit fallback
- ADR required before protocol schema changes

## Background

See [docs/roadmap.md](docs/roadmap.md) for milestone history (M0-M14 done).
See [docs/protocol-futures.md](docs/protocol-futures.md) for withheld protocol
tracks. See [docs/upstream](docs/upstream) for upstream-blocked work.
