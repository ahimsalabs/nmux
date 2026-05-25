# ADR 0041: Client Inventory Subscription

## Status

Accepted.

## Date

2026-05-25

## Context

Presence identifies actors, but the UI also needs transport-level connection
inventory: how many clients are attached, who owns them, which machine they are
from, and which pane they are focused on. This is different from terminal state
and should not be inferred from pane output or presence alone.

The status bar should be able to continuously show a compact client count, and a
future clients pane should be able to render the same data without polling for a
full list.

## Decision

Add an optional live client inventory subscription on `AttachRequest`.

Subscribed live clients receive:

- `ClientInventorySnapshot`: the current full inventory with a daemon-local
  inventory version.
- `ClientInventoryPatch`: incremental joined, updated, and left changes against
  a base version.

Each `ClientConnection` entry has a daemon-assigned `connection_id`, actor
identity, optional frontend metadata such as hostname and client kind, attach
mode, focused pane, connected time, last-seen time, and optional last-input
time. The daemon owns connection IDs and inventory versions.

Inventory frames are control-plane state. They do not alter visible presence,
resize policy, pane focus, terminal state, or process-host behavior.

## Consequences

The status bar can maintain a small live client cache and render counts without
requesting a full list on every change.

Older clients do not set the subscription flag and will not receive inventory
frames. Future clients panes can reuse the same snapshot/patch cache instead of
adding a separate polling protocol.
