# nmux

A portable terminal workspace protocol. Backend-owned terminal state, synced
over FlatBuffers — not client-side PTY replay.

## What it does today

- `nmux daemon` owns a session with a local PTY, serves state over a socket
- `nmux` attaches, renders server-owned pane state, sends input/resize intents
- Split panes, switch tabs, route input to the focused pane, and send commands
- Live attach, reconnect, persisted client state, split redraw, scrollback fetches
- Concurrent live clients with shared surface updates and presence identity
- Scriptable control: `nmux pane split`, `nmux pane send`, `nmux tab new`, `nmux kill`
- Token-authenticated TCP transport for non-Unix-socket experiments
- Experimental container/sandbox host selection for daemon-started pane commands
- Default `libghostty-vt` engine for VT-correct terminal state extraction
- Explicit `interim` text surface for legacy/debug use

## What it doesn't do yet

- Hardened container/sandbox isolation policy
- Frontend Ghostty renderer
- Hardened remote transport and production auth

See [docs/roadmap.md](docs/roadmap.md) for the full roadmap.

## Install

```sh
# run directly from GitHub (requires Nix with flakes)
nix run github:ahimsalabs/nmux
```

Pre-built binaries for Linux (x86_64, aarch64) and macOS (Apple Silicon) are
available from [nightly releases](https://github.com/ahimsalabs/nmux/releases/tag/nightly).

## Quick start

```sh
# build
nix develop . -c just check

# smoke test
nix develop . -c just local-smoke

# interactive shell (one command)
nix develop . -c cargo run --bin nmux

# or manual: daemon in shell 1, client in shell 2
nix develop . -c cargo run --bin nmux -- daemon --live-forever  # shell 1
nix develop . -c cargo run --bin nmux -- --live --stdin-bytes --redraw  # shell 2
# Ctrl-] detaches the client; Ctrl-C stops the daemon
```

Nix examples assume `nix-command` and `flakes` are enabled. If not:
```sh
nix --extra-experimental-features 'nix-command flakes' develop . -c just local-smoke
```

## Checks

| Work | Commands |
| --- | --- |
| Default engine | `nix develop . -c just check && nix develop . -c just local-smoke` |
| Interim/no-default-features | `nix develop . -c just check-interim` |

## Project structure

```
crates/nmux-proto   FlatBuffers wire helpers and generated bindings
crates/nmux-core    Session, process host, terminal engine, adapters
crates/nmux-cli     nmux daemon/client CLI, integration tests
schema/             nmux.fbs protocol schema
docs/               Roadmap, protocol, ADRs, running guide
```

## Rust dependencies

Cargo dependencies are locked in [Cargo.lock](Cargo.lock). Refresh this summary
with `nix develop . -c cargo metadata --format-version 1 --all-features` after
dependency changes.

Direct workspace dependencies:

| Crate | Normal dependencies | Dev/test dependencies | Optional features |
| --- | --- | --- | --- |
| `nmux-proto` | `flatbuffers` | none | none |
| `nmux-core` | `flatbuffers`, `libc`, `nmux-proto`, `portable-pty` | `serde_json` | `libghostty-vt` |
| `nmux-cli` | `clap`, `flatbuffers`, `libc`, `nmux-core`, `nmux-proto`, `serde_json` | `portable-pty` | `libghostty-vt` |

Resolved dependency inventory, including indirect, optional, target-specific,
and test dependencies:

```text
anstream, anstyle, anstyle-parse, anstyle-query, anstyle-wincon, anyhow,
bitflags, cfg-if, cfg_aliases, clap, clap_builder, clap_derive, clap_lex,
colorchoice, downcast-rs, filedescriptor, flatbuffers, heck, int-enum,
is_terminal_polyfill, itoa, lazy_static, libc, libghostty-vt,
libghostty-vt-sys, log, memchr, nix, nmux-cli, nmux-core, nmux-proto,
once_cell_polyfill, portable-pty, proc-macro2, proc-macro2-diagnostics, quote,
rustc_version, semver, serde, serde_core, serde_derive, serde_json, serial2,
shared_library, shell-words, strsim, syn, thiserror, thiserror-impl,
unicode-ident, utf8parse, version_check, winapi, winapi-i686-pc-windows-gnu,
winapi-x86_64-pc-windows-gnu, windows-link, windows-sys, winreg, zmij
```

## More info

- [docs/running.md](docs/running.md) — detailed usage, socket behavior, examples
- [docs/roadmap.md](docs/roadmap.md) — milestones and next steps
- [docs/protocol.md](docs/protocol.md) — FlatBuffers state-sync contract
- [docs/adr](docs/adr) — architecture decisions
- [AGENTS.md](AGENTS.md) — agent operating guide
