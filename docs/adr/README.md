# Architecture Decision Records

This directory records architecture decisions that should remain understandable as nmux moves from notes toward an implementation.

Add a new ADR for decisions that change the protocol shape, process boundaries, terminal-state ownership, adapter boundaries, or licensing posture. Do not rewrite old ADRs to hide history; supersede them with a later record when needed.

## Records

| ADR | Status |
| --- | --- |
| [0001: Backend-Owned Terminal State](0001-backend-owned-terminal-state.md) | Proposed |
| [0002: Rust Core Runtime](0002-rust-core-runtime.md) | Proposed |
| [0003: Scrollback As A Separate Object](0003-scrollback-as-separate-object.md) | Proposed |
| [0004: Presence And Attach Modes](0004-presence-and-attach-modes.md) | Proposed |
| [0005: Process Host Boundary](0005-process-host-boundary.md) | Proposed |
| [0006: Interim PTY Text Surface](0006-interim-pty-text-surface.md) | Proposed |
| [0007: Ghostty/libghostty Frontend Boundary](0007-ghostty-libghostty-frontend-boundary.md) | Accepted |
| [0008: FlatBuffers Attach Request](0008-flatbuffers-attach-request.md) | Accepted |
| [0009: tmux Adapter Process Boundary](0009-tmux-adapter-process-boundary.md) | Accepted |
| [0010: herdr Integration Boundary](0010-herdr-integration-boundary.md) | Accepted |
| [0011: Live Local Interactive Attach](0011-live-local-interactive-attach.md) | Accepted |
| [0012: Backend Terminal Engine Boundary](0012-backend-terminal-engine-boundary.md) | Accepted |
| [0013: Optional libghostty-vt Backend](0013-optional-libghostty-vt-backend.md) | Accepted |
| [0014: Transport Identity Boundary](0014-transport-identity-boundary.md) | Proposed |
| [0015: Cell Semantic Content](0015-cell-semantic-content.md) | Accepted |
| [0016: Terminal Color State](0016-terminal-color-state.md) | Accepted |
| [0017: Terminal Input Mode State](0017-terminal-input-mode-state.md) | Accepted |
| [0018: libghostty-vt Default And CI Gate](0018-libghostty-vt-default-and-ci-gate.md) | Accepted |
| [0019: Color-Only Palette Diffs](0019-color-only-palette-diffs.md) | Accepted |
| [0020: Attributed Error Frames](0020-attributed-error-frames.md) | Accepted |
| [0021: Explicit Attach Status](0021-explicit-attach-status.md) | Accepted |
| [0022: Hyperlink Identity Table](0022-hyperlink-identity-table.md) | Accepted |
| [0023: Post-M13 Default Engine And Product Split](0023-post-m13-default-engine-and-product-split.md) | Accepted |
| [0024: Native VT Source Policy Criteria](0024-native-vt-source-policy-criteria.md) | Accepted |
| [0025: Native VT Packaging Criteria](0025-native-vt-packaging-criteria.md) | Accepted |
| [0026: Native VT CI Promotion Criteria](0026-native-vt-ci-promotion-criteria.md) | Accepted |
| [0027: Speculative Local Echo Boundary](0027-speculative-local-echo-boundary.md) | Proposed |
| [0028: Runtime Control Commands](0028-runtime-control-commands.md) | Accepted |
| [0029: Token-Authenticated TCP Transport](0029-token-authenticated-tcp-transport.md) | Accepted |
| [0030: Live Session Recording Format](0030-live-session-recording-format.md) | Accepted |
| [0031: Container And Sandbox Host Execution](0031-container-and-sandbox-host-execution.md) | Accepted |
| [0032: Single Binary CLI Boundary](0032-single-binary-cli-boundary.md) | Accepted |
| [0033: Remove nmuxd Shim](0033-remove-nmuxd-shim.md) | Accepted |
| [0034: Ghostty VT Default Engine](0034-ghostty-vt-default-engine.md) | Superseded by ADR 0035 |
| [0035: Restore Portable Default Engine](0035-restore-portable-default-engine.md) | Accepted |
