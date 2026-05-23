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
- contributor workflow notes in [docs/contributor-workflow.md](docs/contributor-workflow.md);
- toolchain notes in [docs/toolchain.md](docs/toolchain.md);
- runnable notes in [docs/running.md](docs/running.md);
- default-engine promotion evidence in [docs/default-engine-promotion.md](docs/default-engine-promotion.md);
- CI notes in [docs/ci.md](docs/ci.md), including the default gate and manual
  promotion evidence bundle workflow;
- future protocol-object tracks in [docs/protocol-futures.md](docs/protocol-futures.md);
- opt-in native source-fetch policy in [docs/source-fetch-policy.md](docs/source-fetch-policy.md);
- packaging notes in [docs/packaging.md](docs/packaging.md);
- native VT source-policy criteria in
  [docs/adr/0024-native-vt-source-policy-criteria.md](docs/adr/0024-native-vt-source-policy-criteria.md);
- native VT packaging criteria in
  [docs/adr/0025-native-vt-packaging-criteria.md](docs/adr/0025-native-vt-packaging-criteria.md);
- native VT CI promotion criteria in
  [docs/adr/0026-native-vt-ci-promotion-criteria.md](docs/adr/0026-native-vt-ci-promotion-criteria.md);
- the implementation roadmap in [docs/roadmap.md](docs/roadmap.md), currently focused on post-M14 default-engine promotion evidence, frontend hydration tracking, future protocol-object decisions, and local usability.

## Feature Status

What works today: local `nmuxd`/`nmux` workflows over a Unix socket, one-shot
attach, live attach, read-only reattach, explicit input and resize intents,
persisted client render state, daemon-owned scrollback fetches, nested
`NMUX_*` context reporting, default-engine CI, and an opt-in
`libghostty-vt` correctness path with promotion evidence bundles.

What is not default-ready: the default engine is still the interim text surface,
not a full VT-correct renderer; `libghostty-vt` is still opt-in because native
build cost, repeated CI evidence, non-Nix setup, source-fetch policy, and
packaging are not accepted yet. Frontend Ghostty renderer hydration is a
separate post-M14 track; it is not part of the backend default-engine promotion
evidence tracked in [docs/default-engine-promotion.md](docs/default-engine-promotion.md).

## First Run

Run the default end-to-end smoke first:

```sh
nix develop . -c make local-smoke
```

That starts a temporary `nmuxd`, sends live input through `nmux`, reattaches
read-only from persisted state, checks nested `nmux --print-context`, and proves
the client does not reuse stale state after a socket path is recreated.

For an interactive local workspace, start a daemon in one shell and attach from
another:

```sh
nix develop . -c cargo run --bin nmuxd -- --live-forever
nix develop . -c cargo run --bin nmux -- --live --stdin-bytes --redraw
```

Detach the live client with Ctrl-]. See [docs/running.md](docs/running.md) for
socket selection, one-shot attach, persisted reattach, scrollback, resize, and
opt-in `libghostty-vt` examples.

## Check

Use the narrowest gate that matches the work:

| Work | Commands |
| --- | --- |
| Try the local workflow | `nix develop . -c make local-smoke` |
| Normal default-engine or docs work | `nix develop . -c make check` and `nix develop . -c make local-smoke` |
| Backend `libghostty-vt` correctness work | `nix develop . -c make check` and `nix develop . -c make check-ghostty-vt` |
| Release-style or promotion evidence work | Follow [docs/contributor-workflow.md](docs/contributor-workflow.md) and [docs/toolchain.md](docs/toolchain.md). |

The Nix shell provides `flatc` through `pkgs.flatbuffers`; no separate
FlatBuffers install is needed for the schema check. `make check` is the regular
default-engine gate. `make local-smoke` runs a real local `nmuxd`/`nmux` live
daemon/client smoke over a temporary socket and state file. `make
check-ghostty-vt` is the opt-in full feature gate for backend `libghostty-vt`
changes and currently runs serially with `RUST_TEST_THREADS=1`.

