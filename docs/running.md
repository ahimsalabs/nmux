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
session=local tab=tab-1 pane=pane-1 size=80x24
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
session=local tab=tab-1 pane=pane-1 size=80x24
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

## Bounded Live Attach Prototype

The live attach prototype keeps one local connection open for a bounded number of input/output cycles. It is not raw terminal mode yet; it sends the same `--key` text on each cycle and prints any streamed pane surface update returned by the daemon.

Start a daemon that serves one live client for two input cycles:

```sh
rm -f /tmp/nmux.sock
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --live-cycles 2 --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
```

Attach a bounded live client:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --iterations 2 --key $'ping\n' --interval-ms 500
```

Expected output includes the initial surface and two streamed updates ending in `echo:ping`. The client uses `--interval-ms` as a read timeout for optional update frames. If no output is produced for a cycle, the client continues until the bounded iteration count is reached.

For script-driven live input, pipe lines through stdin:

```sh
printf 'ping\npong\n' | nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --stdin --interval-ms 500
```

With `--stdin`, each input line becomes one live input cycle. Use `--iterations` to cap the number of stdin lines consumed.

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
