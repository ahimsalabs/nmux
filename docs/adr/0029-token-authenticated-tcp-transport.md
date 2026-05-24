# 0029: Token-Authenticated TCP Transport

Status: Accepted

Date: 2026-05-24

## Context

ADR 0014 sketches longer-term remote transport options, but nmux also needs a
small runnable network path for local-lab and tailnet experiments before QUIC,
SSH bootstrap, or Tailscale identity integration exist.

## Decision

Add an opt-in TCP listener and client path guarded by a shared token handshake.
The TCP bridge authenticates before handing the connection to the existing
FlatBuffers attach/control protocol, so Unix-socket and TCP clients share the
same daemon session logic after transport setup.

## Consequences

The default local Unix socket path remains unchanged. TCP is explicit on both
daemon and client (`--tcp-listen`/`--tcp`) and requires `--tcp-token`; it is not
a replacement for the future QUIC or SSH-bootstrap design in ADR 0014.

## Licensing Notes

This transport is implemented with the Rust standard library and nmux protocol
code. It does not copy or depend on GPL or AGPL implementation code.
