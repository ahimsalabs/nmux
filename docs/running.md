# Running nmux

The current prototype is an M2 local attach skeleton. `nmuxd` owns one static workspace tree and one static pane surface, then sends a `WorkspaceTreeSnapshot` followed by a `PaneSurfaceSnapshot` to each local client that connects over a Unix socket. After rendering those snapshots, `nmux` sends one basic `InputEvent` back to the daemon.

Run all checks:

```sh
nix develop path:$PWD -c make check
```

Start a one-shot daemon:

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
```

For a daemon that keeps serving snapshots, omit `--one-shot`.

This is not a terminal emulator yet. It proves the first local daemon/client path: server-owned workspace state, server-owned pane surface state, FlatBuffers envelope framing, client-side rendering from decoded state objects, and client-to-daemon input events.
