# Running nmux

The current prototype is a local attach skeleton with a real local PTY host behind the daemon. `nmuxd` owns one workspace tree, one backend-owned pane surface, one scrollback object, and one attached actor. The client sends an `AttachRequest` with actor ID, attach mode, focused pane, and known pane surface versions. The daemon starts the pane command in a local PTY, polls already-pumped PTY output into backend-owned pane state, then sends a `WorkspaceTreeSnapshot`, a `PresenceUpdate`, and, when needed, either a `PaneSurfaceSnapshot` or a `PaneSurfacePatch`. `nmux` applies those state objects to a client-side pane surface render state before printing. After rendering, it sends one basic `InputEvent`, requests a scrollback range with `ScrollbackFetch`, and renders the returned `ScrollbackChunk`.

Run all checks:

```sh
nix develop path:$PWD -c make check
```

Start a one-shot daemon with the default local shell:

```sh
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --one-shot
```

In another shell, attach a client:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock
```

Expected output:

```text
session=local tab=tab-1 pane=pane-1 size=80x24 resize=fixed
nmux pane-1
server-owned terminal state
scrollback 1..3:
nmux pane-1
server-owned terminal state
```

For a deterministic PTY-output smoke test, run the daemon with an explicit shell command:

```sh
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --one-shot --command "printf 'hello from pty\n'; cat >/dev/null"
```

Expected output after attaching the client:

```text
session=local tab=tab-1 pane=pane-1 size=80x24 resize=fixed
booting nmux workspace
nmux pane-1
server-owned terminal state
hello from pty
scrollback 1..4:
nmux pane-1
server-owned terminal state
```

To prove input-driven output across attaches, keep the daemon running with a command that echoes each submitted line:

```sh
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done"
```

Then send input from one client:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --key $'ping\n' --scrollback-start 3 --scrollback-count 3
```

Attach a second passive client and fetch the same range plus the echoed line:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --no-input --scrollback-start 3 --scrollback-count 4
```

Expected second attach output includes:

```text
ready
ping
echo:ping
```

To keep one local frontend process polling for server-owned surface updates, run a bounded follow loop:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --follow --iterations 3 --interval-ms 500 --state /tmp/nmux-follow.state --scrollback-start 1 --scrollback-count 1
```

`--follow` is a local reconnect loop over the current request/response protocol. It keeps one in-process client render state, sends known pane surface versions on each reconnect, applies snapshots or patches when the daemon has newer state, and treats current-version reconnects as no render update. Follow mode is read-only for now, so it does not repeatedly send default input.

For a daemon that keeps serving snapshots, omit `--one-shot`.

## Live Attach Prototype

The live attach prototype keeps one local connection open for repeated input/output cycles. It is not a terminal UI yet; it sends the same `--key` text on each bounded client cycle, line-streams stdin, or forwards stdin byte chunks, and renders streamed pane surface updates through the same client-side pane surface state used by reconnects. Explicit input modes such as `--key`, `--stdin`, `--stdin-bytes`, and `--no-input` are mutually exclusive. Live-only frontend flags such as `--stdin`, `--stdin-bytes`, `--redraw`, and `--cols`/`--rows` are rejected unless `--live` is set, so ignored-mode mistakes fail before the client tries to connect.

By default, `nmuxd` and `nmux` use the same local socket path: `$XDG_RUNTIME_DIR/nmux/nmuxd.sock` when `XDG_RUNTIME_DIR` is set, otherwise `/tmp/nmux-$UID/nmuxd.sock`. Pass `--socket` on both sides when you want an isolated smoke-test socket.
If the daemon is not running or the client points at the wrong socket, `nmux` reports the socket path in the connection error.
If a socket path already exists, `nmuxd` refuses to replace it and reports the path. Remove a stale socket only after confirming no daemon is using it, or pass a different `--socket`.

Start a daemon that serves one live client for two input cycles:

```sh
nix develop path:$PWD -c cargo run --bin nmuxd -- --live-cycles 2 --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
```

