# 0001: Backend-Owned Terminal State

Status: Proposed

Date: 2026-05-21

## Context

nmux is intended to be a portable Ghostty-style terminal workspace: sessions contain tabs, tabs contain panes, and clients can attach, disconnect, reconnect, and collaborate across unreliable networks and different frontend types.

The project notes in [WORK.md](../../WORK.md) point toward a backend daemon, a FlatBuffers state-sync protocol, multiple frontends, sandbox-hosted PTYs, presence, lazy scrollback fetches, and adapters for external systems. Those goals are hard to satisfy if clients are expected to reconstruct state by replaying raw PTY byte streams independently.

## Decision

`nmuxd` owns the authoritative terminal state for every pane.

PTY bytes flow into a backend terminal-state engine. Clients synchronize versioned terminal-state objects over the nmux protocol instead of treating the PTY byte stream as the product interface. The protocol should describe objects such as:

- workspace tree state
- pane surface snapshots and patches
- scrollback ranges
- cursor and mode state
- input and resize intents
- presence and actor state

Clients render authoritative state and send input/control events back to the daemon.

## Consequences

This makes reconnect, multiple frontends, spectators, read-write collaboration, lazy scrollback fetches, and protocol-first automation natural parts of the design.

It also makes terminal-state modeling a core responsibility. The protocol must handle terminal details such as grapheme clusters, wide cells, styles, hyperlinks, images, alternate screen state, cursor modes, and resize policy without prematurely freezing a simplistic cell model.

The first implementation should therefore define a small state-sync schema before building richer frontends.

## Licensing Boundary

GPL and AGPL projects may be studied as prior art or integrated through isolated adapters, but code from those projects must not be copied into the nmux core.
