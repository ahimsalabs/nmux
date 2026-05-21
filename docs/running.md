# Running nmux

The current prototype is an M4 local attach skeleton. `nmuxd` owns one static workspace tree, one static pane surface, and one static scrollback object. The client sends a local-only attach prelude with known pane surface versions, then the daemon sends a `WorkspaceTreeSnapshot` and, when needed, either a `PaneSurfaceSnapshot` or a `PaneSurfacePatch`. After rendering those state objects, `nmux` sends one basic `InputEvent`, requests a scrollback range with `ScrollbackFetch`, and renders the returned `ScrollbackChunk`.

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
scrollback 1..3:
nmux pane-1
server-owned terminal state
```

For a daemon that keeps serving snapshots, omit `--one-shot`.

This is not a terminal emulator yet. It proves the first local daemon/client path: server-owned workspace state, server-owned pane surface state, server-owned scrollback ranges, FlatBuffers envelope framing, client-side rendering from decoded state objects, and client-to-daemon input events.

## Reconnect Behavior

The reconnect request prelude is intentionally local-only and not part of [schema/nmux.fbs](../schema/nmux.fbs) yet.

Current behavior:

- no known surface version: daemon sends a full `PaneSurfaceSnapshot`
- known `pane-1` surface version is current: daemon sends no surface frame
- known `pane-1` surface version is patchable: daemon sends a `PaneSurfacePatch`
- known `pane-1` surface version is stale: daemon sends a full `PaneSurfaceSnapshot`

This proves the reconnect decision before promoting attach metadata into the public FlatBuffers schema.

## Scrollback Behavior

The static prototype keeps scrollback separate from the visible pane surface. The client currently requests lines `1..3`, and the daemon replies with a `ScrollbackChunk`. Tests assert that the visible surface matches the tail of the static scrollback object.
