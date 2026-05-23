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

## Manual Promotion Sample

The same workflow exposes a manual `workflow_dispatch` job for promotion
evidence:

```sh
nix develop . -c make promotion-sample
```

That job is intentionally manual. It runs the default gate plus the opt-in
`libghostty-vt` gate, records toolchain/source-fetch context in the log, and
times the inner `make check-all` run. Copy passing or failing results into
[default-engine-promotion.md](default-engine-promotion.md) with runner, cache,
source-fetch, and flake context before using them as promotion evidence.

## Nix Setup

CI installs Nix with `cachix/install-nix-action@v31`, which the action README
documents for Linux and macOS runners. The workflow does not configure a
project binary cache yet; cache behavior remains explicit open promotion work.
