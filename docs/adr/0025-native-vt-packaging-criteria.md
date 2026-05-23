# ADR 0025: Native VT Packaging Criteria

## Status

Accepted.

## Date

2026-05-23

## Context

ADR 0023 keeps `libghostty-vt` opt-in until native build, CI, source-fetch,
packaging, and workflow costs are accepted deliberately. Local M14 evidence now
shows that nmux can build opt-in release binaries, stage wrapper scripts and
native `libghostty-vt` runtime libraries, archive that layout, verify binary
versions from the extracted archive, and run a packaged daemon/client smoke
test.

Those samples prove a useful local layout, but they are not a release packaging
decision. They do not define supported target triples, install paths, platform
package formats, signing or notarization, runtime-library linkage rules,
update channels, or release provenance requirements.

## Decision

A future ADR that makes `libghostty-vt` a default engine, a regular CI
requirement, or a packaged binary baseline must choose a native-VT packaging
policy and prove these criteria:

- Supported targets are explicit, including whether Darwin, Linux, or other
  platforms are in scope for the first promotion.
- The package format is explicit for each target: archive, platform package,
  source-only distribution, or another documented artifact.
- Runtime-library strategy is explicit: bundled dynamic library with wrapper or
  rpath, static linkage, platform package dependency, or native-library artifact
  cache.
- The shipped binary behavior is explicit: whether `nmuxd --terminal-engine
  libghostty-vt` is enabled for users, reserved for developer builds, or made
  the daemon default.
- Release checks map to concrete commands, including default-engine checks,
  opt-in terminal-correctness checks, packaging layout/provenance/archive
  checks, and at least one packaged runtime smoke on every supported target.
- Artifacts include provenance that records source mode, `Cargo.lock` hash,
  dependency tree, staged file hashes, dynamic dependency output where
  available, and native runtime-library artifacts.
- Signing, notarization, checksum publication, and update-channel expectations
  are either implemented or explicitly rejected as out of scope for that
  promotion.
- Failure recovery is documented for missing runtime libraries, unsupported
  platforms, invalid source inputs, and package smoke failures.
- License review covers the packaged native VT dependency path and does not
  copy GPL or AGPL code into nmux.

Until a later ADR satisfies those criteria and chooses a concrete packaging
policy, packaged/default builds keep `interim` as the normal path and
`libghostty-vt` remains opt-in correctness work. Local packaging samples are
evidence, not release artifacts.

## Consequences

The current archive layout remains useful for promotion evidence and local
experiments. It can evolve into a release format later, but only after the
runtime-library, target-support, source-policy, provenance, and signing/update
questions are answered together.

Packaging evidence rows should keep recording target platform, source mode,
cache state, artifact hashes, runtime smoke result, and any dynamic dependency
inspection output. CI or release automation should not treat a local archive
sample as a supported package unless a later ADR promotes it.

## Licensing And Compatibility

This decision does not publish or bless a native VT package format. It only
records the criteria for a later packaging decision. Do not copy GPL or AGPL
terminal emulator code into nmux, and do not copy generated Ghostty build output
from `target/` into the repository.
