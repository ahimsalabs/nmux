# ADR 0042: Session-Driven Daemon Lifetime

## Status

Accepted.

## Date

2026-05-25

## Context

nmux started with bounded serving modes such as `--one-shot`, `--live`, and
`--live-clients N` because they made early integration tests and smoke tests
predictable. Those modes are still useful as harness controls, but they are the
wrong abstraction for normal multiplexer lifetime.

A terminal multiplexer daemon owns sessions, panes, and PTYs. Clients are
disposable views that may attach, detach, crash, reconnect, or run short control
commands. Treating client count as the daemon lifetime conflates normal view
churn with session destruction.

tmux follows the same boundary: attached clients affect view state and sizing,
but server exit is driven by explicit shutdown, remaining sessions, configured
empty/unattached policies, and pending jobs.

## Decision

Production nmux daemon lifetime is session driven:

- A normal live daemon serves until explicit session shutdown.
- Attach client disconnect is not a daemon shutdown condition.
- Control and health-probe clients are transient commands, not session owners.
- Pane/process exit is pane state and may later affect session policy, but it is
  separate from client socket closure.
- Bounded client counts remain available only as harness limits for tests,
  benchmarks, and one-shot workflows.

Represent this in code with an explicit connection limit: bounded modes use
`ConnectionLimit::Bounded(N)`, while `--live-forever` uses
`ConnectionLimit::Unbounded`. Avoid `usize::MAX` as a sentinel for production
daemon lifetime.

## Consequences

Future lifecycle work should add named session policies such as explicit kill,
empty-session exit, or idle timeout rather than increasing client counters.

`--live-clients N` remains valid for tests that need a deterministic daemon
exit. It should not be presented as the normal way to keep a user workspace
alive.
