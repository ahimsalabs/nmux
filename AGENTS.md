# AGENTS.md

Operating guide for agents working in this repository.

## Goal

Build nmux into a usable terminal multiplexer. The priority list is in
[WORK.md](WORK.md) — work through it top to bottom. Ship code that users can
run; don't substitute documentation or evidence infrastructure for features.

## Reading order

Start here, then follow links as needed:

1. **This file** — goal, rules, checks
2. **[WORK.md](WORK.md)** — what to build next (punch list, <50 lines)
3. **[README.md](README.md)** — what works today, quick start
4. **[docs/roadmap.md](docs/roadmap.md)** — milestone history (M0-M14 done)
5. **[docs/protocol.md](docs/protocol.md)** — FlatBuffers state-sync contract
6. **[docs/running.md](docs/running.md)** — detailed usage and examples
7. **[docs/adr](docs/adr)** — architecture decisions (read on demand)
8. **[docs/protocol-futures.md](docs/protocol-futures.md)** — withheld protocol tracks

Don't read everything upfront. WORK.md and this file are enough to start.
Read protocol.md when changing the schema, ADRs when making boundary
decisions, running.md when changing CLI behavior.

## Project shape

nmux is a portable terminal workspace. The daemon owns terminal state and
syncs it over FlatBuffers to clients.

Crate layout:

- `crates/nmux-proto` — FlatBuffers wire helpers and generated schema bindings
- `crates/nmux-core` — session state, process hosts, terminal engine boundary
- `crates/nmux-cli` — `nmuxd` daemon, `nmux` client, integration tests

## Context handoff

Use the jj working copy description (`@`) as a progress note between
iterations. At the start of each session, read it with `jj log -r @ -T description`.
Before finishing a session or between major steps, update it:

```sh
jj describe -m "$(cat <<'EOF'
## Current item
Multi-pane: split panes horizontally/vertically

## Done so far
- Added PaneLayout tree to session model
- Unit tests passing for 2-pane horizontal split

## Next step
Wire pane layout into workspace snapshot serialization
EOF
)"
```

This is your cheapest context — ~200 tokens instead of re-deriving state from
the full codebase. When you commit (`jj new`), the old description stays on
the committed revision and `@` starts empty for the next progress note.

## Working rules

- Work sequentially. Don't parallelize implementation steps.
- Build features first. Write docs/ADRs only when they're needed to support a
  feature or record a decision that changes protocol shape or boundaries.
- Check `jj status` and `jj log -r @ -T description` before starting a step.
- Commit with `jj` after each coherent step, after relevant checks pass.
- Keep commits small with a clear review purpose.
- Don't create documentation infrastructure (evidence bundles, promotion
  trackers, criteria docs) until the features in WORK.md are done.
- If subagents are useful, run them as bounded read-only assistants.
- Don't inspect or copy generated Ghostty source under `target/`.

## Commit quality

Every commit message must have a **description body** (not just the subject
line). Explain *why* the change exists — what it enables, what problem it
solves, or what design tradeoff it makes. One to three sentences is enough.

**Issue references:** When a commit completes work from a GitHub issue, include
`closes #N` in the description body. This is the only reliable link between
code and motivation. Don't close issues via `gh issue close` without a
matching commit reference.

**ADRs for significant changes:** New transport layers, new execution
boundaries, new data formats, and new CLI subcommand families all require an
ADR under `docs/adr/`. If you're not sure whether a change is significant
enough, write the ADR — a short ADR is better than a missing one.

Example commit message:
```
cli: add token-authenticated tcp transport

Adds a TCP listener behind a pre-shared token for non-Unix-socket
experiments. Enables remote attach without SSH tunneling. ADR 0025
documents the threat model and why TLS is deferred.

Closes #7
```

## Documentation gardening

Docs exist to support building and using nmux. When they grow without bound
or drift from the code, they become a liability.

**What belongs where:**

