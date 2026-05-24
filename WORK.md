# Work Plan

Current priority order. Build features that make nmux usable as a terminal
multiplexer before investing in promotion evidence, packaging, or documentation
infrastructure.

## Definition of usable

nmux is "usable" when a developer can: start a workspace, split panes, switch
tabs, run commands in each pane, detach, reattach from another terminal, and
see all panes restored.

## Next features (priority order)

1. **Simultaneous multi-client**: broadcast surface updates to all attached
   clients concurrently, not just sequential attach; decide and implement the
   multiplayer cursor/mouse presence model before duplicating frontend state
   tracking (issue #2)
2. **Scriptable CLI**: `nmux pane split`, `nmux tab new`, `nmux tab close`,
   `nmux pane send`, `nmux pane snapshot --json`
3. **Remote transport**: TCP/QUIC listener beyond Unix socket, identity/auth

## Test gaps to close

- Protocol robustness: corrupt FlatBuffer payloads, version mismatch, partial
  frames, mid-connection disconnect
- Multi-pane session unit tests (currently only single-pane)
- macOS CI job (primary dev platform, CI only runs ubuntu-latest)
- Wire `check-ghostty-vt` into CI (tests exist but aren't in check.yml)

## Deferred until usable

- Default engine promotion evidence / packaging / CI bundles
- GitHub Actions cache/build optimization and libghostty-mode CI coverage (issue #4)
- Renderer equivalence oracle
- Streaming playback/replay design for exports, web embeds, timestamp metadata,
  and presence history (issue #3)
- Protocol extensions (hyperlink IDs, images, command lifecycle, physical keys)
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
