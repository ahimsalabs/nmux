# 0031: Container And Sandbox Host Execution

Status: Accepted

Date: 2026-05-24

## Context

ADR 0005 established the process-host boundary and included local, container,
and sandbox host kinds in the model. It also warned against tying terminal
state, protocol shape, or frontend rendering to a specific process launcher.

The current implementation now exposes host selection through `nmuxd --host`
and lowers non-local hosts to explicit process boundaries:

- `--host container --container-image IMAGE` runs the pane command through
  `${NMUX_CONTAINER_RUNTIME:-docker} run --rm -i [-t] ...`;
- `--host sandbox --sandbox-profile PROFILE` runs through macOS
  `sandbox-exec` where available and reports unsupported host errors on other
  platforms.

## Decision

Treat container and sandbox execution as experimental process-host adapters,
not as a hardened security or packaging feature.

The host boundary remains daemon-owned. Clients render the same backend-owned
workspace and terminal state regardless of whether a pane process is local,
container-backed, or sandbox-backed. Host choice must not leak into the
FlatBuffers surface protocol as renderer behavior.

The current container adapter may pass pane command, working directory, and
environment into the runtime. It must remain opt-in and explicit; local process
hosting stays the default. The current sandbox adapter is platform-specific and
must fail clearly when unsupported.

Before these adapters are documented as production isolation, nmux needs a
separate host-execution policy covering filesystem mounts, network access,
runtime discovery, terminal allocation, environment filtering, identity
propagation, and user-facing security claims.

## Consequences

`--host container` and `--host sandbox` are useful for exercising the
process-host abstraction and for local experiments. They are not a promise that
nmux confines untrusted code safely.

Future work may add richer host metadata or remote host kinds, but that should
stay behind the process-host boundary or use append-only protocol extensions
with a new ADR. Do not copy container, sandbox, or terminal launcher code from
GPL or AGPL projects into this repository.

## Licensing Notes

The current adapters are built from nmux code plus platform/runtime command
invocation. They do not copy or depend on GPL or AGPL implementation code.
