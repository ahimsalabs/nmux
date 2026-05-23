# ADR 0022: Hyperlink Identity Table

## Status

Accepted.

## Date

2026-05-22

## Context

`libghostty-vt` can prove OSC 8 hyperlink presence on terminal cells, and nmux
already preserves that presence as bit 0 in `CellRun.flags`. The public schema
also has `CellRun.hyperlink_id`, but nmux deliberately leaves it at zero today
because there is no protocol object that defines the referenced URI, OSC 8 id,
parameters, range ownership, or lifetime.

Inventing opaque IDs without a table would make snapshots and scrollback chunks
ambiguous: clients could tell that a run references "some link" but not which
URI or whether two runs refer to the same link. Encoding URIs directly into
each run would duplicate data and make row patches expensive.

## Decision

Add hyperlink identity through an explicit table before setting nonzero
`CellRun.hyperlink_id` values.

The first schema shape should treat hyperlink IDs as local to the pane surface
or scrollback object that carries the table:

- `hyperlink_id = 0` continues to mean no table-backed hyperlink identity;
- `CellRun.flags` bit 0 continues to mean backend-observed hyperlink presence,
  even before or without a table-backed identity;
- full `PaneSurfaceSnapshot` objects and `ScrollbackChunk` objects carry the
  complete hyperlink table needed by their rows;
- each table entry carries at least a numeric ID, target URI, and optional OSC
  8 identifier/parameter strings when the backend can expose them safely;
- persisted client state must store both run references and the matching table
  so current-surface reattach can render or inspect cached links without raw
  PTY replay.

`PaneSurfacePatch` does not currently carry a hyperlink table or table diff. The
first implementation should therefore emit `ReplaceRows` patches with nonzero
`hyperlink_id` only when every referenced ID already exists in the client's
cached table for the patch base version. If new hyperlink identities appear in
changed rows and no patch table diff exists, the daemon must force
`PatchKind::FullRefreshRequired` so attach or reattach recovers with a complete
snapshot.

A later append-only schema change may add patch-scoped hyperlink table diffs
when the table lifetime and client cache rules are proven.

## Consequences

Clients get stable, inspectable hyperlink identity without parsing OSC 8 escape
sequences or replaying PTY bytes. Snapshots and scrollback chunks remain
self-contained: all nonzero hyperlink IDs in their rows resolve within the same
object.

The presence flag remains valuable. It lets the current implementation preserve
backend-observed link regions before the identity table lands, and it gives
clients a conservative signal if a future backend can detect link presence
without exposing a URI.

The patch rule is intentionally conservative. It may produce more snapshots for
new hyperlink identities until patch table diffs exist, but it avoids dangling
IDs and keeps the state-sync protocol deterministic.
