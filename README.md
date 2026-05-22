# nmux

nmux is an experimental portable terminal workspace protocol. The project direction is:

- backend-owned terminal state, not client-side PTY replay;
- FlatBuffers state-sync messages for sessions, tabs, panes, surfaces, scrollback, presence, input, and resize intent;
- local, sandbox, and adapter process boundaries;
- backend Ghostty/libghostty-vt as the opt-in terminal-state correctness path, with a temporary text surface as the default prototype engine;
- adapters such as tmux or herdr kept outside the core model.

The current implementation is a Rust workspace with:

- `nmuxd`: a local daemon that owns one session, starts a local PTY, and serves nmux protocol frames over a Unix socket;
- `nmux`: a local client that attaches, renders server-owned pane state, sends explicit input and resize/control intents, persists client render state, and can run a live attach loop;
- `nmux-proto`, `nmux-core`, and `nmux-cli` crates;
- ADRs under [docs/adr](docs/adr);
- runnable notes in [docs/running.md](docs/running.md);
- the implementation roadmap in [docs/roadmap.md](docs/roadmap.md), currently focused on backend `libghostty-vt` extraction.

## Check

```sh
nix develop path:$PWD -c make check
nix develop path:$PWD -c make check-ghostty-vt
```

## Quick Smoke

