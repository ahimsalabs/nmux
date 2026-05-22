# Implementation Roadmap

This roadmap promotes the build targets from [WORK.md](../WORK.md) into a tracked implementation sequence. Keep it current as commits land.

## Current Target

M13 is the current target: replace the interim backend text surface with a daemon-owned terminal engine path, starting with the terminal engine boundary and then moving toward backend `libghostty-vt` extraction.

Done:

- Initial architecture notes in [WORK.md](../WORK.md).
- ADR 0001 for backend-owned terminal state.
- ADR 0002 for Rust as the core runtime.
- Initial M0 schema in [schema/nmux.fbs](../schema/nmux.fbs).
- M0 protocol notes in [docs/protocol.md](protocol.md).
- Reproducible schema validation through `nix develop path:$PWD -c make check-schema`.
- Rust workspace and generated protocol crate for core implementation.
- Bounded FlatBuffers envelope framing.
- Initial Rust session model that encodes a `WorkspaceTreeSnapshot`.
- Local `nmuxd` and `nmux` skeletons that exchange the initial workspace snapshot over a Unix socket.
- Static server-owned `PaneSurfaceSnapshot` and dumb client text rendering.
- Basic key `InputEvent` sent from `nmux` back to `nmuxd`.
- Local-only attach prelude with known pane surface versions.
- Current-version reconnect test and stale-version full snapshot test.
- Patchable reconnect response using `PaneSurfacePatch`.
- ADR 0003 for scrollback as a separate synchronized object.
- `ScrollbackFetch` and `ScrollbackChunk` schema bodies.
- Static scrollback object with range-fetch tests and local client rendering.
- ADR 0004 for presence and attach modes.
- `PresenceUpdate`, `AttachMode`, and `PresenceKind` schema objects.
- Local attach prelude actor mode, presence update, and read-only input policy tests.
- Sequential two-client attach test against one local session.
- ADR 0005 for the process host boundary.
- Internal `nmux-core` process host abstraction with local/container/sandbox host choices and lifecycle tests.
- Concrete local command host that starts, writes to, and stops local child processes behind the host interface.
- Local PTY host behind the same process host boundary, using `portable-pty` for start, write, resize, and stop lifecycle coverage.
- ADR 0006 for the interim PTY text surface.
- Core pane state can hydrate visible rows and scrollback from process output bytes.
- Local attach can serve process-derived pane state over the Unix socket.
- Nonblocking process output seam and local PTY output queue for polling already-pumped bytes.
- Local attach polls available process output before choosing snapshot, patch, or no surface response.
- `nmuxd` starts a local PTY host and can run a command-backed PTY smoke through the local client.
- Read-write local input is forwarded to the process host, while read-only attaches do not forward input.
- Local attach polls process output after forwarded input so echoed command output can update backend-owned scrollback.
- Client flags can drive an interaction smoke across attaches with custom input and scrollback ranges.
- ADR 0007 for the Ghostty/libghostty frontend boundary.
- Client-side pane surface render state can initialize from snapshots, apply row patches by row index, and reject patch base-version mismatches.
- The `nmux` CLI renders through client-side pane surface state and can persist that state with `--state` so a later attach sends known pane versions and can apply a server patch.
- The `nmux --follow` local loop keeps one client render state across repeated reconnects, applies snapshots/patches, and treats current-version reconnects as no render update.
- ADR 0008 for promoting the local attach prelude into a public FlatBuffers `AttachRequest`.
- `AttachRequest` and known pane surface versions are public schema objects, and the local attach handshake uses a FlatBuffers envelope instead of the text prelude.
- ADR 0009 for the tmux adapter process boundary, including licensing and ownership rules.
- Internal tmux adapter inventory structs can map a tmux session/window/active-pane view into the existing nmux session model without launching tmux or changing the public schema.
- ADR 0010 for the herdr integration boundary and AGPL membrane.
- ADR 0011 for making live local interactive attach the next milestone before live adapter work.
- The local library has a long-lived attach loop helper with tests for repeated input/output patches, no-update behavior, read-only observation without input forwarding, and clean detach.
- `nmuxd --live` serves one live client until detach; `nmuxd --live-cycles` remains available for bounded smoke tests.
- The CLI integration tests launch `nmuxd --live-cycles` and `nmux --live` against a command-backed PTY and assert repeated streamed output.
- `nmux --live --stdin` streams stdin lines over one attached connection and exits cleanly on stdin EOF when no explicit iteration limit is set.
- `nmux --live --stdin-bytes` sends stdin chunks as `InputKind.RawBytes` on a background thread and temporarily puts interactive TTY stdin into noncanonical mode; local echo defaults off and can preserve the TTY setting with `--local-echo tty`.
- Byte-streamed live sessions support Ctrl-] as a local detach key.
- Interactive byte-streamed live sessions listen for `SIGWINCH` and send resize intents from the current TTY size when explicit `--cols` and `--rows` are not set.
- `nmux --live --no-input` observes command output without forwarding input and, without `--iterations`, keeps polling until the daemon closes the live connection.
- `nmux --live` renders the requested initial scrollback range before streaming live surface updates, including in the first `--redraw` repaint buffer.
- `nmux --live --redraw` clears and repaints the current client-side pane surface on each streamed update instead of appending every render.
- Interactive TTY `--redraw` uses the alternate screen and restores it on exit; captured/piped redraw output remains plain clear/home escape output.
- Redraw mode includes the current workspace summary in every repaint and updates it when live workspace snapshots arrive.
- Live mode renders streamed snapshots and patches through the same client-side pane surface state used by reconnects, and can persist that state with `--state`.
- `nmux --live --cols --rows` sends `ResizeIntent` through the live loop; after process-host resize succeeds, the daemon commits the pane size and republishes a `WorkspaceTreeSnapshot`.
- The CLI workspace summary displays the daemon-published pane resize policy; `nmuxd --resize-policy` can publish `fixed`, `leader`, `active-client`, or `manual`, and `manual` blocks frontend viewport resize intents.
- `nmux --help` and `nmuxd --help` document the live attach, stdin, redraw, resize, and policy flags used by the current prototype.
- `nmux --help` and `nmuxd --help` include runnable local one-shot and live examples.
- The client flushes rendered output after live updates and reports local Ctrl-] detach on stderr.
- The client reports stdin EOF and live server socket close reasons on stderr for unbounded live exits.
- The client warns when an explicit live resize request conflicts with daemon-published `manual` resize policy.
- `nmux --help` calls out that the current renderer uses an interim text surface, not a VT-correct terminal emulator.
- Interactive TTY byte mode warns once on stderr that the interim text surface lacks full VT fidelity; scripted and piped runs stay quiet.
- Live-only frontend flags now fail fast outside `--live` instead of being silently ignored, and `--follow --live` is rejected as an ambiguous client mode.
- Read-write live clients no longer have to send input before receiving output; idle read-write cycles poll process output and stream updates when the backend-owned surface changes.
- The live daemon treats a client EOF/disconnect during read-write polling as a clean detach, so piped stdin clients can finish without requiring a matching daemon cycle count.
- `nmuxd --live-clients COUNT` keeps the same local workspace and PTY alive across bounded sequential live clients.
- `nmuxd` rejects ambiguous live server mode combinations and zero live counts instead of silently choosing one mode.
- `nmuxd` and `nmux` share a stable default socket path for local workflows without `--socket`.
- `nmux --connect-timeout-ms` can wait across daemon socket startup races.
- ADR 0012 documents the backend terminal engine boundary.
- `nmux-core` routes pane output and cursor ownership through a terminal engine trait, with the interim text engine as the current implementation.
- Local daemon serving paths keep terminal engine instances alive per pane across output polls, resize handling, and sequential live clients.
- Resize-driven surface and cursor changes advance the pane surface version, so clients can synchronize them through the normal snapshot/patch path.
- Cursor shape now flows from the terminal engine boundary into pane surface snapshots and patches instead of being hardcoded during serialization.
- Surface kind now flows from the terminal engine boundary into pane surface snapshots, preserving the existing schema path for future alternate-screen extraction.
- Cursor-only terminal updates now use `PatchKind::CursorOnly`, while surface-kind transitions require a snapshot instead of being misrepresented as row replacement patches.
- Scrollback now has pane-owned versioning, so backend-owned history changes are visible independently from pane surface versions.
- Local attach response tests cover surface-kind transitions, proving alternate-screen style changes get a full snapshot when the current patch schema cannot express them.
- Scrollback chunk generation is pane-scoped, and attach/live fetch handling uses the requested pane ID rather than implicitly returning the initial pane.
- Mode-only terminal updates now carry explicit terminal mode fields in the surface schema and apply through `PatchKind::ModeOnly`.
- Client-side surface state applies supported cursor-only and mode-only patch kinds, and rejects unsupported patch kinds instead of treating them as cursor-only updates.
- Pane surface snapshot and patch serialization is pane-scoped, matching the per-pane terminal engine and scrollback boundaries.
- `nmuxd --terminal-engine interim` exposes the current engine choice explicitly; backend `libghostty-vt` is imported behind an opt-in Cargo feature while the project decides when to accept the native Zig/Ghostty build in the default path.
- Pane state now preserves `CellRun` and style-table data internally, and snapshots/patches serialize stored runs instead of flattening every row to one default-style string.
- Style-table changes now force a full pane surface snapshot, while row-run-only changes can still use `PaneSurfacePatch`.
- `ScrollbackChunk` now carries the pane style table, so scrollback row runs no longer reference style IDs without an accompanying table.
- The optional `libghostty-vt` engine extracts visible and scrollback rows as style-separated cell runs with cell-width metadata, while keeping plain rendered text available for current clients. Style-bearing trailing blank cells are preserved so background-colored row regions survive, while default trailing blanks stay trimmed.
- Basic SGR style flags now have `libghostty-vt` extraction coverage for bold, italic, underline, and strikethrough style-table bits.
- Underline color now has `libghostty-vt` extraction coverage proving it resolves into an RGBA style-table entry.
- Cursor-only `libghostty-vt` patches now cover cursor movement, visibility changes, and DECSCUSR visual shape changes.
- `libghostty-vt` extraction tests now cover alternate-screen entry and restoration to the main screen.
- `libghostty-vt` extraction tests now pin alternate-screen scrollback omission: main scrollback is preserved while alternate-screen output stays out of nmux scrollback until the protocol shape is explicit.
- Palette-indexed SGR colors now have `libghostty-vt` extraction coverage proving they resolve into RGBA style-table entries.
- `libghostty-vt` render-state colors are carried as `TerminalColorState`: backend-observable default foreground/background, active palette, palette overrides, and explicit cursor-color state. Color-state changes currently require a full surface refresh while incremental palette diffs remain withheld.
- `libghostty-vt` extraction tests now cover combining marks and emoji ZWJ clusters as preserved run text with per-cell widths, alongside existing double-width cell coverage.
- OSC 8 hyperlink text and backend row/cell hyperlink presence are covered, with run-level presence carried in `CellRun.flags` while `hyperlink_id` remains unset until nmux has a real hyperlink table.
- The `libghostty-vt` safe API is now tested for bracketed paste mode tracking, and nmux carries that mode in pane surface snapshots and patches.
- The `libghostty-vt` paste validator is now tested for newline and bracketed-paste terminator injection detection, and local clients can forward UTF-8 paste input through `PasteInput` with bracketed-paste delimiters controlled by the pane's current mode.
- The `libghostty-vt` safe API is now tested for mouse tracking modes, mouse encoding formats, and SGR mouse encoding; nmux carries both the compatibility mouse-tracking flag and detailed tracking-mode/format state, and local live clients can forward explicit mouse press/release/motion input through daemon-owned terminal-engine encoding.
- The `libghostty-vt` safe API is now tested for focus reporting mode and focus event encoding, and local clients can forward focus gained/lost input through `FocusInput` when the daemon-owned pane mode reports focus reporting enabled.
- The `libghostty-vt` safe API is now tested for application keypad mode tracking, and local clients can forward keypad Enter and digit key names through daemon-owned application-keypad mode encoding.
- The `libghostty-vt` safe API is now tested for application cursor mode tracking, and local clients can forward arrow key names through daemon-owned application-cursor mode encoding.
- Local clients can forward common named keys for Enter, Tab, Backspace, Escape, Insert/Delete, Home/End, PageUp/PageDown, and F1-F12 without requiring clients to inject raw PTY bytes.
- Named-key forwarding now goes through the live pane terminal engine; the `libghostty-vt` key encoder is tested for application-cursor state and modified named keys, and `nmux --key-modifiers` exposes `shift`, `ctrl`, `alt`, and `super` for live named-key input while broader physical-key/text-event forwarding remains a protocol/frontend boundary decision.
- The `libghostty-vt` safe API is now tested for origin and wraparound modes, and nmux carries those mode states in pane surface snapshots and patches.
- `libghostty-vt` render-state cursor blinking is carried in `CursorState` and cursor-only patches, with old client cache entries defaulting to blinking enabled.
- Terminal title and OSC 7 working-directory metadata are carried through pane surface snapshots, patches, and cached client state.
- `libghostty-vt` OSC 133 prompt-row semantics and per-run semantic content are carried through surface snapshots, surface patches, scrollback chunks, and cached client state, while nmux still withholds broader semantic command IDs, ranges, lifecycle, and exit metadata.
- `libghostty-vt` render-state row dirty flags and row state hashes are carried through surface snapshots, surface patches, scrollback chunks, and cached client state; `ReplaceRows` patches now carry only changed rows when pane geometry is stable, while richer run/region damage protocol fields remain withheld.
- `libghostty-vt` Kitty graphics placeholder row metadata is carried through surface snapshots, surface patches, scrollback chunks, and cached client state, while nmux still withholds image placement, dimensions, pixel-data, persistence, and fallback protocol fields.
- The `libghostty-vt` key encoder is tested against terminal application-cursor mode and protocol modifiers, documenting the future frontend path for mode-aware key encoding without raw PTY replay.
- Persisted `nmux --state` files now retain cached title, OSC 7 working directory, terminal modes including mouse tracking mode/format, row runs, style tables, terminal color state, OSC 133 row/run semantic metadata, row dirty flags, row state hashes, and Kitty placeholder row metadata alongside fallback row text; current-version live reattach reuses cached terminal modes for initial input gates when no surface frame arrives; cache reuse is scoped to the daemon socket identity, while older unscoped text-only state files still load as default-style rows and force one fresh snapshot.
- Client-side scrollback decoding now retains `ScrollbackRow` runs alongside rendered fallback text.

