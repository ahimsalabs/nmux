**nmux is a portable Ghostty-style workspace whose backend owns terminal state, and libghostty is the canonical terminal-state/snapshot engine.**

Garden note: this file mixes product thesis, aspirational protocol sketches, and
implemented milestone status. Treat `docs/roadmap.md`, `docs/protocol.md`, and
ADRs as authoritative for current schema and implementation details.

That means nmux should not primarily synchronize raw PTY bytes. It should synchronize **versioned terminal state objects**: session tree, tab tree, pane grid, cursor, scrollback ranges, titles, agent state, presence, and input/control events. That matches the direction in your prior notes: `session -> tab -> pane`, resumable connections, multi-player presence, sandbox-hosted PTYs, and a protocol boundary rather than a herdr-shaped core. 

## The thesis

`tmux -CC : iTerm2 :: nmux : libghostty`

But unlike tmux control mode, nmux should not be a retrofit. iTerm2’s tmux integration lets tmux windows appear as native iTerm2 windows/tabs and lets native menu commands operate on tmux windows. ([iTerm2][1]) nmux’s version should be designed from day one as:

**terminal engine + mux daemon + state-sync protocol + many clients**

Ghostty is a strong substrate because its own architecture already separates GUI apps from `libghostty`: the docs describe `libghostty` as a cross-platform C-ABI-compatible core providing terminal emulation, font handling, and rendering, with the macOS and Linux GUI apps consuming that library. ([Ghostty][2]) The repository also describes `libghostty` as usable for building or embedding terminal functionality, while noting the broader libghostty API/docs/versioning are still evolving. ([GitHub][3]) Ghostty is MIT-licensed, which fits your MIT/Apache preference for the core. ([GitHub][3])

## Architecture

Think of nmux as four layers:

```text
frontends
  ghostty/libghostty native app
  forked Ghostty UI
  web client
  mobile client
  maybe cmux-compatible bridge

state-sync protocol
  FlatBuffers envelope
  snapshots + patches
  input/control streams
  presence
  permissions
  scrollback range fetches

nmuxd
  session/tab/pane model
  process hosts
  sandbox/PTY lifecycle
  authoritative terminal state
  libghostty snapshot extraction

backends/adapters
  native local PTY backend
  sandbox/container backend
  tmux backend
  herdr adapter, optional AGPL membrane
```

The target move is that **nmuxd runs a real Ghostty/libghostty-derived terminal
engine per pane**. Today that is implemented as an opt-in `libghostty-vt`
backend while the default engine remains the interim text surface. PTY bytes go
into the daemon-owned terminal engine, that engine maintains authoritative
terminal state, and nmux serializes that state into snapshots and diffs.
Clients render the state and send input back. This directly uses the thing you
actually wanted from libghostty: snapshotting, VT correctness, Unicode, styles,
modes, images, scrollback, and eventually render integration. Your previous
notes already called out the snapshot/VT layer as the central place Ghostty
belongs, not as a backend adapter.

## Mosh inspiration, but not a Mosh clone

Mosh’s big idea is not just UDP; it is **state synchronization instead of byte-stream replay**. The paper says Mosh synchronizes client/server terminal state, supports intermittent connectivity and roaming, and uses a server-side terminal emulator to synchronize screen states rather than shipping an octet stream like SSH. 

nmux should steal that model:

```text
client: I have pane p version 1042, scrollback ranges 1..200 and 901..1100
server: pane p current version is 1088
server: here are patches 1043..1088
```

or, if the client is too stale:

```text
server: patches expired
server: here is a full snapshot at version 1088
```

Mosh intentionally optimizes for the current visible screen and notes that scrollback history is problematic in that model.  nmux should not inherit that limitation. Treat scrollback as a separate synchronized object:

```text
PaneSurface        current visible/alternate screen
PaneScrollback     durable append-only-ish line store
PaneViewport       client-specific visible range
PanePatch          small current-screen update
ScrollbackChunk    lazy historical range
```

That is what makes it a **portable workspace**, not just a remote shell.

## What “Ghostty control mode” looks like

The rest of this section is a product/protocol sketch, not the current
FlatBuffers contract. Keep the implemented schema details in `docs/protocol.md`
and durable protocol decisions in ADRs.

The Ghostty equivalent of `tmux -CC` is not a line protocol that Ghostty parses. It is more like:

