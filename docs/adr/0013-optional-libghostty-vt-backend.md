# 0013: Optional libghostty-vt Backend

Status: Superseded by ADR 0034

Date: 2026-05-21

## Context

ADR 0012 introduced the daemon-owned terminal engine boundary so the interim
text surface could be replaced without changing nmux client state-sync
semantics. M13 now has an experimental `libghostty-vt` implementation of that
boundary.

The Rust `libghostty-vt` crate provides a safe API over a native Ghostty VT
library build. Its current `libghostty-vt-sys` dependency fetches a pinned
Ghostty source commit and builds the native library with Zig. That is acceptable
for an opt-in correctness path, but it is too heavy and too toolchain-sensitive
to make the default nmux build path without a deliberate project decision.

## Decision

Import `libghostty-vt` as an optional Cargo feature named `libghostty-vt`.
ADR 0034 later enables this feature by default and keeps `interim` available
through no-default-features builds.

The original decision kept `interim` as the default and accepted
`nmuxd --terminal-engine libghostty-vt` only in feature builds. The feature is
pinned to an exact crate version, and the Nix dev shell pins Zig 0.15 because
the current vendored Ghostty build requires that Zig version.

The optional engine must implement the existing nmux terminal engine boundary:
PTY bytes enter the daemon-owned VT engine, and clients still receive
`PaneSurfaceSnapshot`, `PaneSurfacePatch`, and `ScrollbackChunk` objects. The
backend may use Ghostty render state and viewport/history APIs internally, but
nmux does not expose raw PTY replay to clients as a rendering shortcut.

## Consequences

The repository can now test real VT behavior while preserving the fast default
prototype loop.

Feature-gated tests cover ANSI sequence consumption, cursor-only updates,
alternate-screen detection, resize/reflow, backend-owned scrollback extraction,
session-level scrollback chunks, and a live CLI smoke path.

ADR 0034 is the later decision that accepts the native Ghostty/Zig build cost
for the default engine and package baseline.

## Licensing

Use the MIT/Apache Rust crates and the MIT Ghostty source fetched by the sys
crate or supplied through `GHOSTTY_SOURCE_DIR`. Do not copy Ghostty source files
or generated FFI from unrelated projects into nmux, and do not copy GPL or AGPL
terminal emulator code into the core.
