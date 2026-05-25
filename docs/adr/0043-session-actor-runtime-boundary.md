# ADR 0043: Session Actor Runtime Boundary

## Status

Accepted.

## Date

2026-05-25

## Context

The live daemon currently keeps one synchronous session loop and uses Tokio only
at the live client socket edge. Accepted live clients split socket reads and
writes into async tasks, but the daemon still owns one mutable `Session`,
terminal engines, process host state, client inventory, resize constraints, and
fanout sequencing inside the local serve loop.

nmux needs lower end-to-end latency while remaining maintainable with dozens of
client connections and dozens of sessions. It also needs a deterministic path
for replay-oriented tests. A full async rewrite of the session model would make
terminal-state ownership and `libghostty-vt` integration harder, especially
while upstream ownership APIs may still change.

## Decision

Use one daemon-level Tokio runtime for transport, timers, supervision, and
async effects. Inside that runtime, each nmux session is owned by one
single-owner session actor task. The daemon registry maps `session_id` values
to session handles and routes attach, control, and lifecycle requests to the
right actor.

Each session actor owns the full mutable per-session bundle:

- `Session`
- pane terminal engines
- local process host and PTY handles for that session
- attached client inventory and resize constraints
- per-client known surface versions and sequence/accounting state
- session-owned timers, counters, and deterministic IDs

The actor contains a deterministic synchronous `SessionCore` reducer. The async
shell receives work, runs the scheduler, assigns the accepted canonical order,
calls `SessionCore`, and executes returned effects. Tokio readiness does not
directly define canonical session event order.

Ingress is per source, not one shared global queue. Client input, pane/PTY
output, timers, and lifecycle/control requests use bounded lanes. The scheduler
preserves order within each source, may choose cross-source order for fairness,
and emits one canonical accepted event stream. Administrative and lifecycle
commands such as detach, kill, close-pane, and shutdown use a protected
high-priority lane so noisy clients or panes cannot prevent cleanup.

For the local PTY path, use dedicated reader threads per active PTY or pane
host. Reader threads do not mutate terminal state. They read small chunks and
hand bytes to the session actor through a small blocking bounded handoff. When
the handoff is full, the reader blocks or pauses reading so the kernel PTY
buffer naturally backpressures the child process. PTY bytes are not silently
dropped.

Keep `libghostty-vt` engine objects strictly session-owned and non-shared until
upstream ownership semantics stabilize. Do not introduce parser worker ownership
splits, `Arc<Mutex<libghostty-vt>>`, or shared engine references across tasks
without a later decision.

## Consequences

The daemon can host many sessions in one process, like a tmux server hosting
many sessions on one socket. Users who need process-level isolation can run a
separate daemon with a separate socket.

One busy session should not block unrelated sessions. Within a session, one
busy client or pane should not prevent detach, kill, shutdown, or other
protected lifecycle work from making progress.

The session actor remains the only owner of terminal/session mutation, so
multi-client fanout continues to come from one authoritative session state. The
runtime can use async transports and background work without making terminal
state shared.

The first migration should be internal and should not change the FlatBuffers
wire schema. Introduce `SessionCore`, internal session events/effects, and
deterministic tests before moving larger live paths through the actor boundary.

Timer semantics are session-owned. Tokio timers wake the actor near deadlines,
but the scheduler checks injected time and accepts explicit timer events.
Session core and scheduler code must receive time from the shell or test
harness rather than calling wall-clock APIs directly.

Replay and bug-report tracing should build on the canonical accepted event
stream defined by this boundary. The concrete durable replay container is a
separate pending decision.
