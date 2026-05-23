# AGENTS.md

This file is the operating guide for agents working in this repository.

## Project Shape

nmux is a portable Ghostty-style terminal workspace. The current direction is documented in:

- [README.md](README.md) for the seed idea.
- [docs/roadmap.md](docs/roadmap.md) for current milestone status and next steps.
- [WORK.md](WORK.md) for background product and architecture garden notes.
- [docs/contributor-workflow.md](docs/contributor-workflow.md) for default,
  opt-in VT, and promotion-evidence check paths.
- [docs/toolchain.md](docs/toolchain.md) for supported Nix tooling and the
  non-Nix requirements checklist.
- [docs/source-fetch-policy.md](docs/source-fetch-policy.md) for opt-in
  `libghostty-vt-sys` source-fetch rules and remaining promotion blockers.
- [docs/adr/0024-native-vt-source-policy-criteria.md](docs/adr/0024-native-vt-source-policy-criteria.md)
  for criteria a future native-VT source-policy promotion decision must satisfy.
- [docs/packaging.md](docs/packaging.md) for current distribution posture and
  native-VT packaging questions.
- [docs/adr/0025-native-vt-packaging-criteria.md](docs/adr/0025-native-vt-packaging-criteria.md)
  for criteria a future native-VT packaging promotion decision must satisfy.
- [docs/running.md](docs/running.md) for runnable local smoke tests.
- [docs/protocol.md](docs/protocol.md) for the FlatBuffers state-sync contract.
- [docs/protocol-futures.md](docs/protocol-futures.md) for withheld protocol
  object tracks that need ADRs before schema changes.
- [docs/default-engine-promotion.md](docs/default-engine-promotion.md) for the
  evidence required before `libghostty-vt` can become the default engine or a
  regular CI requirement.
- [docs/ci.md](docs/ci.md) for the required default-engine GitHub Actions gate
  and manual promotion evidence bundle workflow.
- [docs/adr/0026-native-vt-ci-promotion-criteria.md](docs/adr/0026-native-vt-ci-promotion-criteria.md)
  for criteria a future native-VT CI promotion decision must satisfy.
- [docs/adr](docs/adr) for durable architecture decisions.
- [docs/upstream](docs/upstream) for upstream/API gaps that block otherwise
  desirable local work.

Use `docs/roadmap.md` as the current implementation tracker and next-step source. Treat `WORK.md` as background garden notes. Promote stable decisions into ADRs when they affect protocol shape, process boundaries, terminal-state ownership, adapter boundaries, or licensing posture.

Current sequencing:

- M12 live workspace usability is implemented enough for the local daemon/client workflow to support bounded and unbounded sequential live clients, read-only default attach, explicit `AttachStatus` current-surface barriers, active-pane-scoped post-attach input/resize/scrollback control without guessing `pane-1`, protocol errors for missing active-tab or active-pane metadata, pane/input-attributed protocol `Error` frames, explicit one-shot text/paste/named-key/focus/mouse input on current-surface reattach, explicit resize requests as `UserCommand` resize intents, automatic terminal-size changes as `FrontendViewport` resize intents, scoped cached-surface reattach across one-shot/follow/live paths, follow-mode rejection of input flags, default local socket workflows, `NMUX_*` pane identity environment injection plus `nmux --print-context` inspection for spawned local PTYs, persisted reconnect state, explicit validation, and runnable docs.
- M13 backend `libghostty-vt` extraction is done for the opt-in terminal-state correctness milestone. The optional engine is imported, feature-tested, and smoke-tested for daemon-owned VT ingestion, cursor state including blink and decoded enum validation plus real `--state` cursor-only reattach persistence, terminal color state, color-only surface patches with scoped palette diffs, mixed no-row color/mode cache refresh, and real `--state` reattach persistence, terminal title metadata, OSC 7 working-directory metadata, metadata-only `CursorOnly` no-row patches with `--state` reattach persistence, OSC 133 row semantic prompt metadata, OSC 133 per-run semantic content with decoded patch/cache enum validation, row dirty flags, row state hashes, Kitty placeholder metadata, styled visible and scrollback rows, cell widths, graphemes, alternate-screen transitions with scrollback omission and structured main scrollback preservation, styled/wide run state with real `--state` reattach persistence, explicit terminal mode payloads with decoded mouse enum validation, mode-only surface patches with real `--state` reattach persistence, sparse row updates, `ReplaceRows` cache persistence for row metadata and OSC 8 hyperlink run flags with real `--state` reattach persistence, `FullRefreshRequired` snapshot recovery on known-version live reattach, decoded attach/presence identity, workspace/surface/status/scrollback IDs, pane-tree, control-plane enum, input modifier, input payload, error message/pane ID, and pane-scoped client ID validation, `AttachStatus.pane_id`, `surface_version`, and `surface_state` as the authoritative current-surface cache key and post-attach control target with rejection for missing, extra, wrong-kind, or wrong-pane following surface frames, explicit full-object hyperlink tables with emitted hyperlink IDs still zero until `libghostty-vt` exposes structured identity data, terminal-generated PTY reply routing for DECRQM query responses through a real live PTY command, PasteInput forwarding, mode-gated MouseInput forwarding with optional SGR-pixel coordinates, mode-gated FocusInput forwarding, current-surface live key/paste/named-key/focus/mouse forwarding and focus/mouse rejection including current-surface SGR-pixel mouse coverage, common named-key forwarding, mode-aware keypad Enter/digit forwarding, mode-aware cursor-arrow forwarding, engine-backed named-key encoding with modifier preservation, monotonic post-attach envelope/input sequencing across mixed live-style frames, protocol-visible host input/resize/output-poll failures, `PaneNotFound` errors for unknown pane-scoped client intents, validated 1-based contiguous public scrollback ranges, stale scrollback version precondition errors, scoped persisted scrollback range/version metadata, stale scrollback retry, current-surface scrollback fetches, live CLI resize/reattach behavior, and mode-aware key encoding. ADR 0018 keeps the default engine `interim` while requiring full feature-enabled `make check-ghostty-vt` coverage for related changes.
- M14 is accepted in ADR 0023: `libghostty-vt` remains opt-in until native build, regular CI, packaging, source-fetch, and developer-workflow evidence tracked in `docs/default-engine-promotion.md` justifies promotion; frontend Ghostty renderer hydration and richer protocol objects are separate tracks with upstream trackers, `docs/protocol-futures.md`, or future ADRs.
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
nix develop . -c flatc --version
nix develop . -c make toolchain-info
nix develop . -c make check
```

The Nix shell provides `flatc` through `pkgs.flatbuffers`; no separate
FlatBuffers install is needed for schema validation in the supported
development path. `make check` runs FlatBuffers schema validation and
`cargo test --workspace`. For narrower iteration, prefer targeted `cargo test`
commands inside the same `nix develop . -c ...` wrapper, then run full
`make check` before committing implementation changes.
`make toolchain-info` prints the active cargo, rustc, flatc, Zig, and
source-fetch environment values used for promotion-evidence records.

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

Use the explicit combined gate for release-style validation or
default-engine-promotion evidence:

```sh
nix develop . -c make check-all
nix develop . -c make promotion-sample
nix develop . -c make promotion-cold-target-sample
nix develop . -c make promotion-local-sample
nix develop . -c make promotion-evidence-bundle
nix develop . -c make promotion-evidence-verify
nix develop . -c make source-fetch-provenance-sample
nix develop . -c make packaging-sample
nix develop . -c make packaging-layout-sample
nix develop . -c make packaging-provenance-sample
nix develop . -c make packaging-provenance-verify
nix develop . -c make packaging-archive-sample
nix develop . -c make packaging-archive-runtime-smoke
```

`check-all` runs the regular default-engine gate plus the opt-in
`libghostty-vt` gate without changing what `make check` means.
`promotion-sample` prints `toolchain-info` and times `check-all` for evidence
rows in `docs/default-engine-promotion.md`.
`promotion-cold-target-sample` clears `target/promotion-cold` and times
`check-all` with that fresh Rust target directory; it does not clear Cargo
registry, Git source, or Nix store caches.
`promotion-local-sample` runs `source-fetch-provenance-sample`,
`promotion-sample`, and `packaging-archive-runtime-smoke` for one local evidence
pass.
`promotion-evidence-bundle` runs `promotion-local-sample` and gathers the log,
toolchain output, source-fetch report, package provenance, cargo tree, and
archive checksum under `target/promotion-evidence`.
`promotion-evidence-verify` checks an existing bundle for required summary
fields, artifact files, source/provenance records, archive hash, and packaged
runtime smoke output; the bundle target runs it before printing the artifact
list.
`source-fetch-provenance-sample` records the active source mode and locked
`libghostty-vt` Cargo package records without inspecting Ghostty source.
`packaging-sample` builds default and opt-in release binaries in separate target
directories and prints artifact sizes plus binary versions for packaging
evidence rows.
`packaging-layout-sample` stages a local opt-in package layout with wrappers
that resolve `libghostty-vt` from `../lib`; it is packaging evidence, not a
release format decision.
`packaging-provenance-sample` writes a local manifest with staged file hashes,
toolchain/source mode, dependency tree, and dynamic dependency output.
`packaging-provenance-verify` regenerates that manifest and asserts the
required toolchain, source-mode, locked native-VT package, staged-file,
runtime-library, dynamic-dependency, and cargo-tree records are present.
`packaging-archive-sample` archives the staged layout, writes a SHA-256 file,
extracts it, and verifies the wrapped binaries from the archive.
`packaging-archive-runtime-smoke` starts the extracted opt-in `libghostty-vt`
daemon and attaches the extracted client to prove the packaged runtime layout
can serve a real pane.
GitHub Actions runs `make check` on pull requests and pushes to `main`; the
promotion-evidence-bundle job is manual and does not make `libghostty-vt` a
required CI gate. Use the field template in `docs/ci.md` when recording manual
CI promotion evidence.

If `nix develop` itself is unavailable, do not rewrite the flake or check in
machine-local store paths. Either use an already entered dev shell, or record the
environmental failure and keep changes scoped to work that can still be
validated honestly.
Use [docs/toolchain.md](docs/toolchain.md) when documenting non-Nix equivalents;
do not treat an unvalidated local setup as default-engine promotion evidence.
Use [docs/source-fetch-policy.md](docs/source-fetch-policy.md) when changing
`libghostty-vt-sys` source-fetch behavior; do not inspect or copy generated
Ghostty build output under `target/`.
Use [docs/packaging.md](docs/packaging.md) when discussing release binaries or
native-VT distribution.
Use [docs/contributor-workflow.md](docs/contributor-workflow.md) when changing
which work requires `make check`, `make check-ghostty-vt`, or `make check-all`.

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
