# ADR 0015: Cell Semantic Content

## Status

Accepted.

## Date

2026-05-22

## Context

M13 already carries OSC 133 row semantic prompt state so clients can distinguish
prompt rows from ordinary output rows. `libghostty-vt` also exposes cell-level
semantic content through its safe API: output, input, and prompt. That state is
more precise than row prompt metadata and maps naturally onto nmux `CellRun`
objects.

Full shell-integration command metadata is a larger protocol decision. Command
IDs, input/output ranges, start and end markers, exit status, and lifetime rules
need their own object model before clients depend on them.

## Decision

Add append-only `CellSemanticContent` to the FlatBuffers schema and store it on
`CellRun`.

The daemon extracts this value from backend `libghostty-vt` cells, splits runs
when adjacent cells differ only by semantic content, and serializes it in
surface snapshots, surface patches, and scrollback chunks. Existing clients and
old persisted local state default missing run semantic content to `Output`.

## Consequences

Clients can render or inspect prompt/input/output spans without replaying raw
PTY bytes or inferring shell state from text. nmux still withholds command-range
metadata until the protocol can model command lifetimes and ranges explicitly.
