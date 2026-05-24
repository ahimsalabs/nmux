# nmux

A portable terminal workspace protocol. Backend-owned terminal state, synced
over FlatBuffers — not client-side PTY replay.

## What it does today

- `nmuxd` owns a session with a local PTY, serves state over a Unix socket
- `nmux` attaches, renders server-owned pane state, sends input/resize intents
- Live attach, reconnect, persisted client state, scrollback fetches
- Opt-in `libghostty-vt` engine for VT-correct terminal state extraction
- Default `interim` text surface (not VT-correct — a prototype)

## What it doesn't do yet

- Multi-pane / pane splitting
- Multi-tab / tab management
- Simultaneous multi-client attach (sequential only)
- Scriptable workspace management (`nmux pane split`, `nmux tab new`)
- Remote transport (local Unix socket only)
- Container/sandbox process hosts
- Frontend Ghostty renderer

See [docs/roadmap.md](docs/roadmap.md) for the full roadmap.

## Quick start

```sh
# build
nix develop . -c make check

# smoke test
nix develop . -c make local-smoke

# interactive shell (one command)
nix develop . -c cargo run --bin nmux -- --shell

# or manual: daemon in shell 1, client in shell 2
nix develop . -c cargo run --bin nmuxd -- --live-forever        # shell 1
nix develop . -c cargo run --bin nmux -- --live --stdin-bytes --redraw  # shell 2
# Ctrl-] detaches the client; Ctrl-C stops the daemon
```

Nix examples assume `nix-command` and `flakes` are enabled. If not:
```sh
nix --extra-experimental-features 'nix-command flakes' develop . -c make local-smoke
```

## Checks

| Work | Commands |
| --- | --- |
| Default engine | `nix develop . -c make check && nix develop . -c make local-smoke` |
| Opt-in libghostty-vt | above + `nix develop . -c make check-ghostty-vt` |

## Project structure

```
crates/nmux-proto   FlatBuffers wire helpers and generated bindings
crates/nmux-core    Session, process host, terminal engine, adapters
crates/nmux-cli     nmuxd daemon, nmux client, integration tests
schema/             nmux.fbs protocol schema
docs/               Roadmap, protocol, ADRs, running guide
```

## More info

- [docs/running.md](docs/running.md) — detailed usage, socket behavior, examples
- [docs/roadmap.md](docs/roadmap.md) — milestones and next steps
- [docs/protocol.md](docs/protocol.md) — FlatBuffers state-sync contract
- [docs/adr](docs/adr) — architecture decisions
- [AGENTS.md](AGENTS.md) — agent operating guide