Next:

- Decide, with a later ADR, when the optional `libghostty-vt` feature should enter regular CI and whether `nmuxd --terminal-engine libghostty-vt` should become the documented default.
- Use [the terminal state extraction checklist](terminal-state-extraction.md) as the gate for making `libghostty-vt` the default engine or expanding protocol fields.
- Keep client attach, reconnect, live streaming, and scrollback fetch semantics on nmux state objects.
- Keep [the Ghostty/libghostty surface hydration tracker](upstream/ghostty-surface-hydration.md) current as upstream APIs change.

## Milestones

### M0: Protocol Contract

Goal: define the smallest state-sync schema that can carry workspace tree snapshots, pane surface snapshots, pane surface patches, input events, resize intents, and errors.

Exit evidence:

- `schema/nmux.fbs` exists.
- Schema validation runs from a documented command.
- `docs/protocol.md` explains M0 scope and exclusions.

### M1: Local nmuxd Skeleton

Goal: start a local daemon that can own a single session, tab, and pane, accept a local client connection, and exchange M0 envelopes.

Exit evidence:

- A local server command starts.
- A local client command attaches.
- The client receives a `WorkspaceTreeSnapshot`.
- Tests cover envelope encode/decode and initial session creation.

Status: Done for the static local skeleton. See [docs/running.md](running.md).

