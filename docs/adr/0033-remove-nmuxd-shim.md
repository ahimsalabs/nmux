# 0033: Remove nmuxd Shim

Status: Accepted

Date: 2026-05-24

## Context

ADR 0032 consolidated daemon startup under `nmux daemon` while keeping `nmuxd`
as a temporary compatibility shim. The project has not shipped a stable public
interface that needs that compatibility, and upcoming release artifacts should
not carry two binaries for one command surface.

Keeping the shim would make packaging, release provenance, help text, and
remote examples present two daemon entrypoints while the intended interface is
already `nmux daemon`.

## Decision

Remove the standalone `nmuxd` binary target. `nmux daemon ...` is the only
daemon entrypoint, and managed startup re-execs the current `nmux` binary with
the `daemon` subcommand.

The default local socket filename changes from `nmuxd.sock` to `nmux.sock` so
new help text and examples no longer expose the removed binary name.

## Consequences

Packages and archives contain one user-facing binary, `nmux`. Release metadata,
checksums, and provenance only need to describe that binary.

Existing scripts that called `nmuxd` must switch to `nmux daemon`. Because nmux
has not promised compatibility for the separate daemon binary, this removes
transition code before it becomes part of the public surface.

## Licensing Notes

This is a CLI packaging and process-startup cleanup. It does not change
terminal-state ownership, the protocol, or third-party source dependencies.