For the complete target inventory, evidence bundle workflow, source-fetch
reports, and packaging/archive verifiers, see [docs/toolchain.md](docs/toolchain.md).
For choosing the right gate by change type, see
[docs/contributor-workflow.md](docs/contributor-workflow.md). CI details and
manual promotion evidence recording live in [docs/ci.md](docs/ci.md).

## Quick Smoke

Inspect the current CLI flags:

```sh
nix develop . -c cargo run --bin nmux -- --help
nix develop . -c cargo run --bin nmuxd -- --help
nix develop . -c cargo run --bin nmux -- --version
nix develop . -c cargo run --bin nmuxd -- --version
```

`nmux --help` also calls out the current renderer limitation: the default prototype uses an interim text surface, not a VT-correct terminal emulator. That is a sequencing device while the local state-sync/live workflow stays fast. Backend `libghostty-vt` extraction is available behind an opt-in Cargo feature, separate from the later question of hydrating a frontend Ghostty renderer from nmux-owned state.

Manual daemon/client examples below use two shells: start the `nmuxd` command in
shell 1 and leave it running, then run the matching `nmux` command in shell 2.
Use `make local-smoke` for a single-command smoke.

One-shot attach:

```sh
rm -f /tmp/nmux.sock
# shell 1
nix develop . -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --one-shot --command "printf 'hello from pty\n'; cat >/dev/null"
# shell 2
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock
```

Bounded live attach:

```sh
# shell 1
nix develop . -c cargo run --bin nmuxd -- --live-cycles 2 --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
# shell 2
nix develop . -c cargo run --bin nmux -- --live --iterations 2 --key $'ping\n' --scrollback-start 1 --scrollback-count 4 --interval-ms 500
```

Both binaries share a stable default socket path for the current user. Explicit `--socket` wins; otherwise a valid absolute `NMUX_SOCKET` value wins, then `$XDG_RUNTIME_DIR/nmux/nmuxd.sock` when `XDG_RUNTIME_DIR` is a valid absolute path, otherwise `/tmp/nmux-$UID/nmuxd.sock`. Use `NMUX_SOCKET` for a shell-scoped local workspace, or pass `--socket` on both sides when you want an isolated smoke-test socket.
Use `nmux --print-socket` or `nmuxd --print-socket` to print the resolved socket path without connecting or binding.
Informational flags such as `--version`, `--help`, `--print-socket`, and
client `--print-context` exit before mode validation or socket/state/PTY work,
so scripts can combine them with broader command templates safely.

```sh
NMUX_SOCKET=/tmp/nmux-project.sock nix develop . -c cargo run --bin nmuxd -- --print-socket
NMUX_SOCKET=/tmp/nmux-project.sock nix develop . -c cargo run --bin nmux -- --print-socket
```

Connection failures include the socket path, which helps distinguish a missing daemon from an isolated test socket.
Use `nmux --connect-timeout-ms MS` when a script may start the client before `nmuxd` has finished binding the socket.
`nmuxd` refuses to replace an existing socket path, so remove stale sockets deliberately or choose a different `--socket`.
On normal bounded exits, `nmuxd` removes the socket path it created if that path still points at the same socket file.
Local PTY commands receive `NMUX=1`, `NMUX_SESSION_ID`, `NMUX_PANE_ID`,
`NMUX_SOCKET`, and `NMUX_ORIGIN` in their environment so nested tools can tell
which nmux pane and socket they are running inside. Run `nmux --print-context`
inside a pane to print those inherited `NMUX_*` key/value lines without
connecting. When `nmuxd` starts inside an nmux pane, it appends the inherited
origin to the child pane origin with `>` so nested tools can see the local hop
chain. Outside a complete nmux pane context, `--print-context` fails clearly
before socket or state work.

```sh
rm -f /tmp/nmux-context.sock
# terminal 1
nix develop . -c cargo run --bin nmuxd -- --socket /tmp/nmux-context.sock --one-shot --command 'cargo run --bin nmux -- --print-context; cat >/dev/null'
# terminal 2
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux-context.sock
```

