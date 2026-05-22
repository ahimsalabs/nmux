# Running nmux

The current prototype is a local attach skeleton with a real local PTY host behind the daemon. `nmuxd` owns one workspace tree, one backend-owned pane surface, one scrollback object, and one attached actor. The client sends an `AttachRequest` with actor ID, attach mode, focused pane, and known pane surface versions. The daemon starts the pane command in a local PTY, polls already-pumped PTY output into backend-owned pane state, then sends a `WorkspaceTreeSnapshot`, a `PresenceUpdate`, and, when needed, either a `PaneSurfaceSnapshot` or a `PaneSurfacePatch`. `nmux` applies those state objects to a client-side pane surface render state before printing. After rendering, it can send one explicit text, paste, named-key, focus, or mouse `InputEvent`, requests a scrollback range with `ScrollbackFetch`, and renders the returned `ScrollbackChunk`.

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

`--follow` is a local reconnect loop over the current request/response protocol. It keeps one in-process client render state, sends known pane surface versions on each reconnect, applies snapshots or patches when the daemon has newer state, and renders the scoped cached surface when a current-version reconnect has no newer surface frame. Follow mode is read-only for now, so it rejects input flags instead of repeatedly sending input.

For a daemon that keeps serving snapshots, omit `--one-shot`.

## Live Attach Prototype

The live attach prototype keeps one local connection open for repeated input/output cycles. It is not a terminal UI yet; without an explicit input or resize flag it observes read-only, while `--key` sends the same text on each bounded client cycle, `--stdin` line-streams stdin, and `--stdin-bytes` forwards raw stdin byte chunks. It renders streamed pane surface updates through the same client-side pane surface state used by reconnects. Explicit input modes such as `--key`, `--key-name`, `--paste`, `--focus`, `--mouse`, `--stdin`, `--stdin-bytes`, and `--no-input` are mutually exclusive. Live-only frontend flags such as `--stdin`, `--stdin-bytes`, `--redraw`, and `--cols`/`--rows` are rejected unless `--live` is set, so ignored-mode mistakes fail before the client tries to connect.

By default, `nmuxd` and `nmux` use the same local socket path: `$XDG_RUNTIME_DIR/nmux/nmuxd.sock` when `XDG_RUNTIME_DIR` is a valid absolute path, otherwise `/tmp/nmux-$UID/nmuxd.sock`. Pass `--socket` on both sides when you want an isolated smoke-test socket.
If the daemon is not running or the client points at the wrong socket, `nmux` reports the socket path in the connection error.
When a script starts `nmux` before `nmuxd` has finished binding, pass `--connect-timeout-ms MS` so the client waits for the socket instead of failing immediately.
If `nmuxd` cannot bind the socket path, it reports that path. If a socket path already exists, `nmuxd` refuses to replace it and includes a recovery hint; remove a stale socket only after confirming no daemon is using it, or pass a different `--socket`.
On normal bounded exits, `nmuxd` removes the socket path it created if that path still points at the same socket file.
Scrollback ranges are 1-based from the oldest retained row, and the client rejects zero `--scrollback-start` or `--scrollback-count` values before connecting.

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

Live mode renders the requested initial scrollback range after the first attached surface, using `--scrollback-start` and `--scrollback-count`. In `--redraw` mode, that initial scrollback context is included in the first repaint buffer before the current pane surface. Live mode can also use `--state` to persist the client-side pane surface cache. On attach, the client sends known pane surface versions from that file; streamed snapshots and patches update the same cache, and a current-version attach renders the cached surface without PTY byte replay. If the state file is corrupt or cannot be written, `nmux` reports the state path in the error. In non-redraw mode, metadata-only updates print changed title or working-directory lines without reprinting unchanged pane text.

By default, live mode prints each rendered update as plain text. Add `--redraw` to clear the terminal and repaint the latest workspace summary plus the current client-side pane surface on each update. When stdout is a TTY, `--redraw` uses the alternate screen and hides the cursor for the live session, then restores both on exit. Captured or piped stdout stays as plain clear/home escape output:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --redraw --stdin-bytes --interval-ms 500
```

Read-write live clients also poll process output during idle cycles. That means a process can update the backend-owned pane surface and stream patches to an attached read-write client even when the client has not sent a key frame in that cycle.

To send a resize intent before each live cycle while also sending pane input, pass both `--cols` and `--rows`:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --iterations 2 --key $'ping\n' --cols 100 --rows 30 --interval-ms 500
```

