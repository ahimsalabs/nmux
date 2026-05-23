# 0014: Transport Identity Boundary

Status: Proposed

Date: 2026-05-22

## Context

nmux currently runs over Unix domain sockets on a single machine. The intended use cases include remote access over Tailscale, headless agent connections, and proxy composition across multiple machines. The protocol needs a network transport and identity layer that does not require SSH as a long-lived transport, supports multiple auth sources, and avoids unnecessary encryption layers.

The primary deployment target is Tailscale (WireGuard mesh). Non-Tailscale deployments and local-only use must also work without protocol changes. See the Transport section of [WORK.md](../../WORK.md) for background.

Prior art: mosh uses SSH to bootstrap a UDP session key, then runs its own encrypted protocol. tmux and screen have no native network transport. The mosh model (SSH bootstrap, then direct connection) is well-established and avoids tying session lifetime to SSH connection lifetime.

## Decision

### Three transport paths, one identity model

nmux supports three transport paths. Each resolves to the same `Actor` identity used by the FlatBuffers protocol. The protocol layer does not know which transport authenticated the peer.

**Local (Unix domain socket).** Identity comes from Unix peer credentials (`SO_PEERCRED` / `getpeereid`). This is the current implementation and remains the zero-overhead default for same-machine use.

**Tailscale (TCP over WireGuard).** `nmuxd` listens on the Tailscale interface (100.x.y.z) on a known port. WireGuard provides encryption and peer authentication at the network layer. Application-layer TLS is not added on top of WireGuard -- double encryption adds latency and complexity with no security benefit on a tailnet.

Identity comes from the `tailscaled` local API: `GET /localapi/v0/whois?addr=IP:port` returns the peer's `UserProfile` (LoginName, DisplayName) and `Node` (Name, StableID, Addresses). An ephemeral per-session token is exchanged on the first protocol message to guard against local-process spoofing on a compromised node (a rogue process on the same machine could call WhoIs or connect to the nmux port). The token is short-lived, signed by `nmuxd`, and verified on each connection.

Tailscale ACLs are the first authorization gate (who can reach the port). nmux permissions are the second gate (what the peer can do once connected). Tailscale ACL grants can carry nmux-specific capabilities (`tailscale.com/cap/nmux`) that `nmuxd` reads from the WhoIs response.

**Non-Tailscale (SSH bootstrap + QUIC/TLS).** `nmux connect host` opens an SSH connection using the user's existing SSH configuration (keys, agent, certificates, `.ssh/config`). The SSH session starts or discovers `nmuxd` on the remote, exchanges a short-lived signed session token, and exits. The client then opens a direct QUIC+TLS connection using that token. The SSH connection lasts seconds, not the session lifetime.

The token encodes: daemon identity, session reference, peer permissions, and expiry. It is signed by `nmuxd` with a daemon-owned key. QUIC provides TLS 1.3 encryption, connection migration (RFC 9000), and 0-RTT reconnection for session resumption. TCP+TLS is an acceptable fallback when QUIC is blocked.

### Identity resolution

All three paths produce the same fields before the first protocol message:

```
actor_id:      stable identifier (Unix uid, Tailscale Node.StableID, or token subject)
user_id:       human-readable login (Unix username, Tailscale LoginName, or SSH principal)
display_name:  optional (Tailscale DisplayName, or SSH comment, or empty)
mode:          read-write or read-only (from policy lookup)
```

These map directly to the existing `AttachRequest` fields. No protocol schema change is required for identity resolution itself.

### Proxy composition

A proxy daemon (`nmux-proxy` or `nmuxd --proxy`) connects upstream to multiple `nmuxd` instances and serves a single workspace downstream:

