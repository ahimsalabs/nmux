# ADR 0037: Event-Driven Host Output Wakeups

## Status

Accepted.

## Date

2026-05-25

## Context

ADR 0036 moved the local interactive first-display path off quiet-window
batching, but live attach still kept a background safety poll for notify-capable
hosts. That poll made read-only attach and metadata-only output reliable, but it
also meant local PTY delivery was still partly timer-driven after readiness was
available.

The daemon may have multiple live attachers observing the same pane. Some
clients are unbounded interactive streams where the right behavior is to send a
changed surface immediately. Other clients are bounded test or CLI invocations
where a cycle should represent real work: input, output, workspace changes, or a
host-completion wakeup. The daemon still owns terminal state and must apply host
bytes once before fanning out per-client surface deltas.

## Decision

Notify-capable process hosts will drive live output delivery through their host
output readiness signal. The live daemon will not run an idle quiet-window output
poll for those hosts. On readiness, it drains available host bytes, updates the
shared session surface, and fans out changed surfaces to every attached client
using each client's known surface versions.

Process-host completion is also a wakeup. A local PTY reader sends a host output
notification on EOF or read error so bounded read-only attaches can observe the
completion event without depending on periodic polling.

Batching is a client/output policy, not the synchronization primitive. Bounded
post-input paths may still wait for a short poll-quiet window after an observed
change to coalesce split writes. Non-notify hosts keep the legacy quiet polling
fallback because they have no readiness fd to wait on. Future slow-client
handling can switch individual clients from immediate streaming to coalesced
delivery without moving PTY ownership out of the daemon.

## Consequences

Local PTY output no longer depends on a background poll timer for notify-capable
hosts. Interactive, read-only, metadata-only, and multi-attacher live paths share
the same event source and fanout path.

Bounded attach cycle accounting must count only real events. Idle wakeups should
not consume cycles. Host output readiness advances all bounded attachers when it
changes the shared surface; host completion wakeups without surface bytes only
advance read-only observers, so they cannot race ahead of pending read-write
input.

The remaining quiet-window behavior is explicit: it is used for non-notify host
fallbacks and for bounded coalescing after a changed post-input frame. Benchmark
profiles should therefore show notify-capable local output latency dominated by
host readiness, input write, and surface serialization, not by idle polling.
