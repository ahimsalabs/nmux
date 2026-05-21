# 0010: herdr Integration Boundary

Status: Accepted

Date: 2026-05-21

## Context

M9 requires any herdr integration to stay outside the nmux core repository and behind the nmux protocol. The project notes call this an AGPL membrane: `nmuxd` may interact with an `nmux-herdr-adapter`, but the MIT/Apache-oriented core must continue to speak nmux-owned protocol objects and must not absorb herdr implementation material.

This is stricter than a normal adapter boundary because licensing is part of the architecture. GPL and AGPL projects can be prior art, references, or external adapter targets, but their code must not be copied into this repository.

The existing nmux shape still applies:

- `nmuxd` owns normalized workspace, tab, pane, terminal-state, attach, reconnect, presence, permission, input, resize, and lifecycle semantics;
- clients speak nmux FlatBuffers state objects;
- adapter-specific behavior is isolated behind process or repository boundaries.

## Decision

herdr integration will stay outside the nmux core repository and outside the nmux public client protocol.

If herdr integration is built, it should live in a separate `nmux-herdr-adapter` process and, if licensing requires it, a separate repository. `nmuxd` may communicate with that process through an nmux-owned adapter boundary. Clients must continue to attach to `nmuxd` and receive normal nmux FlatBuffers messages.

The herdr adapter may own herdr-specific work:

- discovering or connecting to herdr-managed workspaces;
- translating herdr sessions, panes, agents, tasks, or lifecycle events into nmux-owned concepts;
- forwarding nmux input or control intents where policy allows;
- adapting herdr-specific failures into nmux lifecycle or error reports.

`nmuxd` must remain the owner of client-facing behavior:

- durable normalized workspace, tab, and pane identity;
- terminal surface versions and scrollback synchronization;
- attach/reconnect compatibility;
- presence and permission policy;
- input and resize policy;
- public schema evolution.

The public nmux schema must not become a herdr protocol. herdr messages, IDs, terminal-state assumptions, lifecycle semantics, UI assumptions, and agent/task concepts can be translated into nmux-owned objects only after their shape is independently modeled.

## Consequences

No herdr dependency should be added to `nmux-core`, `nmux-cli`, `nmux-proto`, `schema/nmux.fbs`, or in-repository tests.

The first implementation after this ADR should be a pure nmux-owned mapping test, if one is needed at all. That test should define local adapter inventory structs, prove stable normalized ID mapping into the existing nmux session/tree model, and avoid launching herdr or importing any herdr code.

Any live integration should be treated as an external adapter integration. Adapter crashes, incompatibilities, or licensing constraints should surface as adapter lifecycle/error states instead of changing core client semantics.

## Licensing

Do not copy GPL or AGPL code into this repository.

Do not copy herdr source code, protocol internals, tests, internal data structures, UI assumptions, or implementation-derived behavior into nmux core code or docs as implementation source material.

herdr may be treated as prior art or as a separately distributed adapter target. The nmux core must remain independently implemented and communicate across an nmux-owned process/protocol boundary unless the project owner explicitly changes the licensing plan.
