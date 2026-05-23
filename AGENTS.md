# AGENTS.md

This file is the operating guide for agents working in this repository.

## Project Shape

nmux is a portable Ghostty-style terminal workspace. The current direction is documented in:

- [README.md](README.md) for the seed idea.
- [docs/roadmap.md](docs/roadmap.md) for current milestone status and next steps.
- [WORK.md](WORK.md) for background product and architecture garden notes.
- [docs/running.md](docs/running.md) for runnable local smoke tests.
- [docs/protocol.md](docs/protocol.md) for the FlatBuffers state-sync contract.
- [docs/adr](docs/adr) for durable architecture decisions.
- [docs/upstream](docs/upstream) for upstream/API gaps that block otherwise
  desirable local work.

Use `docs/roadmap.md` as the current implementation tracker and next-step source. Treat `WORK.md` as background garden notes. Promote stable decisions into ADRs when they affect protocol shape, process boundaries, terminal-state ownership, adapter boundaries, or licensing posture.

Current sequencing:

- M12 live workspace usability is implemented enough for the local daemon/client workflow to support bounded and unbounded sequential live clients, read-only default attach, explicit `AttachStatus` current-surface barriers, active-pane-scoped post-attach input/resize/scrollback control without guessing `pane-1`, protocol errors for missing active-tab or active-pane metadata, pane/input-attributed protocol `Error` frames, explicit one-shot text/paste/named-key/focus/mouse input on current-surface reattach, explicit resize requests as `UserCommand` resize intents, automatic terminal-size changes as `FrontendViewport` resize intents, scoped cached-surface reattach across one-shot/follow/live paths, follow-mode rejection of input flags, default local socket workflows, persisted reconnect state, explicit validation, and runnable docs.
- M13 backend `libghostty-vt` extraction is the current terminal-state correctness milestone. The optional engine is imported, feature-tested, and smoke-tested for daemon-owned VT ingestion, cursor state including blink and decoded enum validation plus real `--state` cursor-only reattach persistence, terminal color state, color-only surface patches with palette diffs, mixed no-row color/mode cache refresh, and real `--state` reattach persistence, terminal title metadata, OSC 7 working-directory metadata, metadata-only `CursorOnly` no-row patches with `--state` reattach persistence, OSC 133 row semantic prompt metadata, OSC 133 per-run semantic content with decoded patch/cache enum validation, row dirty flags, row state hashes, Kitty placeholder metadata, styled visible and scrollback rows, cell widths, graphemes, alternate-screen transitions with scrollback omission and structured main scrollback preservation, styled/wide run state with real `--state` reattach persistence, explicit terminal mode payloads with decoded mouse enum validation, mode-only surface patches with real `--state` reattach persistence, sparse row updates, `ReplaceRows` cache persistence for row metadata and OSC 8 hyperlink run flags with real `--state` reattach persistence, `FullRefreshRequired` snapshot recovery on known-version live reattach, decoded attach/presence identity, workspace/surface/status/scrollback IDs, pane-tree, control-plane enum, input modifier, input payload, and pane-scoped client ID validation, explicit full-object hyperlink tables with emitted hyperlink IDs still zero until `libghostty-vt` exposes structured identity data, terminal-generated PTY reply routing for DECRQM query responses through a real live PTY command, PasteInput forwarding, mode-gated MouseInput forwarding with optional SGR-pixel coordinates, mode-gated FocusInput forwarding, current-surface live key/paste/named-key/focus/mouse forwarding and focus/mouse rejection including current-surface SGR-pixel mouse coverage, common named-key forwarding, mode-aware keypad Enter/digit forwarding, mode-aware cursor-arrow forwarding, engine-backed named-key encoding with modifier preservation, monotonic post-attach envelope/input sequencing across mixed live-style frames, protocol-visible host input/resize/output-poll failures, `PaneNotFound` errors for unknown pane-scoped client intents, 1-based public scrollback ranges, stale scrollback version precondition errors, scoped persisted scrollback range/version metadata, stale scrollback retry, current-surface scrollback fetches, live CLI resize/reattach behavior, and mode-aware key encoding. ADR 0018 keeps the default engine `interim` while requiring full feature-enabled `make check-ghostty-vt` coverage for related changes and defers regular CI/default-engine promotion.
- M14 is planned as the post-M13 decision checkpoint: decide whether `libghostty-vt` becomes regular CI/default documentation or remains opt-in with explicit native build, packaging, and workflow blockers; keep frontend Ghostty renderer hydration and richer protocol objects as separate tracks.
- Frontend Ghostty renderer hydration is a separate upstream/API question; do not reintroduce client-side raw PTY replay to get there.