| Content | Location |
| --- | --- |
| What to build next | WORK.md |
| What the project is, how to start | README.md |
| Agent rules and reading order | AGENTS.md |
| Milestone history, what's not started | docs/roadmap.md |
| How to run nmux, CLI examples | docs/running.md |
| Protocol schema contract | docs/protocol.md |
| Boundary decisions | docs/adr/NNNN-*.md |
| Upstream-blocked work | docs/upstream/*.md |
| Withheld protocol tracks | docs/protocol-futures.md |

**Gardening rules:**

- Don't append implementation status to WORK.md. It's a punch list — items
  get checked off and removed, not annotated with paragraphs.
- Don't recapitulate every behavior in milestone status blocks. 2-3 sentences
  per milestone in roadmap.md.
- Don't create new tracker docs (evidence bundles, criteria, promotion
  trackers) until WORK.md features are done.
- Keep docs concise. If a file is growing, trim before adding.
- README.md should never contain a paragraph longer than 4 lines.
- If you need to document a protocol decision, write an ADR — don't expand
  WORK.md or README.md.
- Update running.md when CLI behavior changes. Don't duplicate running.md
  content in README.md.
- Delete stale docs rather than maintaining them. If a doc isn't useful for
  building or using nmux, it shouldn't exist.

## Work queue

WORK.md is the canonical priority list. Items also come from GitHub issues.

**At the start of each session:**

1. Check for new issues: `gh issue list -l agent --author broady -R ahimsalabs/nmux --json number,title,body`
2. Triage each new item into WORK.md at the right priority (not just appended)
3. Commit: `work: triage #N into queue`
4. Then start building from the top of WORK.md

**When a work item from an issue is done:**

- The completing commit must include `closes #N` in its description body
- Remove the item from WORK.md
- Close the issue: `gh issue close N -R ahimsalabs/nmux`

Don't close issues when triaging — only when the work is actually complete
and the code passes checks. If the work is partial, leave the issue open and
note progress in WORK.md.

## Self-check (every 10 commits)

After every 10 commits, pause and reflect before continuing. Run:

```sh
jj log --no-graph -r 'ancestors(@, 10)' -T 'description.first_line() ++ "\n"'
wc -l WORK.md README.md AGENTS.md docs/roadmap.md
```

Ask yourself:

1. **Am I building features or churning docs?** If most of the last 10 commits
   are `docs:` or `build:`, stop and switch to feature work.
2. **Are docs growing?** If you've been adding to docs, trim or stop.
3. **Am I stuck in a loop?** If the last 10 commits are all in the same file
   or the same narrow area, step back and pick a different item.
4. **Is WORK.md still accurate?** Remove completed items, update priorities.
5. **Do tests pass?** Run `make check && make local-smoke` if you haven't
   recently.

If the answer to #1 is "mostly docs", the next 10 commits must be mostly
feature code. This is a hard rule.

## Checks

| Work | Commands |
| --- | --- |
| Default engine | `nix develop . -c make check` and `nix develop . -c make local-smoke` |
| Schema changes | `nix develop . -c make generate-schema`, then the matching gate |
| Opt-in libghostty-vt | `nix develop . -c make check` and `nix develop . -c make check-ghostty-vt` |

`make check` runs FlatBuffers schema validation and `cargo test --workspace`.
`make local-smoke` runs a real daemon/client smoke over a temporary socket.
`make check-ghostty-vt` runs the opt-in feature suites with `RUST_TEST_THREADS=1`.

For narrower iteration, use targeted `cargo test` inside `nix develop . -c ...`,
then run the full gate before committing.

## Licensing

- Do not copy code from GPL or AGPL sources.
- GPL/AGPL projects are prior art / references only.
- Use `oracle --prompt '...'` for deep protocol, architecture, or licensing
  questions.

## ADR conventions

When adding an ADR under `docs/adr/`:
- Include status and date metadata
- Update the status table in `docs/adr/README.md`
- ADRs are required before protocol schema changes
