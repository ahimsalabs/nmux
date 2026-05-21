# Running nmux

The current prototype is an M1 local attach skeleton. `nmuxd` owns one static workspace tree and sends a `WorkspaceTreeSnapshot` to each local client that connects over a Unix socket.

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
```

For a daemon that keeps serving snapshots, omit `--one-shot`.

This is not a terminal emulator yet. It proves the first local daemon/client path: server-owned workspace state, FlatBuffers envelope framing, and client-side decode of a workspace tree snapshot.