- The proxy assembles a synthetic `WorkspaceTreeSnapshot` from upstream workspace trees.
- Pane IDs are namespaced by upstream origin (e.g., `dev-server/pane-1`).
- `PaneSurfaceSnapshot` and `PaneSurfacePatch` messages are forwarded from upstream as raw FlatBuffers bytes, rewriting only the pane ID in the envelope. No deserialize/reserialize of cell data.
- `InputEvent`, `ScrollbackFetch`, and `ResizeIntent` are routed to the owning upstream by pane ID prefix.
- Per-pane version streams are independent across upstreams. No cross-upstream version coordination.
- Presence is aggregated: a client attached to the proxy appears in presence on all relevant upstreams.
- The proxy can enforce its own access policy per upstream (e.g., force read-only for production panes).
- If an upstream disconnects, the proxy marks its panes as unavailable in the workspace tree rather than tearing down the whole session.

Chaining works because proxies are nmux protocol speakers on both sides. A proxy-to-proxy chain (bastion pattern) assembles origin metadata at each hop.

### Pane origin metadata

`nmuxd` sets environment variables in every PTY it spawns:

```
NMUX=1
NMUX_SESSION_ID=<session-id>
NMUX_PANE_ID=<pane-id>
NMUX_SOCKET=<socket-path-or-endpoint>
NMUX_ORIGIN=<host-identity>
```

Current local implementation: `nmux-core` exposes session-level command
environment injection for these variables, and `nmuxd` supplies the resolved
local socket endpoint before the pane PTY starts. Automatic nested client
origin-chain attachment is still future protocol/client work.

When `nmux` (the client) runs inside an nmux-managed pane, it reads these variables and includes the parent's origin in its `AttachRequest`. Each hop appends to the origin chain. This enables automatic chain discovery without explicit proxy configuration for the common case.

Known pitfalls and mitigations:

- **Stale variables from crashed daemons.** The client should validate that `NMUX_SOCKET` is reachable before including chain metadata. Stale vars are treated as absent.
- **Leaking through sudo/su.** `sudo` preserves env by default on some systems. `nmuxd` should set vars only in the PTY environment, not export them to the daemon's own environment.
- **Leaking into containers.** Container runtimes typically do not inherit host env vars unless explicitly passed. This is the desired behavior -- containers start a fresh chain.
- **SSH does not forward env vars by default.** For cross-machine chains, the nmux native transport (not SSH) carries the chain metadata in the protocol, not in env vars. Env vars are for local/same-machine nesting detection only.
- **Circular chains.** The client maintains a chain list and rejects connections that would create a cycle (same session ID appearing twice in the chain).

### Zero-overhead local embedding

When a terminal emulator (Ghostty) embeds `nmuxd` in-process, the local render path reads VT state directly from the in-process `libghostty-vt` instance. No FlatBuffers serialization, no IPC, no Unix socket. The FlatBuffers protocol only activates when a remote client connects to the same session.

This is the same dual-path pattern used by Chrome DevTools (V8 inspector: in-process fast path for the renderer, serialized CDP for remote inspectors) and gRPC in-process optimization.

The risk is divergence between paths: a bug in serialization could make remote clients see different state than the local renderer. Mitigation: invariant tests that periodically serialize the in-process VT state and compare it against a direct read, catching any discrepancy in the test suite.

## Consequences

- The FlatBuffers protocol and `AttachRequest` schema do not change for basic identity resolution. The three transport paths are below the protocol layer.
- `PaneNode` in the schema should gain an optional `HostOrigin` field (append-only, non-breaking) to carry origin metadata through the workspace tree. Old clients ignore the field. ADR 0005 anticipated this extension point.
- `HostKind` in the Rust model should gain a `Remote` variant alongside Local, Container, and Sandbox.
- The proxy daemon is a new component that speaks nmux protocol on both sides. It does not require protocol changes -- it operates on existing envelope types.
- The `NMUX_*` environment variables should be set by the local PTY host implementation in `nmux-core`, not by `nmux-cli`, so that any daemon embedding (including Ghostty) gets them automatically.
- The Tailscale identity path depends only on HTTP calls to the tailscaled Unix socket, not on any Tailscale library or SDK in the Rust build.
- The SSH bootstrap path depends on the `ssh` binary being available on the client machine, not on an SSH library in the Rust build.
- No GPL or AGPL code is involved in any transport path.

## Compatibility

This decision does not change the current wire protocol. The `HostOrigin` schema extension and `NMUX_*` environment variables are additive. Existing local Unix socket transport continues to work unchanged.
