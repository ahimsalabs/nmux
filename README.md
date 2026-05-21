# nmux

nmux is an experimental portable terminal workspace protocol. The project direction is:

- backend-owned terminal state, not client-side PTY replay;
- FlatBuffers state-sync messages for sessions, tabs, panes, surfaces, scrollback, presence, input, and resize intent;
- local, sandbox, and adapter process boundaries;
- Ghostty/libghostty as the intended terminal-state engine, with a temporary text surface in the current prototype;
- adapters such as tmux or herdr kept outside the core model.

The current implementation is a Rust workspace with:

- `nmuxd`: a local daemon that owns one session, starts a local PTY, and serves nmux protocol frames over a Unix socket;
- `nmux`: a local client that attaches, renders server-owned pane state, sends input, persists client render state, and can run a live attach loop;
- `nmux-proto`, `nmux-core`, and `nmux-cli` crates;
- ADRs under [docs/adr](docs/adr);
- runnable notes in [docs/running.md](docs/running.md);
- the implementation roadmap in [docs/roadmap.md](docs/roadmap.md).

## Check

```sh
nix develop path:$PWD -c make check
```

## Quick Smoke

One-shot attach:

```sh
rm -f /tmp/nmux.sock
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --one-shot --command "printf 'hello from pty\n'; cat >/dev/null"
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --no-input
```

Bounded live attach:

```sh
rm -f /tmp/nmux.sock
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --live-cycles 2 --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --iterations 2 --key $'ping\n' --interval-ms 500
```

Line-streamed live input:

```sh
rm -f /tmp/nmux.sock
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --live --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
printf 'ping\npong\n' | nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --stdin --interval-ms 500
```

Byte-streamed live input:

```sh
rm -f /tmp/nmux.sock
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --live --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
printf 'ping\npong\n' | nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --stdin-bytes --interval-ms 500
```

This is still a prototype. It is not yet raw terminal mode, and the temporary text surface is not a VT-correct terminal emulator.
