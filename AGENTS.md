# AGENTS.md

Operating guide for agents working in this repository.

## Goal

Build nmux into a usable terminal multiplexer. The priority list is in
[WORK.md](WORK.md) — work through it top to bottom. Ship code that users can
run; don't substitute documentation or evidence infrastructure for features.

## Project shape

nmux is a portable terminal workspace. The daemon owns terminal state and
syncs it over FlatBuffers to clients. See [README.md](README.md) for what
works today, [WORK.md](WORK.md) for what to build next, and
[docs/roadmap.md](docs/roadmap.md) for milestone history.

Crate layout:

- `crates/nmux-proto` — FlatBuffers wire helpers and generated schema bindings
- `crates/nmux-core` — session state, process hosts, terminal engine boundary
- `crates/nmux-cli` — `nmuxd` daemon, `nmux` client, integration tests

Key docs:

- [docs/roadmap.md](docs/roadmap.md) — milestone history and status
- [docs/protocol.md](docs/protocol.md) — FlatBuffers state-sync contract
- [docs/protocol-futures.md](docs/protocol-futures.md) — withheld protocol tracks
- [docs/running.md](docs/running.md) — usage examples
- [docs/adr](docs/adr) — architecture decisions

## Working rules

- Work sequentially. Don't parallelize implementation steps.
- Build features first. Write docs/ADRs only when they're needed to support a
  feature or record a decision that changes protocol shape or boundaries.
- Check `jj status` before starting a step.
- Commit with `jj` after each coherent step, after relevant checks pass.
- Keep commits small with a clear review purpose.
- Don't create documentation infrastructure (evidence bundles, promotion
  trackers, criteria docs) until the features in WORK.md are done.
- Keep WORK.md under 50 lines. It's a punch list, not a journal.
- Keep README.md concise. No feature dump paragraphs.
- If subagents are useful, run them as bounded read-only assistants.
- Don't inspect or copy generated Ghostty source under `target/`.

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
