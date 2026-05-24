# 0030: Live Session Recording Format

Status: Accepted

Date: 2026-05-24

## Context

Issue #3 asked for playback research, including timestamp metadata,
differential recording, presence capture, video export, and possible web
embedding. The first implementation added `nmux --live --record PATH` and
`nmux replay PATH`.

The current file is newline-delimited JSON. Each line is an event with
`elapsed_ms` plus an attach, presence, surface, or detach payload derived from
the live client renderer. This is useful for inspection and tests, but it is
not the same thing as a durable protocol recording format.

## Decision

Keep the current JSON-lines recorder as an experimental, human-inspectable
debug and export seed format. Do not promote it as nmux's long-term archival
recording format.

Before recording becomes a compatibility promise, define a versioned recording
container. That future format should include:

- a file header with nmux recording format version, producer version, and
  feature flags;
- explicit stream metadata for workspace, surface, scrollback, presence, and
  timing channels;
- monotonic timestamps with a documented time base;
- enough object-version data to replay backend-owned state without raw PTY
  byte replay;
- an index or chunk table for random access and partial replay;
- privacy/security guidance for environment values, command text, presence
  identity, scrollback, and terminal output.

JSON-lines remains acceptable for local debugging and prototype exporters
because it is append-friendly, easy to diff, easy to inspect, and does not add
a new decoder dependency. It should not gain undocumented compatibility
semantics by accident.

Before adding more JSON-lines event kinds or documenting it for user-facing
archive use, add interim guardrails: a mandatory header event with format name,
format version, producer version, capture policy, and time base; monotonic event
sequence numbers; explicit unknown-event behavior; and size limits for decoded
payloads.

## Consequences

Current `nmux replay` is best-effort user-facing replay of recorded rendered
surface text. It is not an authoritative protocol replayer, not a restore
mechanism, and not a storage format for long-lived session archives.

Future web embed, video export, or deterministic playback work should either
write the versioned container directly or include a documented migration path
from the current JSON-lines events. Tests may keep using JSON-lines while the
feature is experimental, but test names and docs should avoid implying archival
stability.

Do not encode opaque FlatBuffers blobs inside JSON-lines as the main archival
design. That would keep JSON's indexing and size costs while losing the main
benefit of human-readable diagnostics.

## Licensing Notes

This decision is nmux-specific data-format policy. It does not copy or depend
on GPL or AGPL implementation code.
