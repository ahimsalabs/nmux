# Running nmux

The current default workflow is a local state-sync prototype with a real local
PTY host behind the daemon. `nmux daemon` owns one workspace tree, a daemon-owned
terminal surface, scrollback, pane metadata, resize policy, and the local
process host. `nmux` can attach once, run a live attach loop, reattach
read-only, persist client render state, request daemon-owned scrollback ranges,
send explicit text, paste, named-key, focus, mouse, or resize intents, and print
nested `NMUX_*` pane context from commands running inside a pane. The runnable
default-engine smoke path covers live stdin, persisted reattach, nested context
reporting, and same-path socket recreation so stale cached surfaces do not leak
into a new daemon.

## Quick Start

Run the default end-to-end smoke first:

```sh
nix develop . -c just local-smoke
```

A successful run ends with:

```text
local_smoke_reattach=passed
local_smoke_socket_recreation=passed
local_smoke_print_context=passed
local_smoke_json_info=passed
local_smoke_ready_json=passed
local_smoke_managed_start=passed
local_smoke=passed
```

For the default local workspace, run `nmux` directly. In an interactive TTY it
starts the shared local daemon when needed, attaches with live byte input and
redraw, and leaves the daemon running when you detach:

```sh
nix develop . -c cargo run --bin nmux
```

Rerun the same command from another terminal to reattach. Detach the live
client with Ctrl-] by default, or pass `--detach-key none` to forward that byte
to the pane. Stop the shared daemon with `nmux kill` when finished.

For manual daemon control, start the daemon in one shell and attach from
another:

```sh
# shell 1
nix develop . -c cargo run --bin nmux -- daemon --live-forever
# shell 2
nix develop . -c cargo run --bin nmux -- --live --stdin-bytes --redraw
```

This uses the default `libghostty-vt` engine in default-feature builds and the
shared default socket path unless `--socket` or `NMUX_SOCKET` selects a
different local workspace.
Use `--session NAME` or `-s NAME` on `nmux daemon` to publish a non-default
session name; clients can target the same daemon with `nmux --session NAME`,
`nmux attach NAME`, `nmux new NAME` for managed private sessions, or
`nmux kill NAME` to stop the matching daemon-owned session. The current daemon
still owns one session, so a mismatched target name fails clearly rather than
selecting from a multi-session server.

For a private local workspace owned by one client command, use `--start`:

```sh
nix develop . -c cargo run --bin nmux -- --start --command "printf 'hello from pty\n'; cat >/dev/null"
nix develop . -c cargo run --bin nmux -- --start --cwd "$PWD" --env NMUX_DEMO=1 --command 'printf "cwd:%s env:%s\n" "$PWD" "$NMUX_DEMO"; cat >/dev/null'
nix develop . -c cargo run --bin nmux -- --start --startup-timeout-ms 10000 --command "$SHELL"
nix develop . -c cargo run --bin nmux -- --shell
```

Without `--live`, `--start` runs a managed daemon in one-shot ready-json mode.
With `--live`, it runs a managed daemon in live-forever ready-json mode. Both
forms use a short temporary socket path by default, wait for the daemon
readiness event internally, attach through the normal nmux protocol, and stop
the managed daemon when the client exits. If `--command SHELL` is omitted, the
managed daemon runs `$SHELL` and falls back to `sh`. Managed `--cwd DIR` must
name an existing directory; it and repeatable `--env KEY=VALUE` are passed to
the private daemon before daemon-owned `NMUX_*` identity variables are injected.
`--startup-timeout-ms MS` controls the managed readiness wait before the client
kills the private daemon and reports setup failure. With `--json`, managed
startup failures are reported as client JSON error objects using the daemon
readiness error message rather than nesting daemon JSON inside a string.
Bare interactive `nmux` uses the same live byte-input redraw path. It attaches
to the default socket when one exists, otherwise it starts a persistent shared
local daemon and attaches to it. `nmux --shell` forces the private-shell form
and expands to `--start --live --stdin-bytes --redraw`. In that path the client
uses raw stdin, mirrors daemon-published
mouse tracking onto the host terminal, and forwards modified named keys plus
SGR mouse/scroll input through the same daemon-owned gates used by explicit
`--key-name` and `--mouse` input.