### M2: Dumb Viewer

Goal: render a pane surface from server-owned state and send basic keyboard input back to the daemon.

Exit evidence:

- A simple TUI or web viewer can attach locally.
- The viewer renders a surface snapshot.
- Input events reach the daemon with actor and pane IDs.

Status: Done for the static local skeleton. The local client renders a server-owned `PaneSurfaceSnapshot` and sends a basic key `InputEvent` back to the daemon.

### M3: Reconnect

Goal: allow a client to reconnect with known object versions and receive either patches or a fresh snapshot.

Exit evidence:

- A test demonstrates reconnect from a current version.
- A test demonstrates stale reconnect falling back to a full snapshot.

Status: Done for the local skeleton. `AttachRequest` now carries known pane surface versions in the public FlatBuffers schema, and tests prove current-version, patchable-version, and stale-version behavior for pane surfaces.

### M4: Scrollback Object

Goal: introduce scrollback as a separate synchronized object with lazy range fetches.

Exit evidence:

- Scrollback chunks are versioned separately from the visible surface.
- Tests cover consistency between visible rows and scrollback ranges.

Status: Done for the static local skeleton. The daemon serves a range-addressed `ScrollbackChunk`, and tests cover visible-surface consistency with the scrollback tail.

### M5: Multi-Player

