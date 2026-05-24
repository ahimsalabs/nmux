# Renderer Equivalence Milestone

Status: Tracking.

Last reviewed: 2026-05-23.

This milestone gates stronger user-facing renderer claims. It sits between the
M13 opt-in `libghostty-vt` backend extraction milestone and any later claim that
nmux has a default-ready VT-correct renderer or frontend Ghostty hydration path.

## Current Decision

- The default user-visible renderer remains the interim text surface.
- The opt-in `libghostty-vt` engine is terminal-state extraction evidence, not
  renderer equivalence evidence by itself.
- Frontend Ghostty hydration remains an upstream/API question and must not use
  client-side raw PTY replay as its source of truth.
- Default-engine promotion evidence must not be described as renderer
  equivalence until this milestone has passing evidence.

## Goal

Prove that nmux can render server-owned terminal state with user-visible
fidelity close enough to justify changing default-renderer or frontend claims.
The proof must compare nmux-rendered state against a trusted terminal rendering
path for realistic terminal workloads, while keeping the daemon as the owner of
terminal state.

## Required Evidence

- Fixture corpus: representative terminal workloads covering shell prompts,
  command output, cursor movement, alternate screen programs, color/style
  regions, wide and combining graphemes, hyperlinks, bracketed paste mode,
  mouse/focus modes, resize/reflow, scrollback, and metadata-only changes.
- Oracle renderer: a documented trusted rendering path for the same workloads,
  such as Ghostty/libghostty render output or another explicitly accepted
  reference. The oracle must not be copied into nmux if its license is
  incompatible.
- Comparison harness: a repeatable command that runs the corpus through nmux
  server-owned state and the oracle renderer, then reports structured diffs.
- Tolerance policy: explicit rules for accepted differences, including font
  shaping, ambiguous-width policy, terminal theme defaults, cursor blink timing,
  image protocol omissions, and withheld protocol objects.
- Regression gate: a focused local check that can run before default-renderer
  or frontend claim changes, plus guidance for when the heavier renderer
  equivalence corpus is required.
- User-facing claim map: documented wording for what nmux can and cannot claim
  after the evidence passes.

## Current Harness

`make renderer-equivalence-smoke` runs focused feature-gated corpus projection
checks. The `nmux-core --features libghostty-vt` corpus proves that
representative server-owned terminal state for styled text, default text, wide
cells, title metadata, bracketed paste mode, mouse tracking mode, and hyperlink
presence is projected into nmux `TerminalUpdate` rows, runs, styles, modes, and
metadata without raw ANSI text leaking into fallback rows. The `nmux-cli`
integration smoke runs a real `nmuxd --terminal-engine libghostty-vt` plus
`nmux --json` attach, materializes the exported JSON into a small canonical
workspace/surface/scrollback shape, and compares it to an expected semantic snapshot for
structured rows/runs, style tables and IDs, compact terminal color state, cell
widths, hyperlink-presence flags, OSC 133 row/run semantics, dirty and
Kitty-placeholder row metadata, cursor state including blink state, terminal
modes, title/OSC 7 metadata, initial workspace geometry and wrap/reflow
behavior, main screen restoration after alternate screen, and omission of raw
control text.
The corpus lives in `fixtures/renderer-equivalence/*.json` so each fixture's
shell command, direct terminal-output chunks, and canonical expected state can
grow without burying fixture semantics in test code. The core harness replays
the direct chunks into `libghostty-vt`; the CLI harness runs the shell command
through a real `nmuxd` and `nmux --json`.
Set `NMUX_RENDERER_EQUIVALENCE_ARTIFACT_DIR=target/renderer-equivalence` to
write one raw `nmux --json` artifact plus one deterministic
`*.canonical.json` nmux-state projection per fixture. The canonical artifacts
are the current expected-vs-actual comparison shape and are intended as the
handoff point for a later oracle renderer comparison.

This is nmux-side fixture evidence only. It is intentionally not wired into the
normal default gate, and it does not satisfy the oracle renderer or pixel/state
comparison requirements above.

## Open Questions

- Which renderer is the first oracle: Ghostty/libghostty, a screenshot-based
  Ghostty run, a serialized libghostty render-state comparison, or a smaller
  purpose-built reference harness?
- Which scenarios are hard blockers for "VT-correct renderer" wording versus
  known limitations that can remain explicit?
- How should screenshot or pixel comparisons account for platform fonts,
  antialiasing, terminal theme configuration, and DPI?
- Where should image protocols and hyperlink identity land: this milestone, the
  future protocol-object tracks, or a later renderer-specific milestone?

## Acceptance Rule

Do not promote default-renderer or frontend Ghostty hydration claims until an
ADR accepts the renderer equivalence evidence plan and the repository records a
passing corpus run with clear residual limitations.