Each attach starts with an `AttachRequest` carrying actor identity, attach mode,
focused pane, and known pane surface versions. The daemon polls process output
into backend-owned pane state, then sends a `WorkspaceTreeSnapshot`,
`PresenceUpdate`, `AttachStatus`, and, when needed, either a
`PaneSurfaceSnapshot` or a `PaneSurfacePatch`. Decoded workspace, surface,
attach-status, and scrollback state requires non-empty session, tab, and pane
IDs before it can update client render/cache state. `AttachStatus` identifies
the attached pane and says whether the client is already current or a surface
frame follows. `nmux` applies those state objects to a client-side pane surface
render state before printing, then can send post-attach control frames and
render returned `ScrollbackChunk` objects.

Run the default-engine checks:

```sh
nix develop . -c just check
nix develop . -c just local-smoke
```

See [docs/contributor-workflow.md](contributor-workflow.md) for which checks
apply to default-engine work, terminal-correctness work, and promotion evidence.
Use [docs/toolchain.md](toolchain.md) for the complete Nix command list,
including `just check-ghostty-vt`, `just check-all`, source-fetch verifiers,
and packaging/promotion evidence targets.
`just local-smoke` starts a temporary local daemon, sends live stdin through a
client, reattaches a read-only client with persisted state, verifies the echoed
output remains visible, verifies nested `nmux --print-context` receives the
pane identity environment, verifies JSON informational flags and daemon
readiness JSON, verifies managed `nmux --start --json`, then reuses the same
socket path for a new daemon and checks that the old cached surface is not
rendered. It is the shortest runnable end-to-end workflow check for the default
engine.

For scripts that start a daemon and then attach a client, add `--ready-json` to
`nmux daemon`. It prints one stdout line after the socket is bound and the
initial pane has started:

```json
{"event":"ready","NMUX_SOCKET":"/tmp/nmux.sock","source":"--socket","mode":"live-forever","terminal_engine":"libghostty-vt","resize_policy":"fixed"}
```

If startup fails before that point, `--ready-json` prints an error event before
the usual stderr message:

```json
{"event":"error","error":{"message":"socket path already exists: /tmp/nmux.sock; remove it if it is stale or pass --socket PATH for a different workspace"}}
```

Run the timed default-plus-interim-fallback validation sample when gathering
local release evidence:

```sh
nix develop . -c just promotion-sample
```

See [docs/toolchain.md](toolchain.md) for the supported Nix development path
and the non-Nix requirements checklist.
See [docs/source-fetch-policy.md](source-fetch-policy.md) for the
`libghostty-vt-sys` fetch policy.
See [docs/packaging.md](packaging.md) for the current no-release-binary stance.
See [docs/protocol-futures.md](protocol-futures.md) for withheld protocol
objects that need ADRs before schema changes.

Inspect the installed binary versions without connecting or binding a socket:

```sh
nix develop . -c cargo run --bin nmux -- --version
nix develop . -c cargo run --bin nmux -- daemon --version
nix develop . -c cargo run --bin nmux -- version
nix develop . -c cargo run --bin nmux -- daemon --version
nix develop . -c cargo run --bin nmux -- --version-json
nix develop . -c cargo run --bin nmux -- daemon --version-json
nix develop . -c cargo run --bin nmux -- version --json
```

Plain version output keeps the Cargo package version and adds a concise build
suffix. JSON version output also includes `channel`, `commit`, and
`build_date` fields for release and nightly provenance.

Start a one-shot daemon with the default local shell:

```sh
# shell 1
nix develop . -c cargo run --bin nmux -- daemon --socket /tmp/nmux.sock --one-shot
```

In another shell, attach a client:

```sh
# shell 2
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock
```

The output starts with the current workspace summary. The following rows depend
on the user's default shell and startup files, so use an explicit `--command`
when you need deterministic smoke-test text.

For scripts, add `--json` to a one-shot attach. The client prints one object
with the workspace, authoritative attach status, terminal metadata, current
surface text, structured current-surface rows/styles/hyperlinks, and requested
scrollback rows with their structured metadata. Setup failures, state-save
failures, and protocol errors are emitted as structured JSON error objects:

```sh
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --json
```

For a deterministic PTY-output smoke test, run the daemon with an explicit shell command:

```sh
nix develop . -c cargo run --bin nmux -- daemon --socket /tmp/nmux.sock --one-shot --command "printf 'hello from pty\n'; cat >/dev/null"
```

Use `--cwd DIR` and repeatable `--env KEY=VALUE` when the pane command needs an
existing launch directory or explicit environment. nmux still injects authoritative
`NMUX_*` pane identity variables after user-provided env values:

```sh
nix develop . -c cargo run --bin nmux -- daemon --socket /tmp/nmux.sock --one-shot --cwd "$PWD" --env NMUX_DEMO=1 --command 'printf "cwd:%s env:%s\n" "$PWD" "$NMUX_DEMO"; cat >/dev/null'
```

Expected output after attaching the client:

```text
session=local tab=tab-1 pane=pane-1 size=80x24 resize=fixed
booting nmux workspace
nmux pane-1
server-owned terminal state
hello from pty
scrollback 1..2:
booting nmux workspace
nmux pane-1
```

To prove input-driven output across attaches, keep the daemon running with a command that echoes each submitted line:

```sh
# shell 1
nix develop . -c cargo run --bin nmux -- daemon --socket /tmp/nmux.sock --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done"
```

Then send input from one client:

```sh
# shell 2
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --key $'ping\n' --scrollback-start 3 --scrollback-count 3
```

Attach a second passive client and fetch the same range plus the echoed line:

```sh
# shell 2
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --no-input --scrollback-start 3 --scrollback-count 4
```

Expected second attach output includes:

```text
ready
ping
echo:ping
```

To keep one local frontend process polling for server-owned surface updates, run a bounded follow loop:

```sh
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --follow --iterations 3 --interval-ms 500 --state /tmp/nmux-follow.state --scrollback-start 1 --scrollback-count 1
```

`--follow` is a local reconnect loop over the current request/response protocol. It keeps one in-process client render state, sends known pane surface versions on each reconnect, applies snapshots or patches when the daemon has newer state, and renders the scoped cached surface when `AttachStatus` reports the attached pane is already current. Follow mode is read-only for now, so it rejects input flags instead of repeatedly sending input.

Inspect a persisted client cache without connecting to a daemon:

```sh
nix develop . -c cargo run --bin nmux -- --state /tmp/nmux-follow.state --state-info
nix develop . -c cargo run --bin nmux -- --state /tmp/nmux-follow.state --state-info-json
```

For a daemon that keeps serving snapshots, omit `--one-shot`.

## Live Attach CLI

Live attach keeps one local connection open for repeated input/output cycles.
It is the current CLI workspace path, not a full terminal-emulator UI; without
an explicit input or resize flag it observes read-only, while `--key` sends the
same text on each bounded client cycle, `--stdin` line-streams stdin, and
`--stdin-bytes` forwards raw stdin byte chunks. It renders streamed pane surface
updates through the same client-side pane surface state used by reconnects.
Explicit input modes such as `--key`, `--key-name`, `--paste`, `--focus`,
`--mouse`, `--stdin`, `--stdin-bytes`, and `--no-input` are mutually exclusive.
Live-only frontend flags such as `--stdin`, `--stdin-bytes`, `--redraw`, and
`--speculative-echo`, and `--cols`/`--rows` are rejected unless `--live` is set,
so ignored-mode mistakes fail before the client tries to connect.
`nmux --start` is the single-command form for a private managed daemon; it uses
an isolated temporary socket and state path unless `--socket` or `--state` is
supplied explicitly. Add `--live` when the managed daemon should remain
attached after the initial one-shot response. Add `--cwd DIR` for an existing
working directory or repeatable `--env KEY=VALUE` when the managed pane command
needs launch context without a separate daemon shell. Add
`--startup-timeout-ms MS` when slow local startup needs a longer private-daemon
readiness window than the default 5000 ms.
Use `nmux --shell` when you specifically want a private interactive shell that
is cleaned up with the client.

By default, `nmux daemon` and `nmux` use the same local socket path. The precedence
is explicit `--socket`, then a valid absolute `NMUX_SOCKET`, then
`$XDG_RUNTIME_DIR/nmux/nmux.sock` when `XDG_RUNTIME_DIR` is a valid absolute
path, otherwise `/tmp/nmux-$UID/nmux.sock`. Use `NMUX_SOCKET` for a
shell-scoped local workspace, or pass `--socket` on both sides when you want an
isolated smoke-test socket.
Use `nmux --print-socket` or `nmux daemon --print-socket` to print the resolved socket
path without connecting or binding; use `--print-socket-json` to include both
the path and resolution source for scripts.
Informational flags such as `--version`, `--version-json`, `--help`,
`--print-socket`, `--print-socket-json`, client `--print-context`, and client
`--print-context-json`, `--state-info`, and `--state-info-json` exit before mode
validation or socket/PTY work, so scripts can reuse broader command templates
without accidentally opening a connection or starting a pane process.
`--state-info` and `--state-info-json` require `--state PATH` and inspect the
persisted client cache without connecting. They include the selected socket
path, whether it exists, and whether the persisted cache scope matches that live
socket identity. `--state-info-json` reports setup failures as JSON error
objects.