Goal: support multiple actors with presence and read-only vs read-write attach modes.

Exit evidence:

- Two clients can attach to one session.
- Presence updates identify actors and capabilities.
- Read-only clients cannot send pane input.

Status: Done for the local skeleton. Two clients can attach sequentially to one session, presence identifies actor capabilities, and read-only actors do not send pane input.

### M6: Sandbox Host

Goal: isolate process hosting behind a local/container/sandbox host abstraction.

Exit evidence:

- Pane process lifecycle is mediated by a host interface.
- Local and sandbox host choices are represented without changing the protocol core.

Status: Done for the internal boundary and first local PTY host. `nmux-core` now models host specs, local/container/sandbox host kinds, a `ProcessHost` lifecycle interface, lifecycle tests, a concrete local command host, a local PTY host, an interim process-output text surface, and a nonblocking output polling seam without changing the FlatBuffers protocol. `nmuxd` starts the local PTY host, local attach can poll already-pumped output before responding, read-write input is forwarded to the hosted process, and output is polled again after forwarded input.

### M7: Ghostty Frontend

Goal: prove a native Ghostty/libghostty-facing frontend path.

Exit evidence:

- The project has a documented answer for external surface rendering.
- A prototype frontend can render server-owned state or the needed upstream/API work is documented.