Use `--live` instead of `--live-cycles` to keep serving that one live client until the client detaches:

```sh
rm -f /tmp/nmux.sock
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --live --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
```

Use `--live-forever` to keep the same daemon-owned workspace and PTY alive for sequential live clients until the daemon is stopped:

```sh
nix develop path:$PWD -c cargo run --bin nmuxd -- --live-forever --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
```

Use `--live-clients COUNT` to keep the same daemon-owned workspace and PTY alive for a bounded number of sequential live clients:

```sh
rm -f /tmp/nmux.sock
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --live-clients 2 --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --state /tmp/nmux-live.state --live --iterations 1 --key $'first\n'
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --state /tmp/nmux-live.state --live --no-input --iterations 1 --scrollback-start 1 --scrollback-count 8
```

The second live client attaches to the same backend-owned pane state and can observe output produced by the first live client. With `--state`, it sends its known pane surface version and renders the cached current surface when the daemon has no newer surface update to send.

Attach a bounded live client:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --live --iterations 2 --key $'ping\n' --interval-ms 500
```

Expected output includes the initial surface and two streamed updates ending in `echo:ping`. The client uses `--interval-ms` as a read timeout for optional update frames. If no output is produced for a cycle, the client continues until the bounded iteration count is reached. Bounded live and follow loops reject `--iterations 0` before connecting, and `--interval-ms` must be greater than zero.

Live mode renders the requested initial scrollback range after the first attached surface, using `--scrollback-start` and `--scrollback-count`. In `--redraw` mode, that initial scrollback context is included in the first repaint buffer before the current pane surface. Live mode can also use `--state` to persist the client-side pane surface cache. On attach, the client sends known pane surface versions from that file; streamed snapshots and patches update the same cache, and a current-version attach renders the cached surface without PTY byte replay. If the state file is corrupt or cannot be written, `nmux` reports the state path in the error.

By default, live mode prints each rendered update as plain text. Add `--redraw` to clear the terminal and repaint the latest workspace summary plus the current client-side pane surface on each update. When stdout is a TTY, `--redraw` uses the alternate screen and hides the cursor for the live session, then restores both on exit. Captured or piped stdout stays as plain clear/home escape output:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --redraw --stdin-bytes --interval-ms 500
```

Read-write live clients also poll process output during idle cycles. That means a process can update the backend-owned pane surface and stream patches to an attached read-write client even when the client has not sent a key frame in that cycle.

