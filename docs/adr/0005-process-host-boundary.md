# 0005: Process Host Boundary

Status: Proposed

Date: 2026-05-21

## Context

nmux panes will eventually be backed by PTYs running on different kinds of hosts: local processes, containerized commands, and sandboxed execution environments. M6 in [docs/roadmap.md](../roadmap.md) requires process hosting to be isolated behind a host abstraction.

The current prototype uses static pane state and has no real process lifecycle. Adding PTY spawning directly to the session model would couple terminal-state synchronization to one host implementation too early, and would make later sandbox/container support a protocol and core refactor.

[WORK.md](../../WORK.md) also calls out `HostSpec` as part of the eventual protocol shape, but the immediate exit evidence is narrower: represent local and sandbox host choices without changing the protocol core.

## Decision

Introduce process hosting as a core runtime boundary before wiring real PTYs.

A pane process lifecycle is mediated by a host interface. The interface accepts a host specification, starts or stops a pane process, and returns a small host-owned process record. Host kinds include local, container, and sandbox choices from the start, even if only a planning or recording implementation exists initially.

The session tree and terminal-state protocol remain independent from process launch mechanics. A frontend should not need to know whether a pane is local, container-backed, or sandbox-backed in order to render backend-owned terminal state.

Do not add real PTY spawning, container runtime calls, or sandbox integration until the host boundary has lifecycle tests. Do not copy GPL or AGPL adapter code into this boundary; future integrations stay behind process or repository boundaries unless the project licensing plan changes.

## Consequences

The first implementation can live inside `nmux-core` as plain Rust types and tests. It should prove that:

- a host spec represents local and sandbox choices;
- starting a pane process goes through a host implementation;
- duplicate starts and missing stops are handled explicitly.

Public FlatBuffers schema changes can wait until the host metadata needs to cross the client/server protocol. When `HostSpec` becomes public, it should be added append-only and without making pane rendering depend on host internals.

The local daemon can continue serving static pane state until a later step connects a concrete local PTY host to the session runtime.

## Compatibility

This decision does not change the current wire protocol. Future schema changes must preserve the append-only compatibility rules already used for envelope bodies and table fields.
