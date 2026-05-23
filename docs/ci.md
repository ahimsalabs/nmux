# CI Notes

The repository has a GitHub Actions workflow at
[.github/workflows/check.yml](../.github/workflows/check.yml).

## Required Default Gate

Pull requests and pushes to `main` run the default-engine gate:

```sh
nix develop . -c make toolchain-info
nix develop . -c make check
```

This keeps regular CI on the default `interim` engine. A green required CI run
does not claim `libghostty-vt` correctness and does not change the default
engine decision.

## Manual Promotion Evidence

The same workflow exposes a manual `workflow_dispatch` job for promotion
evidence:

```sh
nix develop . -c make promotion-local-sample
```

That job is intentionally manual. It records source-fetch provenance, runs the
default gate plus the opt-in `libghostty-vt` gate, times the inner
`make check-all` run, and produces a verifiable native-VT package archive. Copy
passing or failing results into
[default-engine-promotion.md](default-engine-promotion.md) with runner, cache,
source-fetch, packaging, and flake context before using them as promotion
evidence.

Before this manual job can become a required or regular native-VT CI gate, a
later ADR must satisfy
[ADR 0026](adr/0026-native-vt-ci-promotion-criteria.md), including runner
matrix, cache behavior, source-fetch behavior, packaging claims, runtime, and
failure-triage expectations.

## Nix Setup

CI installs Nix with `cachix/install-nix-action@v31`, which the action README
documents for Linux and macOS runners. The workflow does not configure a
project binary cache yet; cache behavior remains explicit open promotion work.