Status: Done. ADR 0007 documents the boundary: backend libghostty/libghostty-vt is the primary correctness path, frontend libghostty rendering must not re-parse PTY bytes, and a temporary nmux renderer is the practical prototype path unless Ghostty/libghostty exposes external surface hydration. The local client library has a small pane-surface render state that applies server-owned snapshots and patches by version, the CLI uses the same state path for output rendering, persisted reconnect state, and a read-only follow loop, and [the upstream tracker](upstream/ghostty-surface-hydration.md) records the external hydration API gap.

### M8: tmux Adapter

Goal: attach nmux to tmux as an adapter without making tmux the core model.

Exit evidence:

- Adapter process boundary is documented.
- Basic session/tree mapping is tested.

Status: Done. ADR 0009 documents the tmux adapter process boundary: tmux remains an external adapter target, clients continue to speak nmux FlatBuffers, and `nmuxd` keeps ownership of normalized workspace, pane, terminal-state, attach, reconnect, presence, and permission semantics. The core has a pure tmux inventory-to-nmux session mapping with tests for active window/pane focus hints, stable adapter-derived IDs, and invalid empty inventories, without launching tmux or changing the public schema.

### M9: herdr Adapter

Goal: keep any herdr integration outside the core repository and behind the nmux protocol.

Exit evidence:

- Integration boundary is documented.
- No AGPL code is copied into the nmux core.

Status: Done. ADR 0010 documents the herdr integration boundary: any herdr work stays behind an `nmux-herdr-adapter` process and, if needed, a separate repository; clients continue to speak nmux FlatBuffers; and no GPL or AGPL implementation material is copied into the nmux core.

### M10: Live Local Interactive Attach

Goal: turn local attach from request/response smoke tests into a long-lived interactive PTY session.

Exit evidence:

- `nmuxd` can keep serving one local PTY while at least one client stays attached.
- A long-lived read-write client can send repeated input events over one connection.
- Process output updates backend-owned pane state and reaches the client as patches or snapshots.
- A read-only client can observe updates without forwarding input.
- Tests cover repeated input/output, current-version no-update behavior, and read-only permission enforcement.

Status: Done. ADR 0011 documents why this came before live tmux or herdr adapter work. The local library now has a live attach loop helper and tests proving repeated read-write input can stream pane surface patches over one connection, read-write clients can receive idle process output without sending input first, current-version cycles send no update frame, read-only clients can observe output without forwarding input, and client EOF during read-write polling is a clean detach. `nmuxd --live` serves one live client until detach, `nmuxd --live-cycles` remains available for bounded smoke tests, integration tests cover command-backed PTY smoke, `nmux --live --stdin` streams stdin lines over the attached connection and exits cleanly on stdin EOF when unbounded, `nmux --live --stdin-bytes` sends stdin chunks as `InputKind.RawBytes` while keeping output polling active when stdin input is absent or partial and temporarily puts interactive TTY stdin into noncanonical mode, local echo defaults off and can preserve the TTY setting with `--local-echo tty`, interactive byte-streamed clients listen for `SIGWINCH` and send resize intents from the current TTY size when explicit `--cols` and `--rows` are not set, Ctrl-] detaches a byte-streamed live client, `nmux --live --no-input` can poll until the daemon closes the live connection, `nmux --live --redraw` repaints the current client-side pane surface on each update, live mode renders through the same client-side pane surface state used by reconnects, `nmux --live --cols --rows` forwards `ResizeIntent` through the process-host resize boundary, successful live resizes are committed into the workspace tree and republished as `WorkspaceTreeSnapshot`, `nmuxd --resize-policy` can publish `fixed`, `leader`, `active-client`, or `manual`, `manual` blocks frontend viewport resize intents, the CLI workspace summary displays the daemon-published resize policy, and both binaries have `--help` output for the current prototype flags.