```bash
ghostty --workspace nmux://host/session-id
```

or:

```bash
nmux attach --frontend ghostty session-id
```

The frontend handshake would say:

```text
Hello
  client_id
  user_id
  frontend = ghostty | web | mobile | tui
  capabilities:
    cell_rendering
    ligatures
    images
    sixel/kitty-graphics
    local_echo
    clipboard
    hyperlinks
    truecolor
    font_metrics
    max_patch_rate
```

Then:

```text
AttachWorkspace
  session_id
  known_object_versions:
    tree = 71
    pane:abc.surface = 1042
    pane:abc.scrollback = 980
```

Then the server replies with either patches or fresh snapshots. The implemented
local attach path currently sends `WorkspaceTreeSnapshot`, `PresenceUpdate`,
`AttachStatus`, and then a pane surface frame only when the status says one
follows. Future frontend handshakes may grow beyond that:

```text
WorkspaceTreeSnapshot
PresenceUpdate
AttachStatus
PaneSurfaceSnapshot
PaneSurfacePatch
ScrollbackChunk
future PresenceSnapshot
future AgentStateSnapshot
```

Input goes the other direction:

```text
InputEvent
  pane_id
  actor_id
  monotonic_input_seq
  key/mouse/paste/resize-intent
```

So `tmux -CC for Ghostty` becomes:

**Ghostty/libghostty as a native renderer and terminal UX shell for a backend-owned, versioned terminal-state graph.**

## libghostty’s exact role

There are two distinct libghostty uses.

First, **backend libghostty**: this is the important one. `nmuxd` feeds PTY bytes into libghostty and extracts an nmux snapshot model. Mitchell Hashimoto’s libghostty roadmap specifically calls out `libghostty-vt` as a minimal dependency terminal-sequence parser that maintains terminal state such as cursor position, styles, wrapping, and more, with longer-term libraries for input handling, GPU rendering, Swift frameworks, GTK widgets, and related pieces. ([Mitchell Hashimoto][4])

Second, **frontend libghostty**: this is the nice-to-have or later deep integration. A native frontend could use libghostty for rendering, font shaping, theme parsing, keyboard encoding, and maybe terminal widgets. But you do not want the frontend to re-parse raw PTY bytes as the source of truth. That would reintroduce divergence across clients. The frontend should render the authoritative server state.

That means the hard API question is:

```text
Can libghostty render externally supplied terminal state?
```

If yes, great: nmux snapshots can hydrate a libghostty render state. If no, you either need a small custom renderer for v0, or a Ghostty fork/API contribution that adds “render this external grid/surface” support. The prior “backend emulates, Ghostty renderer sits idle” concern was too pessimistic; the better statement is: **backend libghostty is mandatory, frontend libghostty is an integration milestone.** Your uploaded notes already converged on the Mosh-style “state object” protocol and flagged that raw-output RPC was the wrong shape. 

## FlatBuffers contract

FlatBuffers is still the right serialization layer, but the schema should be state-sync-first, not RPC-first. FlatBuffers supports zero-copy access without parsing/unpacking and works across languages including Rust, Swift, and TypeScript. ([GitHub][5]) Its evolution rules also fit this project: append fields, do not remove fields, deprecate instead. ([FlatBuffers][6])

A good current `nmux.fbs` spine is:

```text
Envelope
  protocol_version
  session_id
  connection_id
  seq
  ack
  body: union

Bodies
  WorkspaceTreeSnapshot
  PaneSurfaceSnapshot
  PaneSurfacePatch
  ScrollbackFetch
  ScrollbackChunk
  InputEvent
  ResizeIntent
  PresenceUpdate
  AttachRequest
  AttachStatus
  Error
```

Do **not** start with a naïve `Cell { codepoint:u32, attrs:u64 }` and freeze it. Terminal cells are messier than that: grapheme clusters, double-width characters, combining marks, hyperlinks, images, underlines, cursor modes, palette changes, and ligatures all matter. The protocol now encodes rows as runs:

```text
CellRun
  text_utf8
  cell_widths
  style_id
  hyperlink_id
  flags

Row
  runs
  dirty_hash
```

Then keep a separate style table. Current wire colors are packed RGBA values
and style booleans are compacted into `Style.flags`; this sketch remains useful
as the conceptual inventory behind those packed fields:

```text
Style
  fg
  bg
  underline_color
  bold
  italic
  faint
  blink
  inverse
  invisible
  overline
  underline_kind
  strikethrough
```

That lets snapshots be compact and patches be row/range-based.

## Resize policy is a first-class problem

Multi-client terminal muxing has a subtle issue: a PTY has one size, but your clients may be a MacBook, iPad, phone, browser split, and TUI. nmux should not let every client resize the PTY whenever its window changes.

Make pane size authoritative:

```text
PaneSize
  cols
  rows
  policy = fixed | active_client | leader | manual
```

Clients send:

```text
ResizeIntent { pane_id, desired_cols, desired_rows, reason }
```

The server publishes committed size through workspace/pane state.

Spectators can have smaller viewports. Controllers can request resize. A “leader” client can own size. This prevents mobile reconnects from trashing everyone else’s layout.

## cmux taxonomy

cmux is not really a backend adapter. It is prior art / a peer frontend shape: a native macOS app built on Ghostty with vertical tabs, notifications, split panes, a socket API, and automation. ([cmux][7]) The GitHub README says it uses libghostty for terminal rendering, reads Ghostty config, and exposes CLI/socket automation. ([GitHub][8]) It is GPL-3.0-or-later, so treat it like inspiration or an integration target, not code to pull into an MIT/Apache core. ([GitHub][8])

So the taxonomy becomes:

```text
frontends
  nmux-ghostty
  nmux-web
  nmux-mobile
  maybe cmux bridge / cmux-compatible mode

backends
  nmux-native
  nmux-tmux
  nmux-herdr

terminal engine
  libghostty
```

Ghostty is not a backend adapter either. It is the terminal engine and likely the flagship native frontend substrate.

## Plugins and scriptability

Avoid an in-process plugin system at first. The nmux protocol is the plugin system.

A scriptable CLI can just speak nmux:

```bash
nmux session create
nmux tab new
nmux pane split --right
nmux pane send --pane p123 'cargo test\n'
nmux pane snapshot --json
nmux agent mark --pane p123 --state waiting
```

Adapters are sidecars:

```text
nmuxd <-> nmux-tmux-adapter
nmuxd <-> nmux-herdr-adapter
nmuxd <-> nmux-sandbox-host
```

That keeps licensing and failure isolation clean. Your prior notes already had the right instinct: adapters as separate processes, with herdr quarantined behind its own AGPL membrane and the MIT/Apache core speaking only the nmux protocol. 

## Transport

Use the same FlatBuffer envelopes over multiple transports:

```text
local:   Unix domain socket
native:  QUIC
browser: WebSocket first, WebTransport later
debug:   JSON framing
```

For mobile-style roaming, QUIC is the natural native transport because RFC 9000 defines connection migration via connection identifiers, allowing a QUIC connection to move to a new network path and survive address/topology changes such as NAT rebinding. ([IETF Datatracker][9]) The quic-go docs describe the exact mobile case: moving from Wi‑Fi to cellular while keeping the application connection alive. ([quic-go][10])

But the protocol should not depend on QUIC. State sync is the real win. QUIC helps the connection survive; nmux snapshots help the workspace survive even when the connection does not.

### Identity and auth

Three transport paths, each with its own identity source. See [ADR 0014](docs/adr/0014-transport-identity-boundary.md) for the boundary decision.

```text
local:      Unix peer credentials (uid/gid)
tailscale:  WireGuard encryption + tailscaled WhoIs API + ephemeral session token
non-ts:     SSH bootstrap → short-lived signed token → QUIC+TLS direct connection
```

On Tailscale, WireGuard provides encryption and peer authentication at the network layer. `nmuxd` calls `GET /localapi/v0/whois?addr=IP:port` on the tailscaled Unix socket to get the peer's `UserProfile.LoginName`, `Node.Name`, and `Node.StableID`. An ephemeral session token adds per-session verification so that WhoIs is not the sole identity gate (guards against local-process spoofing on a compromised node). No application-layer TLS needed on a tailnet.

Without Tailscale, the mosh bootstrap model applies: `nmux connect host` SSHs to the remote, starts or finds `nmuxd`, exchanges a short-lived signed session token, then opens a direct QUIC+TLS connection with that token. SSH handles authentication; the token authorizes the direct connection. The SSH session ends after bootstrap.

