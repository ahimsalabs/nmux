# 0002: Rust Core Runtime

Status: Proposed

Date: 2026-05-21

## Context

nmux is a portable Ghostty-style terminal workspace. The current architecture centers on `nmuxd`, a FlatBuffers state-sync protocol, backend-owned terminal state, multiple clients, reconnect, sandbox/process-host boundaries, adapters, and future Ghostty/libghostty integration.

[WORK.md](../../WORK.md) identifies libghostty as the canonical terminal-state and snapshot engine. The roadmap's next implementation step is to add a minimal codegen/build path for the chosen implementation language and then build the first local daemon/client skeleton.

The repository currently has an initial Go FlatBuffers bootstrap: `go.mod`, `go.sum`, generated files under `internal/protocol/flat/protocol`, and Makefile targets for Go code generation and tests. That bootstrap validated the schema direction, but it should not decide the implementation runtime by accident.

An oracle architecture check was run on 2026-05-21 for the Go-vs-Rust core decision. Its recommendation was to migrate the nmux core/daemon to Rust while treating the existing Go FlatBuffers bootstrap as temporary tooling until the Rust workspace is ready.

## Decision

Use Rust as the nmux core implementation runtime.

The Rust core includes:

- `nmuxd`
- protocol encode/decode bindings
- session/tab/pane state ownership
- local daemon/client skeletons
- process-host abstractions
- adapter process boundaries

FlatBuffers remains the protocol serialization format. [schema/nmux.fbs](../../schema/nmux.fbs) remains the language-neutral contract between daemon, clients, adapters, and future frontends.

## Rationale

Rust is the better fit for the repository's stated direction:

- nmux needs a long-lived multi-client daemon with explicit ownership of terminal state.
- The protocol is state-sync-first, with versioned snapshots, patches, input events, resize intents, and later scrollback/presence objects.
- The daemon will sit close to PTYs, process lifecycle, local sockets, network transports, and adapter boundaries.
- Ghostty/libghostty alignment is central to the project shape, and Rust gives the core a strong systems-runtime posture for FFI and low-level integration work.
- Rust keeps memory ownership, concurrency, and protocol object lifetimes explicit as the daemon grows.

Go remains useful for quick service prototypes and tooling, but nmux should not become a Go daemon by accident when the long-term center of gravity is a native terminal-state engine with networking attached.

This decision does not change ADR 0001. It supports it: Rust is the runtime for implementing backend-owned terminal state.

## Consequences

The roadmap should treat Rust as the chosen implementation language for M1 and later core work.

The development environment should include Rust tooling, and `make check` should move from Go test targets to Rust build/test targets once the Rust workspace replaces the temporary bootstrap.

Generated FlatBuffers bindings should move from Go to Rust for the core. Other language bindings may still be generated later for clients, compatibility tools, or SDKs.

Rust becomes the default language for core daemon, CLI, local client skeleton, protocol tests, and adapter-boundary definitions. Go should not be used for new core implementation work unless a later ADR changes this decision.

## Migration Notes

The existing Go FlatBuffers bootstrap should be replaced in a later commit, not mixed into this ADR-only change.

A follow-up implementation commit should:

- add `Cargo.toml` and `Cargo.lock` for the Rust workspace or crate layout
- update `flake.nix` to provide Rust tooling
- change `Makefile` schema generation from Go codegen to Rust-oriented generation or Rust build integration
- replace `internal/protocol/flat/protocol` with Rust-generated or Rust-consumed protocol bindings
- remove `go.mod` and `go.sum` once no Go targets remain
- update [docs/roadmap.md](../roadmap.md) to identify Rust as the chosen implementation language

During migration, keep [schema/nmux.fbs](../../schema/nmux.fbs) as the source of truth and avoid protocol shape changes unless they are documented separately.