### M11: Terminal Frontend Polish

Goal: make the temporary local CLI frontend more comfortable and explicit while preserving the longer-term direction toward a VT-correct libghostty-backed surface.

Exit evidence:

- Live redraw output is stable and intentionally flushed for interactive use.
- The CLI clearly reports or avoids ambiguous states around detach, EOF, resize policy, and unsupported terminal-surface fidelity.
- Runnable docs and help output cover the expected live attach workflows.
- Tests cover any changed user-visible terminal output behavior.

Status: Done. M11 kept the local frontend prototype honest while making live attach easier to use. The client now flushes live and redraw output after render updates; reports Ctrl-] detach, stdin EOF, and live server close reasons on stderr; warns when explicit live resize requests conflict with daemon-published `manual` resize policy; calls out interim text-surface and non-VT-correct renderer limitations in help and interactive byte mode; rejects live-only flags outside `--live`; renders requested initial scrollback context before streaming updates, including the initial redraw paint; uses the alternate screen for interactive TTY redraw and restores it on exit; keeps the current workspace summary visible in redraw mode; and includes runnable one-shot and live examples in help output. Unit and CLI integration tests cover the changed user-visible terminal output behavior, and `nix develop path:$PWD -c make check` is the full verification gate.

### M12: Live Workspace Usability

Goal: continue moving the local prototype toward a usable live workspace while preserving backend-owned terminal state and avoiding raw PTY replay shortcuts.

Exit evidence:

- The next live-workflow improvement is selected from current CLI behavior and documented before or as it lands.
- Any user-visible behavior change is covered by focused tests and `nix develop path:$PWD -c make check`.
- README, running docs, help output, and WORK.md stay aligned with the implemented behavior.
- Interim renderer limitations remain explicit until a libghostty-backed state/render integration replaces the temporary text surface.

Status: Done for the current local live-workflow milestone. Future live-workflow improvements should be selected from current CLI behavior and should preserve backend-owned state sync without expanding licensing risk or pretending the interim text surface is VT-correct.

Initial slice: `nmuxd --live-clients COUNT` keeps one daemon-owned local workspace and PTY alive across a bounded number of sequential live clients. This is intentionally not simultaneous multi-client attach; it is a small persistence step for live reattach workflows. The daemon also rejects ambiguous live server mode combinations and zero live counts so scripted workflows fail before binding a socket. A persisted live client state file can now reattach at the current pane surface version without waiting for a raw replay or requiring a redundant surface update. Client bounded live/follow loops reject `--iterations 0` before connecting so scripts cannot silently request a no-op loop. Explicit client input modes now reject conflicting `--key`, `--stdin`, `--stdin-bytes`, and `--no-input` combinations before connecting. Live loop timing, explicit resize dimensions, scrollback range flags, and connect timeout values reject zero or out-of-range values before connecting. Client state load/save failures report the state path so bad persisted live reattach state is actionable. `nmuxd` and `nmux` now share a stable default local socket path, so simple local workflows do not require spelling `--socket` on both sides; help output and quick-start docs show that path first and document how the default path is chosen, including fallback when `XDG_RUNTIME_DIR` is empty or relative. Client connection failures include the socket path so missing-daemon and wrong-socket mistakes are actionable, and `nmux --connect-timeout-ms` can wait across daemon socket startup races. `nmuxd --live-forever` serves sequential live clients against one workspace and PTY until the daemon is stopped. `nmuxd` bind failures include the socket path, existing socket paths are rejected with a recovery hint so users do not accidentally hide another local workspace, and normal bounded daemon exits remove their socket path only when that path still points at the socket file the daemon created.

### M13: Backend libghostty-vt Extraction

Goal: replace the interim byte-to-text surface with daemon-owned `libghostty-vt` terminal-state extraction while preserving nmux state sync.