```sh
NMUX_SOCKET=/tmp/nmux-project.sock nix develop . -c cargo run --bin nmux -- daemon --print-socket
NMUX_SOCKET=/tmp/nmux-project.sock nix develop . -c cargo run --bin nmux -- --print-socket
NMUX_SOCKET=/tmp/nmux-project.sock nix develop . -c cargo run --bin nmux -- --print-socket-json
nix develop . -c cargo run --bin nmux -- --state /tmp/nmux-live.state --state-info-json
```

Direct TCP is available for local-lab and tailnet experiments. Start the daemon
with `nmux daemon --listen HOST:PORT --token TOKEN`. Attach with
`nmux HOST:PORT --token TOKEN`, `nmux --tcp HOST:PORT --tcp-token TOKEN`, or
set `NMUX_TOKEN` instead of passing a token flag. A positional host without a
port uses port 7007. This is direct token-authenticated TCP, not SSH bootstrap.

```sh
nix develop . -c cargo run --bin nmux -- daemon --listen 127.0.0.1:7007 --token TOKEN --live-forever
nix develop . -c cargo run --bin nmux -- 127.0.0.1:7007 --token TOKEN --live --stdin-bytes --redraw
```

If the daemon is not running or the client points at the wrong socket, `nmux` reports the socket path in the connection error.
When a script starts `nmux` before `nmux daemon` has finished binding, pass `--connect-timeout-ms MS` so the client waits for the socket instead of failing immediately.
If `nmux daemon` cannot bind the socket path, it reports that path. If a socket path already exists, the daemon refuses to replace it and includes a recovery hint; remove a stale socket only after confirming no daemon is using it, or pass a different `--socket`.
On normal bounded exits, the daemon removes the socket path it created if that path still points at the same socket file.
Commands started in the local PTY receive `NMUX=1`, `NMUX_SESSION_ID`,
`NMUX_PANE_ID`, `NMUX_SOCKET`, and `NMUX_ORIGIN` in their environment. These
are local pane identity hints for nested nmux tooling and do not make the
frontend replay raw PTY bytes. If `nmux daemon` is launched from inside an nmux pane,
it appends the inherited origin to the child pane origin with `>` so nested
tools can see the local hop chain. Inside a pane, `nmux --print-context` prints
the inherited `NMUX_*` key/value lines without connecting, and
`nmux --print-context-json` prints the same context as a JSON object; outside a
complete nmux pane context, both fail before socket or state work, and the JSON
form reports that setup failure as a JSON error object.

To smoke the nested context path through a real daemon-owned PTY, start a
one-shot daemon whose pane command invokes the client binary:

```sh
rm -f /tmp/nmux-context.sock
# terminal 1
nix develop . -c cargo run --bin nmux -- daemon --socket /tmp/nmux-context.sock --one-shot --command 'cargo run --bin nmux -- --print-context; cat >/dev/null'
# terminal 2
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux-context.sock
```

Scrollback ranges are 1-based from the oldest retained row, and the client rejects zero `--scrollback-start`, `--scrollback-count`, or `--scrollback-tail` values before connecting. Use `--scrollback-tail COUNT` when a client should resolve the most recent retained rows without knowing the current total first; it cannot be combined with an explicit start/count range. Use `--no-scrollback` when a one-shot or live attach should render only the current surface; it cannot be combined with explicit scrollback range flags. Out-of-range requests can return an empty scrollback section; `--state` does not persist those zero-row chunks as future version preconditions.

Start a daemon that serves one live client for two input cycles:

```sh
nix develop . -c cargo run --bin nmux -- daemon --live-cycles 2 --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
```

Use `--live` instead of `--live-cycles` to keep serving that one live client until the client detaches:

```sh
rm -f /tmp/nmux.sock
# shell 1
nix develop . -c cargo run --bin nmux -- daemon --socket /tmp/nmux.sock --live --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
```

Use `--live-forever` to keep the same daemon-owned workspace and PTY alive for sequential live clients until you stop it with Ctrl-C in the daemon shell:

```sh
nix develop . -c cargo run --bin nmux -- daemon --live-forever --ready-json --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
```

Use `--live-clients COUNT` to keep the same daemon-owned workspace and PTY alive for a bounded number of sequential live clients:

```sh
rm -f /tmp/nmux.sock
# shell 1
nix develop . -c cargo run --bin nmux -- daemon --socket /tmp/nmux.sock --live-clients 2 --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
# shell 2
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --state /tmp/nmux-live.state --live --iterations 1 --key $'first\n'
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --state /tmp/nmux-live.state --live --no-input --iterations 1 --scrollback-start 1 --scrollback-count 8
```

The second live client attaches to the same backend-owned pane state and can observe output produced by the first live client. With `--state`, it sends its known pane surface version and renders the cached current surface when the daemon has no newer surface update to send.

Attach a bounded live client:

```sh
nix develop . -c cargo run --bin nmux -- --live --iterations 2 --key $'ping\n' --interval-ms 500
```

Expected output includes the initial surface and two streamed updates ending in `echo:ping`. The client uses `--interval-ms` as a read timeout for optional update frames. If no output is produced for a cycle, the client continues until the bounded iteration count is reached. Bounded live and follow loops reject `--iterations 0` before connecting, and `--interval-ms` must be greater than zero.
For scripts, add `--json` to one-shot or follow attach to print machine-readable
attach objects instead of renderer text, or add it to live mode to print
newline-delimited attach, workspace, and surface update events. Attach and
surface events include structured terminal state, row payloads, style tables,
and hyperlink tables for scripts that need more than rendered fallback text.
Successful live JSON sessions end with a `detach` event whose reason is
`iteration-limit`, `stdin-eof`, `local-detach`, or `server-closed`, so scripts
do not need to parse stderr lifecycle notes.
Live JSON also emits `error` events for setup failures before attach, such as a
corrupt `--state` file or missing daemon socket, for final state-save failures,
and for protocol errors after attach.
`--json` is mutually exclusive with `--redraw`.

Add `--record PATH` to live mode to write the same structured event stream to a
newline-delimited JSON file with an `elapsed_ms` timestamp on each event. The
record includes the initial attach surface, presence, streamed workspace and
surface updates, protocol errors, and detach reason. `nmux replay PATH` reads
that file and prints recorded attach/surface text frames in order; it is a
surface-state playback aid, not raw PTY replay.

Live mode renders the requested initial scrollback range after the first attached surface, using `--scrollback-start`/`--scrollback-count` or `--scrollback-tail`, unless `--no-scrollback` asks for a current-surface-only attach; the printed header reports the actual returned row range and includes the total when the response is not the tail. In `--redraw` mode, that initial scrollback context is included in the first repaint buffer before the current pane surface. Live mode can also use `--state` to persist the client-side pane surface cache. On attach, the client sends known pane surface versions from that file; streamed snapshots and patches update the same cache, and a current-version attach renders the cached surface without PTY byte replay. If the state file is corrupt or cannot be written, `nmux` reports the state path in the error. State saves write a temporary file in the target directory and rename it into place. In non-redraw mode, metadata-only `CursorOnly` updates carry no row changes and print changed title or working-directory lines without reprinting unchanged pane text.

By default, live mode prints each rendered update as plain text. Add `--redraw` to repaint the current client-side pane surface on each update. When stdout is a TTY, `--redraw` uses differential rendering: only rows that actually changed are rewritten via cursor-addressed updates, eliminating full-screen flicker. A latency counter overlay in the top-right corner (inverse video) shows milliseconds between frames. When the `libghostty-vt` engine produces structured style data, the client reconstructs ANSI SGR escape sequences (bold, italic, 24-bit RGB colors) from the protocol's structured `StyleSummary` objects — the client never parses a raw VT byte. Styled output and the latency overlay persist across detach/reattach through the state file. Cursor-only and mode-only patches with unchanged metadata are no-ops in redraw mode. `--redraw` also uses the alternate screen and hides the cursor for the live session, restoring both on exit. Captured or piped stdout falls back to plain clear/home escape output without differential rendering or styling:

```sh
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --redraw --stdin-bytes --interval-ms 500
```