Inspect the current CLI flags:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --help
nix develop path:$PWD -c cargo run --bin nmuxd -- --help
```

`nmux --help` also calls out the current renderer limitation: the default prototype uses an interim text surface, not a VT-correct terminal emulator. That is a sequencing device while the local state-sync/live workflow stays fast. Backend `libghostty-vt` extraction is available behind an opt-in Cargo feature, separate from the later question of hydrating a frontend Ghostty renderer from nmux-owned state.

One-shot attach:

```sh
rm -f /tmp/nmux.sock
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --one-shot --command "printf 'hello from pty\n'; cat >/dev/null"
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock
```

Bounded live attach:

```sh
nix develop path:$PWD -c cargo run --bin nmuxd -- --live-cycles 2 --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
nix develop path:$PWD -c cargo run --bin nmux -- --live --iterations 2 --key $'ping\n' --scrollback-start 1 --scrollback-count 4 --interval-ms 500
```

Both binaries share a stable default socket path for the current user. They use `$XDG_RUNTIME_DIR/nmux/nmuxd.sock` only when `XDG_RUNTIME_DIR` is a valid absolute path, otherwise `/tmp/nmux-$UID/nmuxd.sock`. Pass `--socket` on both sides when you want an isolated smoke-test socket.
Connection failures include the socket path, which helps distinguish a missing daemon from an isolated test socket.
Use `nmux --connect-timeout-ms MS` when a script may start the client before `nmuxd` has finished binding the socket.
`nmuxd` refuses to replace an existing socket path, so remove stale sockets deliberately or choose a different `--socket`.
On normal bounded exits, `nmuxd` removes the socket path it created if that path still points at the same socket file.
Live attach renders the requested initial scrollback range before streaming updates, including when `--redraw` is enabled. When the backend reports terminal title or OSC 7 working-directory metadata, the local CLI prints those metadata lines with the current pane surface; metadata-only live updates print just the changed metadata lines in non-redraw mode.
Use `nmuxd --live-clients COUNT` to keep the same local workspace alive across a bounded number of sequential live clients. Pair it with `nmux --state PATH` to reattach from a persisted client-side surface cache, including cached terminal metadata, when the daemon has no newer surface update to send. One-shot, follow, and live attach paths all reuse that scoped cached surface instead of replaying PTY bytes. Follow mode is observation-only and rejects input flags instead of silently dropping them.
Use `nmuxd --live-forever` for an unbounded sequential local workspace that survives repeated live client detach and reattach until the daemon is stopped.
State load/save failures include the state path in the error.
Bounded client loops require `--iterations` greater than zero.
Without an explicit input or resize flag, `nmux` attaches read-only; use `--key`, `--key-name`, `--paste`, `--focus`, `--mouse`, `--stdin`, or `--stdin-bytes` to opt into sending input, or live `--cols`/`--rows` to send a resize control intent.
Explicit one-shot input is still sent when a persisted state file proves the visible surface is already current.
Scrollback ranges are 1-based from the oldest retained row and require positive `--scrollback-start` and `--scrollback-count` values. Local clients persist last-seen scrollback range metadata in `--state`, fetch scrollback even when the visible surface is already current, send matching cached versions as fetch preconditions, and retry once without a precondition if the daemon reports a stale scrollback version.
Explicit input modes such as `--key`, `--stdin`, `--stdin-bytes`, and `--no-input` are mutually exclusive.
Live polling intervals must be greater than zero, and explicit resize dimensions must be between 1 and 65535.

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

For interactive `--stdin-bytes`, Ctrl-] detaches the client.
Add `--redraw` to repaint the workspace summary and current pane surface in place on each live update; when stdout is a TTY, redraw uses the alternate screen and restores it on exit. Interactive byte mode uses noncanonical stdin, defaults local echo off, can preserve the TTY echo setting with `--local-echo tty`, and sends TTY-size resize intents on `SIGWINCH` unless explicit `--cols` and `--rows` are provided. Explicit `--cols` and `--rows` are user-command resize intents, so they can commit a resize without sending pane input even when the daemon publishes `resize=manual`.
Live-only frontend flags such as `--stdin`, `--stdin-bytes`, `--redraw`, and `--cols`/`--rows` are rejected unless `--live` is set.

Resize policy smoke:

```sh
rm -f /tmp/nmux.sock
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --live-cycles 1 --resize-policy active-client --command "printf 'ready\n'; sleep 1"
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --no-input --iterations 1
```

Expected output includes `resize=active-client`.

This is still a prototype. It has an interactive byte-streamed live path, but the default temporary text surface is not a VT-correct terminal emulator; ANSI styling, cursor motion, alternate screen, images, and grapheme/cell-width correctness are not complete in the default engine. Backend `libghostty-vt` extraction is available as an experimental opt-in engine for correctness work. The opt-in engine now covers VT byte ingestion, cursor updates including blink state, alternate-screen detection/restoration with alternate scrollback omission, resize/reflow, backend-owned styled scrollback, style-separated row runs including style-bearing trailing blanks, row semantic prompt metadata, per-run semantic content, cell widths, combining marks, emoji ZWJ clusters, basic SGR style flags, underline color, palette-indexed SGR colors, terminal color state, render-state default colors/palette, palette overrides, explicit cursor color, color-only surface patches, terminal title metadata, OSC 7 working-directory metadata extraction, metadata-only no-row patches, OSC 133 row semantic prompt state, row-level dirty state, row state hashes, sparse row updates, Kitty graphics placeholder metadata, hyperlink presence on row runs, bracketed paste, paste safety validation, PasteInput forwarding with daemon-owned delimiter selection, detailed mouse tracking mode/format state and pane-bounds/mode-gated MouseInput forwarding with modifiers, focus reporting and daemon-gated FocusInput forwarding with protocol-visible disabled-mode errors, common named-key forwarding, application keypad tracking and mode-aware keypad Enter/digit forwarding, application cursor tracking and mode-aware arrow-key forwarding, public named-key modifier syntax, engine-backed key encoding with modifier preservation, explicit encoder output, Error frames for structured-input encoding failures, host input/resize failures, unknown pane-scoped intents, and stale scrollback version preconditions, origin, and wraparound modes, mode-aware key encoding, and mode-only surface patches. Feature-gated live CLI tests cover command streaming, committed user-command resize metadata, reattach behavior, metadata-only updates, style-table full-refresh recovery, and restored alternate-screen scrollback omission through `nmuxd --terminal-engine libghostty-vt`. Image placement data, hyperlink IDs, incremental palette diffs, and broader shell command metadata still wait for matching nmux protocol objects.
The core now routes pane output through a daemon-owned terminal engine boundary so that interim behavior can be replaced without changing client-side state-sync semantics.
`nmuxd --terminal-engine interim` makes the default engine explicit. `nmuxd --terminal-engine libghostty-vt` is available only in `--features libghostty-vt` builds; it is covered by full opt-in feature test suites, but not yet the default because it pulls in the native Ghostty/Zig build path.
