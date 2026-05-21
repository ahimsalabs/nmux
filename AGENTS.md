# AGENTS.md

This file is the operating guide for agents working in this repository.

## Project Shape

nmux is a portable Ghostty-style terminal workspace. The current direction is documented in:

- [README.md](README.md) for the seed idea.
- [WORK.md](WORK.md) for evolving product and architecture notes.
- [docs/adr](docs/adr) for durable architecture decisions.

Treat `WORK.md` as the active garden. Promote stable decisions into ADRs when they affect protocol shape, process boundaries, terminal-state ownership, adapter boundaries, or licensing posture.

## Working Rules

- Work sequentially. Do not parallelize implementation or documentation steps.
- Commit with `jj` between completed steps.
- Keep commits small enough that each one has a clear review purpose.
- Preserve user or agent work already present in the worktree unless explicitly told to change it.
- Prefer documentation under `docs/` once a note needs to outlive the current scratch plan.
- Update this file when repo workflow expectations change.

## Licensing Rules

- Do not copy code from GPL or AGPL sources into this repository.
- GPL and AGPL projects may be treated as prior art, references, or isolated adapter targets.
- Keep any future GPL/AGPL integration behind a process or repository boundary unless the project owner explicitly changes the licensing plan.

## Research Rules

Use `oracle --prompt '...'` when a question needs deep research about protocols, architecture, or other facts that should not be guessed from local context.

Do not use outside code as implementation source material unless its license is compatible with the intended nmux core licensing posture.

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
