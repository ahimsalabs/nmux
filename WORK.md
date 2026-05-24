# Work Plan

Current priority order. Build features that make nmux usable as a terminal
multiplexer before investing in promotion evidence, packaging, or documentation
infrastructure.

## Definition of usable

nmux is "usable" when a developer can: start a workspace, split panes, switch
tabs, run commands in each pane, detach, reattach from another terminal, and
see all panes restored.

## Next features (priority order)

None queued. Close the test gaps below before adding promotion, packaging, or
evidence infrastructure.

## Test gaps to close

None queued.

## Deferred until usable

- Default engine promotion evidence / packaging / CI bundles
- Renderer equivalence oracle
- Protocol extensions (hyperlink IDs, images, command lifecycle, physical keys)
- Per-client cursor/mouse overlays; current multiplayer MVP uses actor identity,
  attach focused pane, and the shared daemon-owned terminal cursor (issue #2)
- Frontend Ghostty renderer hydration
- Container/sandbox process hosts

## Architectural constraints

- Backend owns terminal state; no client-side PTY replay
- FlatBuffers state-sync protocol, not RPC
- No GPL/AGPL code in the core
- libghostty-vt stays opt-in until promotion criteria (ADR 0023) are met
- ADR required before protocol schema changes

## Background

See [docs/roadmap.md](docs/roadmap.md) for milestone history (M0-M14 done).
See [docs/protocol-futures.md](docs/protocol-futures.md) for withheld protocol
tracks. See [docs/upstream](docs/upstream) for upstream-blocked work.