The current implementation is a Rust workspace:

- `crates/nmux-proto` owns FlatBuffers wire helpers and generated schema bindings.
- `crates/nmux-core` owns session state, process hosts, the terminal engine boundary, interim text-surface logic, optional `libghostty-vt` extraction, and adapter mapping helpers.
- `crates/nmux-cli` owns the `nmuxd` daemon, `nmux` client, Unix-socket local transport, and CLI integration tests.

## Working Rules

- Work sequentially. Do not parallelize implementation or documentation steps. If subagents are useful, run them as bounded read-only assistants and integrate their findings in the main worktree yourself.
- Check `jj status` before starting a step.
- Commit with `jj` after each coherent implementation or documentation step, after relevant checks pass.
- Keep commits small enough that each one has a clear review purpose.
- Preserve user or agent work already present in the worktree unless explicitly told to change it.
- Prefer documentation under `docs/` once a note needs to outlive the current scratch plan.
- Keep `WORK.md`, `README.md`, `docs/roadmap.md`, `docs/running.md`, and `docs/terminal-state-extraction.md` aligned when M13 coverage or protocol boundaries change.
- Record upstream/API blockers under `docs/upstream/` when local implementation
  would otherwise require raw PTY replay, duplicated terminal-state tracking, or
  guessing missing `libghostty-vt` data.
- When adding an ADR under `docs/adr/`, include status and date metadata and update `docs/adr/README.md` in the same commit.
- Update this file when repo workflow expectations change.
- Use subagents only for bounded read-only review, research synthesis, or implementation advice. Do not use them for parallel file edits or competing implementation tracks.
- Do not inspect or copy generated vendored Ghostty source under `target/`; treat it as build output for the `libghostty-vt` dependency, not as nmux source material.
- Keep generated protocol bindings in `crates/nmux-proto/src/generated` derived from `schema/nmux.fbs`; do not hand-edit generated files.

## Checks

Use the Nix development shell for repo checks:

```sh
nix develop . -c make check
```

`make check` runs FlatBuffers schema validation and `cargo test --workspace`. For narrower iteration, prefer targeted `cargo test` commands inside the same `nix develop . -c ...` wrapper, then run full `make check` before committing implementation changes.

If `schema/nmux.fbs` changes, regenerate bindings with:

```sh
nix develop . -c make generate-schema
```

The experimental backend Ghostty VT engine is feature-gated. Use this full
feature-enabled package check when changing the optional path:

```sh
nix develop . -c make check-ghostty-vt
```

The target runs `nmux-core` and `nmux-cli` with `--features libghostty-vt`, not
only name-filtered smoke tests. It sets `GIT_CONFIG_GLOBAL=/dev/null` to avoid
local GitHub HTTPS-to-SSH rewrites while `libghostty-vt-sys` fetches its pinned
Ghostty source. The Nix shell pins Zig 0.15 for that native build. Keep this
path opt-in unless a later ADR explicitly makes the native Ghostty/Zig build
part of regular CI, default development, and packaging.

If `nix develop` itself is unavailable, do not rewrite the flake or check in
machine-local store paths. Either use an already entered dev shell, or record the
environmental failure and keep changes scoped to work that can still be
validated honestly.

## Licensing Rules

- Do not copy code from GPL or AGPL sources into this repository.
- GPL and AGPL projects may be treated as prior art, references, or isolated adapter targets.
- Keep any future GPL/AGPL integration behind a process or repository boundary unless the project owner explicitly changes the licensing plan.

## Research Rules

Use `oracle --prompt '...'` for deep protocol, architecture, licensing, or upstream-fact questions that should not be guessed. If oracle quota or external research is unavailable, keep moving on repo-local work that does not depend on the unanswered question. Do not use outside code as implementation source material unless its license is compatible with the intended nmux core licensing posture.

## Documentation Rules

ADRs should use this shape:

- Title with ADR number and decision name.
- Status.
- Date.
- Context.
- Decision.
- Consequences.
- Licensing or compatibility notes when relevant.

Do not rewrite old ADRs to hide history. Add a new ADR that supersedes a prior one when the decision changes.
