# Source Fetch Policy

This repository keeps backend `libghostty-vt` extraction opt-in. The current
source-fetch policy is acceptable for local correctness work, but it is not yet
enough to make `libghostty-vt` the default engine, a regular CI requirement, or
a packaging baseline.

## Current Opt-In Behavior

- `nmux-core` depends on `libghostty-vt = "=0.1.1"` only through the optional
  `libghostty-vt` Cargo feature.
- `libghostty-vt-sys` is pinned through `Cargo.lock` and fetches a pinned
  Ghostty source tree for the native VT library unless `GHOSTTY_SOURCE_DIR`
  points at an existing local Ghostty checkout.
- `make check-ghostty-vt` sets `GIT_CONFIG_GLOBAL=/dev/null` so local Git URL
  rewrite rules do not alter that HTTPS fetch.
- `make check-ghostty-vt` also validates `GHOSTTY_SOURCE_DIR` before running the
  native build: when the variable is set, it must point at an existing readable
  source directory; when it is unset, the build uses the pinned
  `libghostty-vt-sys` fetch path.
- `make toolchain-info` and `make promotion-sample` report the source mode as
  `ghostty_source_mode=pinned-fetch` or `ghostty_source_mode=local`, along with
  whether the local directory is present.
- Generated Ghostty build output under `target/` is build output, not nmux
  source material. Do not inspect it as implementation guidance or copy it into
  this repository.

For local opt-in validation, either allow the pinned upstream fetch or provide a
local Ghostty checkout with `GHOSTTY_SOURCE_DIR`. Record which path was used
when adding default-engine promotion evidence.

## Promotion Blockers

Before `libghostty-vt` can become the documented default engine, a regular CI
requirement, or a packaged binary dependency, a later ADR must choose one of
these source policies and record the consequences:

- pinned network fetch with CI/cache controls;
- vendored or mirrored source with update and license-review rules;
- pre-fetched local source via `GHOSTTY_SOURCE_DIR`;
- platform package or artifact cache for the native Ghostty VT library.

That decision also needs offline-build behavior, cache invalidation, provenance,
and binary packaging expectations. Until then, the default engine remains
`interim`, and `make check` remains independent of the native Ghostty/Zig build.
See [packaging.md](packaging.md) for the matching binary distribution questions.
