# ADR 0024: Native VT Source Policy Criteria

## Status

Superseded by ADR 0034.

## Date

2026-05-23

## Context

ADR 0034 later chooses the source policy for the default path: local development
may use the `libghostty-vt-sys` pinned fetch or `GHOSTTY_SOURCE_DIR`, while the
Nix package supplies a pinned Nix-fetched Ghostty source. This ADR records the
earlier promotion criteria.

ADR 0023 kept `libghostty-vt` opt-in after M13 until native build cost, regular
CI, non-Nix/toolchain provisioning, source-fetch policy, packaging, and workflow
consequences were deliberately accepted. ADR 0034 later accepts those tradeoffs
for the default path. The original opt-in path was useful for local correctness
work: `libghostty-vt-sys` fetched the pinned Ghostty source unless
`GHOSTTY_SOURCE_DIR` pointed at a local checkout, and nmux recorded the active
mode in promotion evidence.

That current behavior is not enough by itself for a default engine, regular CI
requirement, or packaged binary baseline. A default or packaged build needs a
clearer source policy so contributors and release automation know whether they
are using a network fetch, a pre-fetched checkout, a mirror, vendored source, or
a native library artifact.

## Decision

This ADR required any later promotion beyond the opt-in path to choose a
specific native VT source policy and prove these criteria:

- The source mode is explicit in docs, CI logs, and package provenance.
- The build is reproducible from `Cargo.lock` plus documented source inputs.
- CI behavior is stated for cache hits, cache misses, and network failures.
- Offline or pre-fetched behavior is tested or explicitly rejected as a
  non-goal for that promotion.
- Local Git URL rewrite rules cannot silently change the fetched source.
- `GHOSTTY_SOURCE_DIR` overrides, if supported, require an existing directory
  and are recorded in evidence.
- Release artifacts record the Ghostty source mode, source identity, nmux
  `Cargo.lock` hash, native runtime-library artifacts, and dependency tree.
- License review covers the chosen source path and does not copy GPL or AGPL
  code into nmux.
- Generated Ghostty build output under `target/` remains build output, not nmux
  source material or implementation guidance.

ADR 0034 is the later ADR that satisfies this requirement for the default path:
local development may use the `libghostty-vt-sys` pinned fetch or
`GHOSTTY_SOURCE_DIR`, while Nix package builds provide a pinned Nix-fetched
Ghostty source through `GHOSTTY_SOURCE_DIR`.

## Consequences

The project can keep collecting useful local promotion evidence without treating
that evidence as a policy decision. Evidence rows should say whether they used
the pinned fetch path or `GHOSTTY_SOURCE_DIR`, and packaging samples should keep
recording source mode and package provenance.

The likely policy choices remain open: pinned network fetch with CI/cache
controls, mirrored or vendored source with update rules, pre-fetched local
source via `GHOSTTY_SOURCE_DIR`, or a platform package/artifact cache for the
native VT library. The next promotion ADR should choose one, document its
failure modes, and update contributor and packaging guidance accordingly.

## Licensing And Compatibility

This decision does not import or vendor Ghostty source. It only records the
criteria for a later source-policy decision. Do not copy GPL or AGPL terminal
emulator code into nmux, and do not copy generated Ghostty build output from
`target/` into the repository.