Live attach renders the requested initial scrollback range before streaming updates, including when `--redraw` is enabled. Reconnects use `AttachStatus` as an explicit current-surface barrier and pane authority, so a client no longer waits on a timeout to learn that no surface frame follows, does not infer the attached pane from the workspace root, renders cached current surfaces only when the cached version matches `AttachStatus.surface_version`, and rejects responses whose `Snapshot`/`Patch` status disagrees with the following surface frame. When the backend reports terminal title or OSC 7 working-directory metadata, the local CLI prints those metadata lines with the current pane surface; metadata-only `CursorOnly` live updates carry no row changes and print just the changed metadata lines in non-redraw mode.
Use `nmuxd --live-clients COUNT` to keep the same local workspace alive across a bounded number of sequential live clients. Pair it with `nmux --state PATH` to reattach from a persisted client-side surface cache, including cached terminal metadata, when the daemon has no newer surface update to send. One-shot, follow, and live attach paths all reuse that scoped cached surface instead of replaying PTY bytes. Follow mode is observation-only and rejects input flags instead of silently dropping them.
Use `nmuxd --live-forever` for an unbounded sequential local workspace that survives repeated live client detach and reattach until the daemon is stopped.
State load/save failures include the state path in the error, and state saves
write a temporary file before renaming it into place.
Bounded client loops require `--iterations` greater than zero.
Bounded daemon live counts report the failing flag name when the count is not a
valid number and must be greater than zero.
Without an explicit input or resize flag, `nmux` attaches read-only; use `--key`, `--key-name`, `--paste`, `--focus`, `--mouse`, `--stdin`, or `--stdin-bytes` to opt into sending input, or live `--cols`/`--rows` to send a resize control intent. `--key-modifiers` and `--mouse-modifiers` refine their matching named-key or mouse input flag rather than selecting a separate input mode.
Explicit one-shot input is still sent when a persisted state file proves the visible surface is already current.
Scrollback ranges are 1-based from the oldest retained row and require positive `--scrollback-start` and `--scrollback-count` values. Local clients validate decoded scrollback chunks as contiguous public ranges, persist last-seen non-empty scrollback range metadata in `--state`, fetch scrollback even when the visible surface is already current, send matching cached versions as fetch preconditions, skip empty out-of-range chunks as cache keys, and retry once without a precondition if the daemon reports a stale scrollback version.
Explicit input modes such as `--key`, `--key-name`, `--paste`, `--focus`, `--mouse`, `--stdin`, `--stdin-bytes`, and `--no-input` are mutually exclusive.
Live polling intervals must be greater than zero, and explicit resize dimensions must be between 1 and 65535.

Line-streamed live input:

```sh
rm -f /tmp/nmux.sock
nix develop . -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --live --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
printf 'ping\npong\n' | nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --stdin --interval-ms 500
```

Line-streamed live clients keep polling daemon output while stdin is open and
waiting for the next complete line.

Byte-streamed live input:

```sh
rm -f /tmp/nmux.sock
nix develop . -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --live --command "printf 'ready\n'; while IFS= read -r line; do printf 'echo:%s\n' \"\$line\"; done"
printf 'ping\npong\n' | nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --stdin-bytes --interval-ms 500
```

For interactive `--stdin-bytes`, Ctrl-] detaches the client.
Add `--redraw` to repaint the workspace summary and current pane surface in place on each live update; when stdout is a TTY, redraw uses the alternate screen and restores it on exit. Interactive byte mode uses noncanonical stdin, defaults local echo off, can preserve the TTY echo setting with `--local-echo tty`, and sends TTY-size resize intents on `SIGWINCH` unless explicit `--cols` and `--rows` are provided. Explicit `--cols` and `--rows` are user-command resize intents, so they can commit a resize without sending pane input even when the daemon publishes `resize=manual`.
Live-only frontend flags such as `--stdin`, `--stdin-bytes`, `--redraw`, and `--cols`/`--rows` are rejected unless `--live` is set.

Resize policy smoke:

```sh
rm -f /tmp/nmux.sock
nix develop . -c cargo run --bin nmuxd -- --socket /tmp/nmux.sock --live-cycles 1 --resize-policy active-client --command "printf 'ready\n'; sleep 1"
nix develop . -c cargo run --bin nmux -- --socket /tmp/nmux.sock --live --no-input --iterations 1
```

Expected output includes `resize=active-client`.