After the process-host resize succeeds, the daemon commits the pane size into the workspace tree and republishes a `WorkspaceTreeSnapshot`. The CLI prints the updated workspace summary, including the committed size and daemon-published resize policy.

The local daemon publishes `resize=fixed` by default. Use `nmuxd --resize-policy fixed|leader|active-client|manual` to advertise a different pane resize policy. `manual` ignores frontend viewport resize intents; explicit user-command resize intents remain eligible. Explicit `--cols` and `--rows` values must both be in the local PTY range, 1 through 65535, and cannot be combined with `--no-input`. A resize-only live client attaches read-write and sends explicit `--cols`/`--rows` as a user-command resize intent, so the daemon can apply that control intent without sending pane input even when the published policy is `manual`.

For a resize-only live control intent, omit pane input flags:

```sh
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --iterations 1 --cols 100 --rows 30
```

Read-only actors that send resize intents receive `PermissionDenied`; the stock CLI treats `--no-input` as observation-only and rejects `--no-input` with `--cols`/`--rows` before connecting.

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

For explicit input events, `--paste TEXT`, `--focus gained|lost`, `--key-name NAME`, and `--mouse action:button:row:col` send structured input frames instead of raw text in one-shot and live mode. Named keys include `enter`, `tab`, `backspace`, `escape`, `insert`, `delete`, `home`, `end`, `page-up`, `page-down`, `f1` through `f12`, `keypad-enter`, `keypad-0` through `keypad-9`, and `arrow-up|arrow-down|arrow-right|arrow-left`. `--key-modifiers MODS` can accompany `--key-name`; use `shift`, `ctrl`, `alt`, `super`, or a `+`/`,` combination such as `ctrl+shift`. Mouse actions are `press`, `release`, or `motion`; buttons are `none`, `left`, `middle`, `right`, `wheel-up`, or `wheel-down`; row and column are 1-based cells in the CLI and are converted to zero-based protocol coordinates. `--mouse-modifiers MODS` uses the same modifier names for mouse input. Focus, named-key, paste, and mouse forwarding are encoded by the daemon from daemon-owned pane modes rather than cached client mode state, and mouse coordinates are validated against the daemon-owned pane size.

If an input event is valid protocol but cannot be encoded by the active terminal
engine, if the host refuses the forwarded bytes, or if a live resize cannot be
applied by the host, the daemon returns an `Error` frame and the CLI prints the
server-provided reason instead of reporting an ambiguous closed connection.
One-shot clients also check for an input error before requesting scrollback, so
unsafe paste, encoding failures, and host input failures are reported directly.
Pane-scoped input, resize, and scrollback requests for unknown panes return a
`PaneNotFound` error instead of waiting for a response that will never arrive.

For read-only live observation, use `--no-input`:

```sh
rm -f /tmp/nmux.sock
nix develop path:$PWD -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --live-cycles 3 --command "printf 'ready\n'; sleep 0.05; printf 'tick-one\n'; sleep 0.05; printf 'tick-two\n'; sleep 1"
nix develop path:$PWD -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --no-input --interval-ms 500
```

When an unbounded live client exits because the daemon closes the live socket, the client reports `nmux: live server closed connection` on stderr.

The read-only client attaches once, sends no input or resize control intents, and prints streamed surface updates when the daemon observes process output. If `--iterations` is omitted, it keeps polling until the daemon closes the live connection.

This is not a terminal emulator yet. The interim text surface only converts simple output bytes into backend-owned visible rows and scrollback. It proves the first local daemon/client path: server-owned workspace state, server-owned pane surface state derived from a local PTY, server-owned scrollback ranges, FlatBuffers envelope framing, client-side rendering from decoded state objects, and client-to-daemon input forwarding.
`nmuxd --terminal-engine interim` selects this current implementation explicitly. Backend `libghostty-vt` extraction is imported behind the `libghostty-vt` Cargo feature, but the default build keeps the interim engine to avoid making the native Ghostty/Zig build part of every development loop.

## Optional libghostty-vt Build

The experimental backend VT engine is gated behind the `libghostty-vt` Cargo
feature. The dev shell pins Zig 0.15 because the Ghostty commit used by
`libghostty-vt-sys` requires that Zig version.

The opt-in engine keeps PTY bytes in `nmuxd` and maps Ghostty state back into
nmux snapshots, patches, and scrollback chunks. The local CLI prints non-empty
terminal title and OSC 7 working-directory metadata alongside the rendered pane
surface, including redraw output.

Current coverage includes:

