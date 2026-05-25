# ADR 0044: Deterministic Replay Trace Container

## Status

Pending.

## Date

2026-05-25

## Context

ADR 0030 accepted the current JSON-lines live recorder as an experimental,
human-inspectable debug and export seed format. It explicitly did not promote
that format as nmux's long-term archival recording format.

ADR 0043 defines a future runtime boundary with one session actor, a
deterministic `SessionCore`, per-source scheduling, and one canonical accepted
event stream. That boundary can support deterministic replay tests, bug-report
traces, asciinema-like playback, and durable session recordings, but the
storage format should not be accepted before implementation experience proves
the exact shape.

## Pending Decision

The intended long-term replay system should record canonical input-side session
events as the source of truth and record emitted output as a secondary output
trace.

Canonical events include client input, paste chunks, control mutations, PTY
byte chunks, timer firings, lifecycle changes, and host events after the
session scheduler accepts them. Emitted output trace records include structured
workspace, surface, scrollback, presence, error, and control-output records.
Raw protocol frame bytes may be captured in a full-fidelity debug mode, but
they are not the primary replay source of truth.

Snapshots should capture deterministic session and terminal state, including
the replay event index and timing information they cover. Snapshots should not
capture live sockets, Tokio tasks, channels, OS process handles, PTY file
descriptors, or transient writer queue contents.

Replay records should separate compact required fields from optional diagnostic
metadata. The required core should stay small enough for durable recording and
asciinema-like playback. Optional metadata levels may include arrival time,
accepted time, wall-clock UTC, queue depths, lane names, client labels, byte
counts, raw frame bytes, and scheduler diagnostics.

The recording time model should include both deterministic/session time and
presentation time:

- session monotonic time for deterministic reducer behavior and timer
  decisions;
- recording-relative presentation time for asciinema-like playback;
- optional wall-clock UTC metadata for indexing and diagnostics.

The durable container should be chunked rather than one monolithic compressed
stream. Candidate chunk types include headers, snapshots, canonical event
segments, output-trace segments, optional metadata segments, and index chunks.
Per-chunk compression allows partial replay, bug-report export, corruption
containment, and later compaction. Durable replay should likely use low-level
zstd compression, while hot in-memory traces can remain uncompressed or use LZ4
on export.

Persistence should run through a background writer. The session actor records
to an in-memory ring immediately and sends durable records to a bounded writer
queue when recording is enabled. Normal production recording should prioritize
live latency and mark or stop degraded recordings if the writer cannot keep up.
Strict tests may use a mode that backpressures until records are durable.

## Consequences

This ADR is pending and does not replace ADR 0030 yet. JSON-lines recording can
remain the experimental user-facing replay/export path until this container is
implemented and promoted.

The accepted runtime boundary should nevertheless avoid choices that would
prevent this replay model: session-visible time and IDs should be injected or
deterministic, emitted effects should be representable as data for tests, and
the scheduler should produce a canonical accepted order independent of Tokio
`select!` races.

Before accepting this ADR, nmux should build enough of `SessionCore`,
scheduler tests, in-memory trace export, and at least one durable recording
prototype to validate overhead, file size, and test usefulness.
