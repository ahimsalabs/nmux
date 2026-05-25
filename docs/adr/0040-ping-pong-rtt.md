# ADR 0040: Ping-Pong RTT Metrics

## Status

Accepted.

## Date

2026-05-25

## Context

The live client status bar reports local render metrics, but it does not show
transport health. Measuring input-to-display latency is useful, but it includes
PTY forwarding, application behavior, terminal-engine processing, surface
encoding, client decode, and rendering. It does not isolate client/server
round-trip latency.

nmux also needs a side-effect-free control signal that can support connection
health display and later connection inventory without writing to panes or
changing visible presence.

## Decision

Add `Ping` and `Pong` protocol frames.

A live client sends `Ping` with an `actor_id` and monotonically increasing
`ping_seq`. The daemon replies immediately with `Pong`, echoing those fields.
The client measures elapsed monotonic time between sending the ping and reading
the matching pong, then reports the most recent RTT in the status bar.

Ping/pong frames are control-plane frames. They do not attach or detach
presence, do not change focus, do not participate in resize policy, and never
touch process hosts, PTYs, terminal engines, or surface caches.

## Consequences

The RTT metric reflects socket write/read, scheduling, FlatBuffers
serialization, and daemon frame dispatch overhead. It intentionally excludes
terminal processing and rendering.

The same frame family can later support an online connection list by recording
per-client last-ping or last-pong activity in daemon connection state. That
inventory is a separate feature; this ADR only adds the protocol primitive and
client-side RTT display.