- cursor position/visibility/shape/blink, terminal title metadata, OSC 7
  working-directory metadata, terminal color state, render-state default
  colors/palette, palette overrides, and explicit cursor color;
- style-separated visible rows, styled scrollback rows, style-bearing trailing
  blanks, cell widths, combining marks, emoji ZWJ clusters, basic SGR style
  flags, underline color, palette-indexed colors, row-level dirty state, row
  state hashes, hyperlink presence, and Kitty graphics placeholder metadata;
- alternate-screen entry/restoration with alternate scrollback omission,
  resize/reflow, committed live resize metadata, metadata-only no-row patches,
  style-table full-refresh reattach, sparse row updates, and mode-only surface
  patches;
- OSC 133 row semantic prompt state and per-run semantic content;
- bracketed paste, paste safety validation, paste forwarding, mouse tracking
  and pane-bounds/mode-gated mouse forwarding with modifiers, focus reporting and
  daemon-gated focus forwarding with Error frames, application keypad tracking, common named-key
  forwarding, mode-aware keypad Enter/digit forwarding, mode-aware arrow-key
  forwarding, engine-backed key encoding with modifier preservation, explicit
  encoder output, origin mode, wraparound mode, and mode-aware key encoding.

Image placement data, hyperlink IDs, and broader shell command metadata remain
intentionally withheld until the expected backend behavior and nmux protocol
shape are clear.

```sh
nix develop path:$PWD -c make check-ghostty-vt
```

The target runs full `nmux-core` and `nmux-cli` test suites with
`--features libghostty-vt`, and sets `GIT_CONFIG_GLOBAL=/dev/null`. That Git
setting is not logically required by nmux; it avoids a local Git configuration
that rewrites GitHub HTTPS URLs to SSH. The
`libghostty-vt-sys` build script fetches Ghostty from an HTTPS URL unless
`GHOSTTY_SOURCE_DIR` points at an existing Ghostty checkout.

## Presence And Attach Modes

The FlatBuffers `AttachRequest` carries actor ID, user metadata, focused pane, and attach mode. The daemon replies with `PresenceUpdate`.

Current behavior:

- read-write actors may send pane input
- read-only actors may receive workspace, presence, surface, and scrollback state
- read-write local input is forwarded to the process host
- output is polled again after forwarded input so echoed text can update backend-owned scrollback
- read-only actors do not send pane input or resize control intents in the local
  client flow

The local skeleton currently accepts clients sequentially. Simultaneous multi-client attach is a later expansion.

## Reconnect Behavior

Reconnect metadata is carried by `AttachRequest.known_surfaces`.

Current behavior:

- no known surface version: daemon sends a full `PaneSurfaceSnapshot`
- known `pane-1` surface version is current: daemon sends no surface frame
- known `pane-1` surface version is patchable: daemon sends a `PaneSurfacePatch`
- known `pane-1` surface version has a latest `FullRefreshRequired` update:
  daemon sends a full `PaneSurfaceSnapshot`
- known `pane-1` surface version is stale: daemon sends a full `PaneSurfaceSnapshot`

The CLI can persist its local render state with `--state`. This records the
rendered pane surface, terminal title, OSC 7 working directory, terminal modes
including mouse tracking mode/format,
cached row runs, the cached style table, terminal color state, OSC 133 row/run
semantic metadata, row dirty flags, row state hashes, Kitty placeholder row
metadata, last known surface version, and last seen scrollback range/version
metadata, so a later process can request a surface patch, apply it to the
cached surface, or render the cached current surface when the daemon has no
newer surface frame, instead of replaying raw PTY bytes. The state file is scoped to
the daemon socket identity, so a recreated socket path forces a fresh snapshot
instead of reusing stale rows from an older daemon. Older state files that only
contain rendered row text still load as default-style rows with default
metadata, default modes, default color state, and no scrollback metadata, but
they also force one fresh snapshot before being rewritten with the current
socket scope. When a later request asks for the same scrollback range, the CLI
still fetches daemon-owned scrollback even if the visible surface is already
current. It sends the cached scrollback version as a fetch precondition and
retries once without that precondition if the daemon reports `StaleVersion`.
Explicit one-shot text, paste, named-key, focus, and mouse input is still
forwarded before the scrollback fetch when the visible surface is already
current. When a scoped state file is already current and the daemon sends no
surface frame, explicit live key, paste, named-key, focus, and mouse input is
still sent to daemon-owned input handling; disabled focus or mouse modes return
protocol `Error` frames instead of relying on cached client mode state.

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