Add `--speculative-echo` only with `--redraw` and `--key` to enable the
experimental client-local local echo overlay. It predicts one outstanding
printable single-cell append at the confirmed cursor position, repaints the
redraw buffer immediately with the predicted glyph underlined, and then
replaces that overlay with the next daemon-owned surface update. It does not
change the confirmed client cache,
protocol frames, scrollback, or daemon terminal state, and it deliberately skips
raw stdin-byte mode, line-streamed stdin, paste, named keys, control input, wide
graphemes, wrapping, alternate-screen claims, and JSON output. Repeated
mismatches suppress prediction temporarily; after a short run of otherwise
predictable skipped keys, the overlay tries again.

Read-write live clients also poll process output during idle cycles. That means a process can update the backend-owned pane surface and stream patches to an attached read-write client even when the client has not sent a key frame in that cycle.

To send a resize intent before each live cycle while also sending pane input, pass both `--cols` and `--rows`:

```sh
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --iterations 2 --key $'ping\n' --cols 100 --rows 30 --interval-ms 500
```

After the process-host resize succeeds, the daemon commits the pane size into the workspace tree and republishes a `WorkspaceTreeSnapshot`. The CLI prints the updated workspace summary, including the committed size and daemon-published resize policy.

The local daemon publishes `resize=fixed` by default. Use `nmux daemon --resize-policy fixed|leader|active-client|manual` to advertise a different pane resize policy. `manual` ignores frontend viewport resize intents; explicit user-command resize intents remain eligible. `nmux daemon --cols COUNT --rows COUNT` sets the initial local PTY and workspace pane size before the command starts. Client-side `nmux --live --cols COUNT --rows COUNT` sends a post-start resize intent. Both forms require dimensions in the local PTY range, 1 through 65535; the live-client form cannot be combined with `--no-input`. A resize-only live client attaches read-write and sends explicit `--cols`/`--rows` as a user-command resize intent, so the daemon can apply that control intent without sending pane input even when the published policy is `manual`.

For a resize-only live control intent, omit pane input flags:

```sh
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --iterations 1 --cols 100 --rows 30
```

Read-only actors that send resize intents receive `PermissionDenied`; the stock CLI treats `--no-input` as observation-only and rejects `--no-input` with `--cols`/`--rows` before connecting.

For line-streamed live input, pipe lines through stdin:

```sh
printf 'ping\npong\n' | nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --stdin --interval-ms 500
```

With `--stdin`, each input line is read and sent during its live input cycle, after the client has attached. If `--iterations` is omitted, stdin EOF ends the client loop without falling back to the default `--key` input and the client reports `nmux: stdin EOF; detached` on stderr. If `--iterations` is present, the client runs at most that many stdin cycles. The daemon treats client EOF during live read-write polling as a clean detach.

For byte-streamed live input, use `--stdin-bytes`:

```sh
printf 'ping\npong\n' | nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --stdin-bytes --interval-ms 500
```

`--stdin-bytes` reads stdin on a background thread and sends available chunks during the live polling loop as `InputKind.RawBytes`. This keeps output polling active even while no complete input line is available. When stdin is an interactive TTY, the client temporarily uses raw input and local echo defaults off for this mode; piped stdin is left untouched. Pass `--local-echo tty` to preserve the TTY's existing echo setting while still using raw byte input. Interactive byte mode also listens for `SIGWINCH` and sends resize intents from the current TTY size unless explicit `--cols` and `--rows` were supplied. When both stdin and stdout are TTYs, byte mode warns once on stderr that the interim text surface lacks full VT fidelity; piped and scripted runs stay quiet. Press Ctrl-] to detach from a byte-streamed live session; use `--detach-key none` to pass Ctrl-] through to the pane instead. The client reports local detach on stderr. When unbounded byte-streamed stdin reaches EOF, the client reports `nmux: stdin EOF; detached`.

For explicit input events, `--paste TEXT`, `--focus gained|lost`, `--key-name NAME`, and `--mouse action:button:row:col` send structured input frames instead of raw text in one-shot and live mode. Named keys include `enter`, `tab`, `backspace`, `escape`, `insert`, `delete`, `home`, `end`, `page-up`, `page-down`, `f1` through `f12`, `keypad-enter`, `keypad-0` through `keypad-9`, and `arrow-up|arrow-down|arrow-right|arrow-left`. `--key-modifiers MODS` can accompany `--key-name`; use `shift`, `ctrl`, `alt`, `super`, or a `+`/`,` combination such as `ctrl+shift`. Mouse actions are `press`, `release`, or `motion`; buttons are `none`, `left`, `middle`, `right`, `wheel-up`, or `wheel-down`; row and column are 1-based cells in the CLI and are converted to zero-based protocol coordinates. `--mouse-pixels X:Y` can accompany `--mouse` to provide explicit pixel coordinates for terminals using SGR-pixels mouse mode. `--mouse-modifiers MODS` uses the same modifier names for mouse input; decoded protocol input rejects missing input payload tables, missing or empty pane/actor IDs, and modifier bits outside the low four `shift`, `ctrl`, `alt`, and `super` bits. Decoded protocol errors reject missing or empty messages and empty pane IDs when present. Focus, named-key, paste, and mouse forwarding are encoded by the daemon from daemon-owned pane modes rather than cached client mode state, and mouse cell and pixel coordinates are validated against the daemon-owned pane size.