Non-goal: frontend Ghostty renderer hydration. That remains tracked separately because it depends on an API that can render externally supplied nmux state without client-side PTY replay.

Exit evidence:

- PTY bytes enter `libghostty-vt` inside `nmuxd`, and clients still receive nmux `PaneSurfaceSnapshot`, `PaneSurfacePatch`, and `ScrollbackChunk` objects.
- The extraction boundary documents how cursor state, modes, alternate screen, palettes, hyperlinks, images, grapheme clusters, and cell widths map into current or future nmux schema.
- [The terminal state extraction checklist](terminal-state-extraction.md) is kept current with the first backend mapping and schema gaps.
- Existing live attach, reconnect, scrollback, and resize tests continue to assert backend-owned state-sync semantics.
- The interim text surface remains explicitly labeled temporary until replaced.
- No GPL or AGPL terminal parser code is copied into the core.

Status: Started. ADR 0012 documents the terminal engine boundary, ADR 0013 documents the optional `libghostty-vt` backend, ADR 0015 documents per-run cell semantic content, and ADR 0016 documents terminal color state. `nmux-core` exposes a terminal engine trait, the interim text behavior implements it, pane cursor state and resize updates are routed through that engine boundary, local daemon serving paths keep engine instances alive per pane across output polls and sequential clients, and `nmuxd --terminal-engine interim` exposes the current engine choice. `libghostty-vt` is now an optional Cargo feature with a compile-checked and smoke-tested engine path. The optional engine extracts cursor state including blink, surface kind, terminal title, OSC 7 working-directory metadata, terminal modes including detailed mouse tracking mode and format, terminal color state, OSC 133 row semantic prompt state, OSC 133 per-run semantic content, row dirty state, Kitty virtual placeholder rows, style-separated visible row runs, cell widths, resize/reflow output, and backend-owned scrollback into nmux state objects; snapshots and scrollback chunks carry style tables, terminal color state, row semantic prompt metadata, run semantic content, row dirty flags, row state hashes, and Kitty placeholder metadata, snapshots and patches carry title, working-directory, mode, and color metadata, patches avoid unsupported style-table changes, color-state changes require a full refresh, stable-geometry row replacements emit sparse row updates, mode-only updates use explicit `PatchKind::ModeOnly` payloads, persisted client state keeps cached cursor blink, title, working-directory, mode state, terminal color state, row semantic prompt state, run semantic content, row dirty state, row state hashes, Kitty placeholder state, row runs, and style tables, alternate-screen scrollback omission is pinned while the protocol shape remains withheld, local paste forwarding uses `PasteInput` and pane-mode-controlled bracketed-paste delimiters, local focus forwarding uses `FocusInput` gated by daemon-owned focus-reporting mode, common named-key forwarding avoids raw PTY replay through terminal-engine encoding, keypad Enter and digit forwarding uses daemon-owned application-keypad mode, arrow-key forwarding uses daemon-owned application-cursor mode, modified named-key protocol input is preserved for engines that can encode it, mouse press/release/motion forwarding uses daemon-owned mouse-tracking mode and terminal-engine encoding, broader semantic command IDs/ranges/lifecycle/exit metadata remains withheld, richer run/region damage fields remain withheld, image placement fields remain withheld, incremental palette diffs remain withheld, and broader physical-key/text-event forwarding remains withheld. The default engine remains interim until the project deliberately accepts the native Zig/Ghostty build in regular development and CI.

Initial boundary slice: `nmux-core` now exposes a terminal engine boundary for daemon-owned pane output hydration. The existing interim text behavior lives behind that boundary, preserving current `PaneSurfaceSnapshot`, `PaneSurfacePatch`, and `ScrollbackChunk` semantics while creating the replacement point for backend `libghostty-vt` extraction. Local daemon serving paths keep terminal engine instances alive per pane across output polls, resize handling, and sequential clients, and cursor state is now produced by the terminal engine boundary, matching the stateful shape expected from a real VT engine. Focused tests prove a single stateful engine instance can span output, resize, and later output. The optional `libghostty-vt` feature compiles against the Rust crate plus vendored native Ghostty VT build, consumes ANSI sequences without leaking them into text rows, emits cursor-only patches for cursor movement, reports alternate-screen state, handles resize after wrapped output, extracts backend-owned scrollback through Ghostty's viewport state, feeds that history through `ScrollbackChunk`, and has a feature-gated live CLI smoke test for `nmuxd --terminal-engine libghostty-vt`.
