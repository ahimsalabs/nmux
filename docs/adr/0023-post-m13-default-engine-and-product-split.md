# ADR 0023: Post-M13 Default Engine And Product Split

## Status

Accepted.

## Date

2026-05-23

## Context

M13 proved the opt-in backend `libghostty-vt` extraction path deeply enough for
the current nmux state-sync model. PTY bytes can enter daemon-owned
`libghostty-vt` state, while clients still receive nmux-owned
`PaneSurfaceSnapshot`, `PaneSurfacePatch`, `ScrollbackChunk`, `AttachStatus`,
`InputEvent`, `ResizeIntent`, and `Error` frames. The opt-in path now covers
cursor state, modes, color state, metadata, styled rows, scrollback, dirty
state, row hashes, Kitty placeholders, hyperlink presence, structured input
gating, and reconnect/cache behavior through full feature-enabled tests.

That does not mean `libghostty-vt` should silently become the default engine.
ADR 0018 deliberately kept the native Ghostty/Zig build out of regular
development and `make check` until native build cost, regular CI,
non-Nix/toolchain provisioning, source-fetch policy, packaging, and developer
workflow costs are intentionally accepted.

M13 also identified several real terminal-state domains that should not be
folded into the extraction milestone by inertia: wired hyperlink identities,
image placement and pixel data, richer damage objects, semantic command
lifecycle metadata, physical-key/text-event forwarding, and frontend Ghostty
renderer hydration.

## Decision

Treat M13 as complete for the opt-in backend extraction milestone, not as a
default-engine promotion.

Keep the default daemon engine `interim` and keep `libghostty-vt` opt-in until a
future decision accepts the native build cost, regular CI, non-Nix/toolchain
provisioning, source-fetch policy, packaging, and developer-workflow costs.
`make check-ghostty-vt` remains the required gate for changes that touch the
opt-in engine, feature-sensitive attach/reconnect behavior, cached state, or
daemon-owned structured input.

Split post-M13 work into explicit tracks:

- Default-engine promotion readiness: measure and document native build time,
  CI behavior, non-Nix toolchain provisioning, source-fetch or vendoring policy,
  and packaging implications before changing defaults.
- Frontend Ghostty renderer hydration: keep this upstream/API-driven and do not
  reintroduce client-side raw PTY replay to make a frontend render.
- Protocol object expansion: require a focused ADR and compatibility plan before
  adding schema fields for wired hyperlink IDs, image placement or pixel data,
  richer damage metadata, semantic command lifecycle data, or physical-key and
  text-event forwarding. Track the pre-schema questions in
  [Future Protocol Tracks](../protocol-futures.md).
- Local usability spine: preserve attach, reconnect, live streaming, scrollback
  fetches, cached state, and daemon-owned structured input while those tracks
  progress.

## Consequences

The project can stop treating every future terminal feature as unfinished M13
work. The backend extraction path is available for correctness work, but the
default user and contributor path remains fast and explicit about interim
renderer limitations.

Future schema additions must carry their own compatibility rationale instead of
using M13 as blanket justification. Upstream/API blockers belong in
`docs/upstream/` until local implementation can proceed without duplicating
terminal state or replaying raw PTY bytes on clients.

## Licensing And Compatibility

This decision does not change licensing posture. Do not copy GPL or AGPL code
into nmux. `libghostty-vt` remains an optional MIT-compatible dependency path,
and generated or vendored Ghostty build output under `target/` is not nmux
source material.