If an input event is valid protocol but cannot be encoded by the active terminal
engine, if the host refuses the forwarded bytes, or if a live resize cannot be
applied by the host, the daemon returns an `Error` frame and the CLI prints the
server-provided reason instead of reporting an ambiguous closed connection. The
CLI also includes structured protocol attribution such as error code, pane ID,
retryable flag, and originating `InputEvent.input_seq` when those fields are
present.
One-shot clients also check for an input or output-polling error before
requesting scrollback, so unsafe paste, encoding failures, host input failures,
and host output polling failures are reported directly.
Pane-scoped input, resize, and scrollback requests for unknown panes return a
`PaneNotFound` error instead of waiting for a response that will never arrive.
If the daemon cannot resolve its active tab or active pane while setting up an
attach, it also returns `PaneNotFound` instead of guessing `pane-1`.

For read-only live observation, use `--no-input`:

```sh
rm -f /tmp/nmux.sock
nix develop . -c cargo run --bin nmux -- daemon --socket /tmp/nmux.sock --live-cycles 3 --command "printf 'ready\n'; sleep 0.05; printf 'tick-one\n'; sleep 0.05; printf 'tick-two\n'; sleep 1"
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --no-input --interval-ms 500
```

When an unbounded live client exits because the daemon closes the live socket, the client reports `nmux: live server closed connection` on stderr.

The read-only client attaches once, sends no input or resize control intents, and prints streamed surface updates when the daemon observes process output. If `--iterations` is omitted, it keeps polling until the daemon closes the live connection.

The default engine is `libghostty-vt` when nmux is built with default Cargo
features. PTY bytes stay in `nmux daemon`, and daemon-owned Ghostty VT state is
mapped into nmux snapshots, patches, and scrollback chunks. Builds made with
`--no-default-features` fall back to the legacy/debug interim text surface, and
default-feature builds can still select it explicitly with
`nmux daemon --terminal-engine interim`.

## libghostty-vt Build

The backend VT engine is enabled by the default `libghostty-vt` Cargo feature.
The dev shell pins Zig 0.15 because the Ghostty commit used by
`libghostty-vt-sys` requires that Zig version.

The backend engine keeps PTY bytes in `nmux daemon` and maps Ghostty state back
into nmux snapshots, patches, and scrollback chunks. The local CLI prints
non-empty terminal title and OSC 7 working-directory metadata alongside the
rendered pane surface, including redraw output.

To smoke the VT engine manually:

```sh
rm -f /tmp/nmux-vt.sock
# shell 1
nix develop . -c env GIT_CONFIG_GLOBAL=/dev/null cargo run -p nmux-cli --bin nmux -- daemon --socket /tmp/nmux-vt.sock --one-shot --command "printf '\033[31mvt engine\033[0m\n'; cat >/dev/null"
# shell 2
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux-vt.sock
```

Expected client output includes `vt engine` without raw ANSI escape bytes.

Current coverage includes:

- cursor position/visibility/shape/blink, terminal title metadata, OSC 7
  working-directory metadata, terminal color state, render-state default
  colors/palette, palette overrides, and explicit cursor color;
- style-separated visible rows, styled scrollback rows, style-bearing and
  semantic-content trailing blanks, cell widths, combining marks, emoji ZWJ clusters, SGR style
  flags, underline variants, underline color, palette-indexed colors, row-level dirty state, row
  state hashes, hyperlink presence, and Kitty graphics placeholder metadata;
- alternate-screen entry/restoration with alternate scrollback omission,
  resize/reflow, committed live resize metadata, metadata-only `CursorOnly`
  no-row patches,
  style-table full-refresh reattach, sparse row updates, and mode-only surface
  patches;