To send a resize intent before each live input cycle, pass both `--cols` and `--rows`:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --iterations 2 --key $'ping\n' --cols 100 --rows 30 --interval-ms 500
```

After the process-host resize succeeds, the daemon commits the pane size into the workspace tree and republishes a `WorkspaceTreeSnapshot`. The CLI prints the updated workspace summary, including the committed size and daemon-published resize policy.

The local daemon publishes `resize=fixed` by default. Use `nmuxd --resize-policy fixed|leader|active-client|manual` to advertise a different pane resize policy. `manual` ignores frontend viewport resize intents; explicit user-command resize intents remain eligible. Explicit `--cols` and `--rows` values must both be in the local PTY range, 1 through 65535. If a live client supplies `--cols` and `--rows` while the daemon publishes `manual`, the client reports `nmux: resize request ignored by manual resize policy` on stderr.

For line-streamed live input, pipe lines through stdin:

```sh
printf 'ping\npong\n' | nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --stdin --interval-ms 500
```

With `--stdin`, each input line is read and sent during its live input cycle, after the client has attached. If `--iterations` is omitted, stdin EOF ends the client loop without falling back to the default `--key` input and the client reports `nmux: stdin EOF; detached` on stderr. If `--iterations` is present, the client runs at most that many stdin cycles. The daemon treats client EOF during live read-write polling as a clean detach.

For byte-streamed live input, use `--stdin-bytes`:

```sh
printf 'ping\npong\n' | nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --stdin-bytes --interval-ms 500
```

`--stdin-bytes` reads stdin on a background thread and sends available chunks during the live polling loop as `InputKind.RawBytes`. This keeps output polling active even while no complete input line is available. When stdin is an interactive TTY, the client temporarily disables canonical input and local echo for this mode; piped stdin is left untouched. Pass `--local-echo tty` to preserve the TTY's existing echo setting while still using noncanonical byte input. Interactive byte mode also listens for `SIGWINCH` and sends resize intents from the current TTY size unless explicit `--cols` and `--rows` were supplied. When both stdin and stdout are TTYs, byte mode warns once on stderr that the interim text surface lacks full VT fidelity; piped and scripted runs stay quiet. Press Ctrl-] to detach from a byte-streamed live session; the client reports that local detach on stderr. When unbounded byte-streamed stdin reaches EOF, the client reports `nmux: stdin EOF; detached`.

For read-only live observation, use `--no-input`:

```sh
rm -f /tmp/nmux.sock
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --live-cycles 3 --command "printf 'ready\n'; sleep 0.05; printf 'tick-one\n'; sleep 0.05; printf 'tick-two\n'; sleep 1"
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --no-input --interval-ms 500
```

When an unbounded live client exits because the daemon closes the live socket, the client reports `nmux: live server closed connection` on stderr.

The read-only client attaches once, sends no input, and prints streamed surface updates when the daemon observes process output. If `--iterations` is omitted, it keeps polling until the daemon closes the live connection.

This is not a terminal emulator yet. The interim text surface only converts simple output bytes into backend-owned visible rows and scrollback. It proves the first local daemon/client path: server-owned workspace state, server-owned pane surface state derived from a local PTY, server-owned scrollback ranges, FlatBuffers envelope framing, client-side rendering from decoded state objects, and client-to-daemon input forwarding.

## Presence And Attach Modes

The FlatBuffers `AttachRequest` carries actor ID, user metadata, focused pane, and attach mode. The daemon replies with `PresenceUpdate`.

Current behavior:

- read-write actors may send pane input
- read-only actors may receive workspace, presence, surface, and scrollback state
- read-write local input is forwarded to the process host
- output is polled again after forwarded input so echoed text can update backend-owned scrollback
- read-only actors do not send pane input in the local client flow

The local skeleton currently accepts clients sequentially. Simultaneous multi-client attach is a later expansion.

## Reconnect Behavior

Reconnect metadata is carried by `AttachRequest.known_surfaces`.

Current behavior:

- no known surface version: daemon sends a full `PaneSurfaceSnapshot`
- known `pane-1` surface version is current: daemon sends no surface frame
- known `pane-1` surface version is patchable: daemon sends a `PaneSurfacePatch`
- known `pane-1` surface version is stale: daemon sends a full `PaneSurfaceSnapshot`

The CLI can persist its local render state with `--state`. This records the rendered pane surface and the last known server version, so a later process can request a patch and apply it to the cached surface instead of replaying raw PTY bytes.

Start a long-running command-backed daemon:

```sh
rm -f /tmp/nmux.sock /tmp/nmux-client.state
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done"
```

Attach once and persist the rendered surface:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --state /tmp/nmux-client.state --no-input --scrollback-start 1 --scrollback-count 1
```

Then attach again using the same state file after sending input from another client:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --key $'ping\n' --scrollback-start 1 --scrollback-count 1
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --state /tmp/nmux-client.state --no-input --scrollback-start 1 --scrollback-count 4
```

The final attach sends the cached `pane-1` surface version in `AttachRequest`. If the daemon has exactly one newer surface version, it sends `PaneSurfacePatch`; `nmux` applies that patch to the persisted client surface and updates `/tmp/nmux-client.state`.

This proves the reconnect decision and client-side patch rendering through the public FlatBuffers attach metadata.

## Scrollback Behavior

The prototype keeps scrollback separate from the visible pane surface. The client currently requests two lines starting at line `1`, and the daemon replies with a `ScrollbackChunk`. Tests assert that the visible surface matches the tail of the backend-owned scrollback object.
