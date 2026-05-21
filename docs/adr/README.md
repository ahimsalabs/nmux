# Architecture Decision Records

This directory records architecture decisions that should remain understandable as nmux moves from notes toward an implementation.

Add a new ADR for decisions that change the protocol shape, process boundaries, terminal-state ownership, adapter boundaries, or licensing posture. Do not rewrite old ADRs to hide history; supersede them with a later record when needed.

## Records

- [0001: Backend-Owned Terminal State](0001-backend-owned-terminal-state.md)
- [0002: Rust Core Runtime](0002-rust-core-runtime.md)
- [0003: Scrollback As A Separate Object](0003-scrollback-as-separate-object.md)
- [0004: Presence And Attach Modes](0004-presence-and-attach-modes.md)
- [0005: Process Host Boundary](0005-process-host-boundary.md)
- [0006: Interim PTY Text Surface](0006-interim-pty-text-surface.md)
- [0007: Ghostty/libghostty Frontend Boundary](0007-ghostty-libghostty-frontend-boundary.md)
- [0008: FlatBuffers Attach Request](0008-flatbuffers-attach-request.md)