- OSC 133 row semantic prompt state and per-run semantic content;
- bracketed paste, paste safety validation, paste forwarding, mouse tracking
  and pane-bounds/mode-gated mouse forwarding with modifiers and optional
  SGR-pixel coordinates, focus reporting and
  daemon-gated focus forwarding with pane/input-attributed Error frames,
  application keypad tracking, common named-key forwarding, mode-aware keypad
  Enter/digit forwarding, mode-aware arrow-key forwarding, engine-backed key
  encoding with modifier preservation, explicit encoder output, origin mode,
  wraparound mode, and mode-aware key encoding.

Image placement data, wired hyperlink IDs, and broader shell command metadata
remain intentionally withheld until the expected backend behavior and nmux
protocol shape are clear.

```sh
nix develop . -c just check-ghostty-vt
```

The target runs the feature-enabled workspace tests with `RUST_TEST_THREADS=1`
and `GIT_CONFIG_GLOBAL=/dev/null`. The serial test-harness setting is part of
the current FFI-backed native-VT gate. The Git setting is not logically required
by nmux; it avoids a local Git configuration that rewrites GitHub HTTPS URLs to
SSH. The `libghostty-vt-sys` build script fetches Ghostty from an HTTPS URL
unless `GHOSTTY_SOURCE_DIR` points at an existing Ghostty checkout.

For interim-only validation, run `nix develop . -c just check-interim`.

## Presence And Attach Modes

The FlatBuffers `AttachRequest` carries actor ID, user metadata, focused pane, attach mode, and known pane surface versions. Decoded attach requests and presence updates reject missing or empty identity strings, empty focused pane IDs when present, and missing or empty known-surface pane IDs. The daemon replies with `PresenceUpdate` and `AttachStatus`. `AttachStatus.surface_state = Current` is the explicit no-surface-update attach barrier; `Snapshot` and `Patch` mean the corresponding surface frame follows immediately.

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
- known surface version for the attached pane is current: daemon sends `AttachStatus.surface_state = Current` and no surface frame
- known surface version for the attached pane is patchable: daemon sends `AttachStatus.surface_state = Patch` followed by a `PaneSurfacePatch`
- known surface version for the attached pane has a latest `FullRefreshRequired` update:
  daemon recovers during attach with `AttachStatus.surface_state = Snapshot`
  followed immediately by a full `PaneSurfaceSnapshot`
- known surface version for the attached pane is stale: daemon sends `AttachStatus.surface_state = Snapshot` followed by a full `PaneSurfaceSnapshot`

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
Use `nmux --state PATH --state-info-json` to inspect this cache shape from a
script without opening a socket; its socket fields tell the script whether the
cache belongs to the currently selected daemon socket.

Start a long-running command-backed daemon:

```sh
rm -f /tmp/nmux.sock /tmp/nmux-client.state
nix develop . -c cargo run --bin nmux -- daemon --socket /tmp/nmux.sock --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"$line\"; done"
```

Attach once and persist the rendered surface:

```sh
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --state /tmp/nmux-client.state --no-input --scrollback-start 1 --scrollback-count 1
```

Then attach again using the same state file after sending input from another client:

```sh
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --key $'ping\n' --scrollback-start 1 --scrollback-count 1
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --state /tmp/nmux-client.state --no-input --scrollback-start 1 --scrollback-count 4
```

The final attach sends the cached surface version for the attached pane in
`AttachRequest`. If the daemon has exactly one newer surface version, it sends
`PaneSurfacePatch`; `nmux` applies that patch to the persisted client surface
and updates `/tmp/nmux-client.state`. `AttachStatus.pane_id` remains the
authority for the attached pane; post-attach input, resize, and scrollback
control use that pane ID instead of assuming a fixed local pane name.

This proves the reconnect decision and client-side patch rendering through the public FlatBuffers attach metadata.

## Scrollback Behavior

The prototype keeps scrollback separate from the visible pane surface. Clients
request explicit 1-based ranges with `--scrollback-start` and
`--scrollback-count`, or ask the client to resolve the latest retained rows with
`--scrollback-tail COUNT`. The daemon replies with a `ScrollbackChunk` for the
attached pane. Persisted client state records last-seen scrollback range/version
metadata, still fetches daemon-owned scrollback when the visible surface is
current, and retries once without a version precondition if the daemon reports a
stale scrollback version. Tests assert that visible surfaces and scrollback
chunks stay backend-owned state objects rather than raw PTY replay.