All three paths resolve to the same `Actor` for the nmux protocol: `actor_id`, `user_id`, `display_name`, `mode`. The FlatBuffers layer does not know which transport authenticated the peer.

### Proxy composition and chaining

A proxy daemon can aggregate multiple upstream `nmuxd` instances into one workspace tree. See [ADR 0014](docs/adr/0014-transport-identity-boundary.md) for design rationale.

```text
client → nmux-proxy → nmuxd@machine-a (local PTY)
                     → nmuxd@machine-b (tmux adapter)
                     → nmuxd@machine-c (herdr adapter)
```

The proxy assembles a synthetic workspace tree, namespaces pane IDs by upstream origin, routes input/scrollback to the owning upstream, and forwards surface state downstream. FlatBuffers pane state can be forwarded as raw bytes without deserialize/reserialize. Per-pane version streams are independent across upstreams.

Chaining works because each proxy is just another nmux speaker. `nmuxd` sets `NMUX_*` environment variables in spawned PTYs; a client running inside an nmux pane reads those vars and sends chain/origin metadata in its `AttachRequest`. Each hop appends to the origin chain automatically.

### Zero-overhead local path

When the terminal emulator (Ghostty) embeds `nmuxd` in-process, the local render path reads VT state directly from the in-process `libghostty-vt` instance -- no FlatBuffers serialization, no IPC. The FlatBuffers protocol only activates when a remote client connects. This dual-path model (in-process fast path + serialized remote path) is the same pattern used by Chrome DevTools / V8 inspector and gRPC in-process optimization. Consistency between paths should be enforced by invariant tests that compare serialized snapshots against direct state reads.

## The first build target

Do not start with a beautiful Ghostty fork. Start with a local state-sync spine that can be used and tested end to end, then swap in libghostty-backed terminal state when that boundary is ready.

Backend `libghostty-vt` is not indefinitely deferred. It follows the usable local state-sync spine so extraction can be validated against real nmux snapshots, patches, scrollback ranges, reconnect behavior, and live attach workflows. Frontend Ghostty rendering remains a separate integration milestone because it depends on whether Ghostty/libghostty can hydrate a renderer from nmux-owned state without replaying raw PTY bytes on the client.

Current implementation status:

```text
M0: nmux.fbs [done]
  Envelope
  WorkspaceTreeSnapshot
  PaneSurfaceSnapshot
  PaneSurfacePatch
  InputEvent
  ResizeIntent

M1: nmuxd local [done for local skeleton]
  spawn local PTY
  feed bytes into interim backend-owned text surface
  serve visible surface snapshots/patches
  accept input over local Unix socket
  fixed pane size

M2: dumb viewer [done for CLI text renderer]
  terminal text renderer
  attach
  receive snapshots
  send keys

M3: reconnect [done]
  client drops
  reconnects with known versions
  server sends patch or full snapshot

M4: scrollback object [done]
  lazy range fetch
  snapshot + scrollback consistency tests

M5: multi-player [done for sequential local clients]
  presence
  actor IDs
  read-only vs read-write attach

M6: sandbox host [done for host abstraction and local PTY host]
  local/container/sandbox process host abstraction

M7: Ghostty frontend [done as boundary decision and upstream API tracker]
  either embed libghostty renderer if state injection is possible
  or fork/contribute API for external surface rendering

M8: tmux adapter [done for process boundary and pure mapping test]

M9: herdr adapter in separate AGPL repo [done as boundary decision]

M10: live local interactive attach [done]
  one attached local connection can stream repeated input/output cycles
  read-only live clients can observe output without forwarding input until daemon close
  read-only live resize intents return protocol PermissionDenied instead of being silently ignored
  live CLI can stream stdin lines until EOF without default key fallback
  live CLI can send stdin byte chunks as InputKind.RawBytes without blocking output polling on full lines
  interactive --stdin-bytes temporarily uses noncanonical stdin with local echo defaulting off and `--local-echo tty` available
  interactive --stdin-bytes listens for SIGWINCH and sends TTY-size resize intents unless explicit --cols/--rows are set
  Ctrl-] detaches byte-streamed live clients
  --redraw clears and repaints the current client-side pane surface on each update
  CLI workspace summary displays daemon-published resize policy
  successful live resize intents commit pane size and republish workspace snapshot
  nmuxd --resize-policy can publish fixed, leader, active-client, or manual policy
  manual resize policy blocks frontend viewport resize intents
  nmux and nmuxd --help document live attach, stdin, redraw, resize, and policy flags
  nmuxd --live serves one live client until detach
  live updates render through client-side pane surface state

M11: terminal frontend polish [done]
  live and redraw output is explicitly flushed after render updates
  Ctrl-] detach reports a local detach status on stderr
  stdin EOF and live server socket close exits report status on stderr
  explicit live resize requests are sent as user-command resize intents so daemon-published manual policy can accept them
  resize-only live clients can commit explicit ResizeIntent without sending pane input
  --live --no-input rejects explicit --cols/--rows before connecting because resize control requires a writable actor
  nmux --help and interactive TTY byte mode call out interim text surface / non-VT-correct renderer limitations
  live-only frontend flags fail fast outside --live instead of being silently ignored
  live attach renders requested initial scrollback context before streaming updates, including initial --redraw paint
  interactive TTY --redraw uses alternate screen and restores it on exit
  redraw mode keeps current workspace summary visible and updates it on live workspace snapshots
  runnable docs and help output cover expected live attach workflows
  tests cover changed user-visible terminal output behavior

M12: live workspace usability [done]
  nmuxd --live-clients COUNT keeps one workspace and PTY alive across bounded sequential live clients
  nmuxd live mode flags fail fast on ambiguous server modes and zero live counts
  nmux --state renders scoped cached current surfaces on one-shot, follow, and live reattach instead of waiting for raw replay
  explicit one-shot --key/--paste input is forwarded even when the client already has the current visible surface
  explicit one-shot --key-name/--focus/--mouse input is forwarded before scrollback fetch using daemon-owned input gates
  nmux client bounded live/follow iterations fail fast on zero counts
  nmux --follow rejects input flags instead of silently dropping explicit input
  nmux attaches read-only by default unless an explicit input flag or live resize control flag is provided
  explicit nmux input modes fail fast on conflicting --key/--key-name/--paste/--focus/--mouse/--stdin/--stdin-bytes/--no-input combinations
  nmux live loop timing and explicit resize dimensions fail fast on zero or out-of-range values
  nmux --state load/save failures include the state path before socket connection work
  nmuxd and nmux share a stable default socket path for local workflows without --socket
  help output and quick-start docs show default-socket live workflows first
  help output documents the shared default socket path
  nmux connection failures include the socket path
  nmuxd --live-forever serves sequential live clients until the daemon is stopped
  nmuxd refuses to replace an existing socket path and reports the path
  nmuxd removes its socket path on normal bounded exits
  nmux scrollback range flags fail fast on zero start/count values
  default socket selection falls back when XDG_RUNTIME_DIR is empty or relative
  nmuxd normal-exit cleanup only removes the original socket file if unchanged
  existing socket path errors include a recovery hint
  nmuxd bind failures include the socket path
  nmux --connect-timeout-ms waits for daemon socket startup races across attach modes
  one-shot and live post-attach input, resize, and scrollback control frames use the attached active pane ID instead of assuming pane-1
  preserve the backend-owned state-sync model rather than adding raw PTY replay shortcuts
  keep interim renderer limitations explicit until libghostty-backed state/render integration is available
  keep runnable docs, help output, and tests aligned with each user-visible behavior change

M13: backend libghostty-vt extraction [current correctness milestone]
  terminal engine boundary wraps current interim text surface behavior
  terminal engine boundary owns pane cursor state along with surface and scrollback output
  local daemon serving paths keep terminal engines alive per pane across output polls, resize handling, and sequential clients
  nmuxd exposes --terminal-engine interim as an explicit default while libghostty-vt remains opt-in
  keep the terminal-state extraction checklist current while importing libghostty-vt and before expanding protocol fields
  optional libghostty-vt feature feeds PTY bytes into daemon-owned VT state, not clients
  optional libghostty-vt engine maps cursor, surface kind, terminal modes, visible rows, styled scrollback rows, row runs, style IDs, cell widths, resize/reflow, and backend-owned scrollback into nmux objects
  libghostty-vt extraction preserves style-bearing and semantic-content trailing blank cells while still trimming plain default trailing blanks
  optional libghostty-vt coverage includes cursor visibility/shape/blink state, alternate-screen entry/restoration with alternate scrollback omission and structured main scrollback preservation, combining marks, emoji ZWJ clusters, SGR style flags and underline variants, underline color, palette-indexed SGR color resolution, terminal color state, render-state default colors/palette, palette overrides, explicit cursor color, title metadata, OSC 7 working-directory metadata extraction, OSC 133 row semantic prompt state, OSC 133 per-run semantic content, row-level dirty state, Kitty graphics placeholder metadata, hyperlink presence on row runs, bracketed paste, paste safety validation, local PasteInput forwarding with daemon-owned delimiter selection, detailed mouse tracking mode/format state and pane-bounds/mode-gated MouseInput forwarding with validated low-four-bit modifiers, focus reporting and daemon-gated FocusInput forwarding with disabled-mode Error frames, common named-key forwarding, application keypad tracking and mode-aware keypad Enter/digit forwarding, application cursor tracking and mode-aware arrow-key forwarding, public named-key modifier syntax, engine-backed named-key encoding with modifier preservation, explicit encoder output, origin, and wraparound modes, and mode-aware key encoding
  MouseInput carries optional pixel coordinates for SgrPixels mouse mode while keeping zero-based cell coordinates for pane-bounds gating and fallback encoding
  OSC 8 hyperlink text and backend row/cell hyperlink presence are preserved through CellRun flags, feature-gated local and live CLI coverage proves libghostty-vt OSC 8 run flags survive ReplaceRows cache updates, state encode/decode, and real --state reattach, and hyperlink IDs remain unset until backend URI identity is wired into nmux's hyperlink table
  current libghostty-vt bindings expose hyperlink presence but not structured per-cell identity plus URI/id/params lookup, so docs/upstream/libghostty-vt-hyperlink-identity.md tracks the upstream/API gap instead of duplicating OSC 8 terminal state in nmux
  CursorState carries cursor blink metadata from libghostty-vt, and old cached client cursor state defaults to blinking enabled
  PaneSurfaceSnapshot and PaneSurfacePatch carry terminal title and OSC 7 working-directory metadata
  title/OSC 7-only updates use CursorOnly no-row surface patches, and feature-gated live CLI coverage proves non-redraw clients print metadata-only changes without reprinting unchanged row text and persist those metadata-only changes through --state reattach
  local CLI rendering prints non-empty terminal title and OSC 7 working-directory metadata with the current pane surface
  feature-gated local and live CLI coverage proves libghostty-vt CursorOnly patches stream after attach, avoid row repaint, update ClientAttachState cursor metadata, and survive state encode/decode plus real --state reattach
  libghostty-vt terminal-generated PTY writes are exposed through the terminal engine and host-backed output polling writes DECRQM query replies back to the pane process instead of dropping emulator responses; feature-gated live CLI coverage proves a real PTY command can read the reply
  structured-input encoding failures return protocol Error frames instead of opaque daemon exits, and local one-shot clients surface the server-provided reason before scrollback fetches
  one-shot and live host write failures return protocol Error frames instead of tearing down nmuxd with opaque I/O failures
  live host resize failures return protocol Error frames instead of tearing down nmuxd with opaque I/O failures
  initial and post-attach host output polling failures return protocol Error frames instead of tearing down nmuxd with opaque I/O failures
  pane-scoped one-shot and live client intents for unknown panes return protocol PaneNotFound errors instead of hanging or falling through to process-host behavior, decoded input/resize/scrollback-fetch intents reject missing or empty pane/actor IDs before they can target an accidental empty pane, decoded attach requests and presence updates reject missing or empty actor/user/display identity and focused/known-surface pane metadata instead of substituting local defaults, decoded workspace/surface/attach-status/scrollback state rejects missing or empty session/tab/pane IDs instead of creating empty client cache keys, and attach setup rejects missing active-tab or active-pane metadata instead of guessing pane-1
  public scrollback ranges consistently use 1-based line numbers from ScrollbackFetch through ScrollbackChunk and decoded client summaries, and decoded clients reject zero start/count or row line values instead of normalizing them to the oldest row
  ScrollbackFetch known_scrollback_version 0 means no client precondition, and nonzero stale scrollback versions return protocol StaleVersion errors
  nmux --state preserves distinct last-seen scrollback range/version metadata entries scoped to the daemon socket identity, current-surface reconnects still fetch daemon-owned scrollback, matching later fetches use cached versions as preconditions, and stale fetches retry once with no version precondition
  SurfaceRow, RowUpdate, and ScrollbackRow carry OSC 133 row semantic prompt metadata; CellRun carries OSC 133 output/input/prompt semantic content; decoded surfaces, patches, scrollback, and cached patch application reject unknown semantic enum values; broader semantic command IDs, ranges, lifecycle, and exit metadata remain withheld
  PaneSurfaceSnapshot, PaneSurfacePatch, and ScrollbackChunk carry TerminalColorState; color-only updates use PatchKind::ColorOnly, feature-gated local and live CLI coverage proves ColorOnly patches update cached client colors from palette diffs, reject invalid palette diffs without partial cache mutation, refresh their serialized mode payloads for mixed no-row color/mode changes without row repaint, and survive state encode/decode plus real --state reattach, while palette overrides that alter existing row style-table entries force full refreshes
  decoded and cached clients reject unknown SurfaceKind, PatchKind, cursor-shape, mouse-mode, and mouse-format values, decoded control-plane frames reject unknown pane-tree/attach/presence/status/resize/error enum values and missing or empty error messages/pane IDs, decoded input rejects unsupported modifier bits and missing InputKind payload tables, and no-row patch kinds plus FullRefreshRequired recovery markers reject row payloads so version advancement cannot silently discard unexpected row changes
  PaneSurfaceSnapshot and ScrollbackChunk carry an explicit Hyperlink table and cached client state can persist table entries, while libghostty-vt still leaves CellRun.hyperlink_id unset until backend URI identity is wired
  ReplaceRows patches now carry only changed rows when the pane geometry is stable; libghostty-vt no-row patch classification compares structured row runs as well as fallback text so cursor/mode updates cannot hide row-run metadata changes; feature-gated local and live CLI coverage proves ReplaceRows patches update cached row semantic content, dirty state, and row_state_hash metadata through state encode/decode and real --state reattach; client caches reject duplicate or out-of-bounds row updates without partial mutation; SurfaceRow, RowUpdate, and ScrollbackRow carry backend row dirty flags and row state hashes as metadata, while richer run/region damage protocol fields remain withheld
  client-side scrollback decoding preserves ScrollbackRow dirty_hash and row_state_hash metadata instead of reducing scrollback rows to text and runs only
  SurfaceRow, RowUpdate, and ScrollbackRow carry Kitty virtual placeholder metadata; image placement and pixel-data protocol fields remain withheld
  PaneSurfaceSnapshot, PaneSurfacePatch, and ScrollbackChunk preserve row runs instead of collapsing state to text-only rows
  client-side tests prove decoded surface patches and scrollback chunks preserve structured CellRun style IDs, cell widths, hyperlink-presence flags, and semantic content instead of collapsing to rendered fallback text, and decoded snapshots/scrollback reject row runs that reference missing style or hyperlink table entries
  feature-gated live CLI coverage proves styled row runs, non-default style-table entries, and wide-cell width metadata survive real --state reattach
  feature-gated live CLI coverage proves libghostty-vt streams command output and committed user-command resize metadata through nmuxd without leaking raw ANSI controls
  feature-gated live CLI coverage proves restored libghostty-vt alternate-screen output stays out of requested scrollback
  PaneSurfaceSnapshot and PaneSurfacePatch carry terminal mode state, and mode-only updates no longer force full refreshes
  feature-gated local and live CLI coverage proves libghostty-vt ModeOnly patches stream after attach, avoid row repaint, update ClientAttachState modes, and survive state encode/decode plus real --state reattach
  ADR 0017 documents terminal input mode state and daemon-owned input gating for paste, focus, named keys, and mouse input
  style-table changes force a full surface snapshot, while row-run-only changes can still use PaneSurfacePatch
  live attach clients with a known surface version receive a PaneSurfaceSnapshot, not a PaneSurfacePatch, when the daemon marks the latest surface update FullRefreshRequired
  feature-gated live CLI coverage proves libghostty-vt reattach recovers from style-table FullRefreshRequired updates without raw ANSI leakage
  ScrollbackChunk carries the pane style table so scrollback row runs do not reference missing style IDs
  nmux --state preserves cached title, OSC 7 working directory, terminal modes including mouse tracking mode/format, row runs, style tables, terminal color state, OSC 133 row/run semantic metadata, row dirty flags, row state hashes, Kitty placeholder row metadata, and distinct last-seen scrollback range metadata for patchable reconnects, validates cached terminal enum values and row run style references on load, and scopes caches to the daemon socket identity so recreated socket paths force a fresh snapshot
  current-version reattach sends explicit paste input before scrollback fetch and keeps bracketed-paste delimiter selection daemon-owned
  current-version live reattach sends explicit focus input to the daemon even when no surface frame arrives, and CLI integration coverage proves focus-reporting-disabled cases produce daemon-owned Error frames
  current-version live reattach covers key, paste, named-key, focus, and mouse input when no surface frame arrives, including CLI paste forwarding, disabled-mode rejection, malformed mouse enum rejection, current-surface CLI libghostty-vt bracketed-paste wrapping, current-surface CLI libghostty-vt SGR mouse forwarding, live CLI libghostty-vt SGR-pixel mouse forwarding, and mode-aware CLI libghostty-vt application-keypad and application-cursor named-key forwarding
  AttachStatus makes current-surface attach explicit, removes timeout-based no-surface detection, and lets post-attach scrollback preconditions use the attached pane ID
  Error frames carry structured pane attribution and input sequence attribution for pane-scoped/input failures instead of requiring clients to parse reason strings
  focused local coverage proves clients maintain monotonic Envelope.seq and InputEvent.input_seq values across one-shot and live-style post-attach frames, including repeated structured input separated by scrollback fetches and resize intents
  document cursor, mode, alternate-screen, palette, hyperlink, image, grapheme, and cell-width gaps before schema changes
  preserve frontend state-sync semantics; do not introduce client-side raw PTY replay
  keep frontend Ghostty renderer hydration as a separate upstream/API question
  do not copy code from GPL or AGPL terminal parsers

M14: post-M13 promotion and product split [planned]
  decide whether libghostty-vt becomes the documented default and regular CI path, or keep it opt-in with an ADR that names concrete native build, packaging, and developer-workflow blockers
  keep frontend Ghostty renderer hydration separate from backend terminal-state extraction until upstream can render externally supplied nmux state without client-side PTY replay
  split future protocol expansion into explicit tracks before schema changes: wired hyperlink IDs, image placement/pixel data, richer damage objects, semantic command lifecycle metadata, and physical-key/text-event forwarding
  keep local attach, reconnect, live streaming, scrollback fetch, cached state, and daemon-owned structured-input behavior stable while the default-engine decision is made
  use docs/upstream trackers for upstream-blocked work instead of treating it as local implementation debt
```