This is still a prototype. It has an interactive byte-streamed live path, but the default temporary text surface is not a VT-correct terminal emulator; ANSI styling, cursor motion, alternate screen, images, and grapheme/cell-width correctness are not complete in the default engine. Backend `libghostty-vt` extraction is available as an experimental opt-in engine for correctness work. The opt-in engine now covers VT byte ingestion, cursor updates including blink state with decoded cursor enum validation, alternate-screen detection/restoration with alternate scrollback omission and structured main scrollback preservation, resize/reflow, backend-owned styled scrollback, style-separated row runs including style-bearing and semantic-content trailing blanks, row semantic prompt metadata, per-run semantic content with decoded patch/cache enum validation, cell widths, combining marks, emoji ZWJ clusters, SGR style flags and underline variants, underline color, palette-indexed SGR colors, terminal color state, render-state default colors/palette, palette overrides, explicit cursor color, color-only surface patches with scoped palette diffs and mixed no-row color/mode cache refresh, terminal title metadata, OSC 7 working-directory metadata extraction, metadata-only `CursorOnly` no-row patches with persisted state reattach coverage, OSC 133 row semantic prompt state, row-level dirty state, row state hashes, row-run-aware no-row patch classification, sparse row updates, Kitty graphics placeholder metadata, hyperlink presence on row runs and explicit full-object hyperlink tables without wired hyperlink IDs yet, terminal-generated PTY reply routing for DECRQM query responses, bracketed paste, paste safety validation, PasteInput forwarding with daemon-owned delimiter selection, detailed mouse tracking mode/format state with decoded mouse/pane-tree/control-plane enum validation, decoded attach/presence/input payload, workspace/surface/status/scrollback ID, error message/pane ID, pane-scoped ID validation, recovery-marker row-payload rejection, 1-based scrollback range validation, and pane-bounds/mode-gated MouseInput forwarding with cell or SGR-pixel coordinates and validated modifiers, focus reporting and daemon-gated FocusInput forwarding with protocol-visible disabled-mode errors, common named-key forwarding with validated modifiers, application keypad tracking and mode-aware keypad Enter/digit forwarding, application cursor tracking and mode-aware arrow-key forwarding, public named-key modifier syntax, engine-backed key encoding with modifier preservation, explicit encoder output, Error frames for structured-input encoding failures, host input/resize/output-poll failures, unknown pane-scoped intents and missing active-tab/active-pane metadata, and stale scrollback version preconditions with structured pane/input attribution, origin, and wraparound modes, mode-aware key encoding, and mode-only surface patches. CLI server-error output includes structured protocol attribution such as error code, pane ID, retryable flag, and input sequence when present. Feature-gated live CLI tests cover command streaming, committed user-command resize metadata, reattach behavior including cursor-only, mode-only, color-only, styled/wide run, and `ReplaceRows` row metadata/hyperlink state persistence, current-surface bracketed paste, SGR mouse including SGR-pixel coordinates, application-keypad named-key, and application-cursor named-key forwarding, metadata-only updates and state reattach, style-table full-refresh recovery, terminal query reply routing through a real PTY command, monotonic post-attach envelope/input sequencing across mixed live-style frames, and restored alternate-screen scrollback omission through `nmuxd --terminal-engine libghostty-vt`. Image placement data, wired hyperlink IDs, and broader shell command metadata still wait for matching nmux protocol objects.
The core now routes pane output through a daemon-owned terminal engine boundary so that interim behavior can be replaced without changing client-side state-sync semantics.
`nmuxd --terminal-engine interim` makes the default engine explicit. `nmuxd --terminal-engine libghostty-vt` is available only in `--features libghostty-vt` builds; it is covered by full opt-in feature test suites, but not yet the default because it pulls in the native Ghostty/Zig build path.
ADR 0023 closes M13 as the opt-in extraction milestone and keeps `libghostty-vt` out of the default path until native build time, regular CI, non-Nix toolchain provisioning, source-fetch policy, packaging, and developer workflow costs are accepted deliberately. Track that evidence in [docs/default-engine-promotion.md](docs/default-engine-promotion.md). Frontend Ghostty renderer hydration and richer protocol objects stay separate tracks until their API and compatibility shapes are explicit.
