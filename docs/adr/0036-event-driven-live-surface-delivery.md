# ADR 0036: Event-Driven Live Surface Delivery

## Status

Accepted.

## Date

2026-05-25

## Context

Live local attach previously treated PTY output as a batch to collect until a
quiet timeout elapsed before sending surface updates. That preserved compact
updates for bursty output, but it put the interactive input-to-display path
behind a timer even when the PTY reader had already delivered output.

The daemon owns terminal state and multiple live attachers can observe the same
pane at different surface versions. Any lower-latency delivery policy must keep
one authoritative session surface and fan out versioned updates per attacher,
instead of letting each client race its own PTY reader or terminal state.

## Decision

Live attach will stream the first changed surface produced by interactive input
as soon as PTY output readiness is observed. The daemon drains available output
from the target pane first, applies it to the shared session surface, then writes
the resulting versioned surface update to each attached client using that
client's `known_surface_versions`.

Quiet-window batching remains valid for background output, startup/attach
snapshot collection, noisy scrollback-producing commands, and future
backpressure handling for slow clients. It is not the default local interactive
latency path.

## Consequences

Interactive local echo no longer waits for the post-input quiet timeout before
the first display update. A later output event can deliver the next surface
version, so applications that write in chunks may produce more frames than the
old quiet-batched path.

Multi-attacher behavior stays centralized: the daemon applies PTY bytes once,
then fans out protocol frames to each client according to that client's known
surface version. Slow-client policy can become adaptive later by marking a
client as batching/coalescing without changing terminal-state ownership.

Benchmarking should distinguish first-display latency from quiet/coalesced
latency. The first-display metric is the one that matters for local interactive
keystroke response; quiet/coalesced latency remains useful for high-volume
output behavior.