The crisp product phrase is:

**nmux: a Mosh-inspired, Ghostty-powered, FlatBuffers-native terminal workspace protocol.**

Or more directly:

**portable Ghostty workspaces, synchronized across clients.**

[1]: https://iterm2.com/3.3/documentation-highlights.html "Highlights for New Users - Documentation - iTerm2 - macOS Terminal Replacement"
[2]: https://ghostty.org/docs/about "About Ghostty"
[3]: https://github.com/ghostty-org/ghostty "GitHub - ghostty-org/ghostty:  Ghostty is a fast, feature-rich, and cross-platform terminal emulator that uses platform-native UI and GPU acceleration. · GitHub"
[4]: https://mitchellh.com/writing/libghostty-is-coming "Libghostty Is Coming – Mitchell Hashimoto"
[5]: https://github.com/google/flatbuffers "GitHub - google/flatbuffers: FlatBuffers: Memory Efficient Serialization Library · GitHub"
[6]: https://flatbuffers.dev/evolution/ "Evolution - FlatBuffers Docs"
[7]: https://cmux.com/ "cmux — The terminal built for multitasking"
[8]: https://github.com/manaflow-ai/cmux "GitHub - manaflow-ai/cmux: Ghostty-based macOS terminal with vertical tabs and notifications for AI coding agents · GitHub"
[9]: https://datatracker.ietf.org/doc/html/rfc9000 "
            
                RFC 9000 - QUIC: A UDP-Based Multiplexed and Secure Transport
            
        "
[10]: https://quic-go.net/docs/quic/connection-migration/ "Connection Migration – quic-go docs"
