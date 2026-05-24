# ADR 0027: Speculative Local Echo Boundary

Status: Proposed

Date: 2026-05-24

## Context

nmux clients send explicit input events to a daemon that owns terminal state and
returns structured surface snapshots or patches. That state-sync shape makes a
Mosh-style speculative local echo possible: a client could display a predicted
printable character immediately at the confirmed cursor position, then replace
that prediction when the next server-owned surface update arrives.

The expected benefit is smaller than Mosh's high-latency SSH use case because
the current nmux transport is a local Unix socket. The main visible delay SLE
might mask is the server-side quiet-polling window used after forwarding input
to a PTY. The risk is higher than ordinary local echo because a wrong prediction
can imply terminal state the daemon never confirmed.

## Decision

Any nmux SLE work must start as a client-local overlay on top of confirmed
`ClientAttachState` surfaces. The overlay may affect live rendering, but it must
not mutate cached confirmed rows, surface versions, row state hashes, scrollback
state, protocol frames, or daemon-owned terminal state.

The first supported prediction scope is deliberately narrow:

- printable single-cell text sent as a normal key input while attached
  read-write;
- confirmed main-screen surface with a visible cursor;
- cursor row and column inside the current surface bounds;
- no active alternate screen claim, mouse/focus/paste input, resize intent,
  raw stdin-byte mode, named-key input, bracketed paste, or application cursor
  interaction;
- no prediction across a row wrap, wide grapheme, combining cluster, tab,
  newline, carriage return, escape, or backspace until explicit tests cover it.

The overlay should track predicted cells separately from confirmed cells,
including the base pane ID, surface version, row index, column, predicted text,
and input sequence that caused the prediction. Rendering may mark pending cells
with a temporary visual style such as underline only if that can be done without
changing the confirmed `SurfaceRowUpdate` style table.

Reconciliation happens only when a server-owned surface update is applied:

- if a confirmed row/version contains the predicted text at the predicted
  coordinates, drop the matching prediction;
- if the confirmed update changes the predicted row, cursor, surface kind, or
  base version in an incompatible way, discard the prediction and record a
  miss;
- if a full snapshot or `ReplaceRows` update arrives, treat it as authoritative
  and clear incompatible predictions before rendering.

Visible prediction should not become a default behavior until nmux has either
measured that local latency needs it or added explicit daemon settlement
metadata. A future protocol extension may expose input high-water marks such as
"received through", "delivered to PTY through", and "echo settled through" so a
client can distinguish "not confirmed yet" from "wrong". Until then, any
prototype must treat unconfirmed predictions as short-lived and disposable
rather than counting every missing row update as a settled miss.

A prediction engine must be able to back off. The initial policy should disable
predictions after recent misses and re-enable only after a quiet period or after
confirmed simple echo behavior is observed. Password prompts, editors, shells
with custom line editing, remote full-screen programs, and alternate-screen
applications should bias toward no prediction.

## Consequences

SLE remains an optional frontend behavior, not a protocol guarantee. It should
be guarded by an explicit client option or experimental mode until local latency
measurements and regression tests justify a default.

The daemon remains the sole terminal-state authority. Confirmed rendering,
persisted client state, scrollback fetches, renderer-equivalence fixtures, and
default-engine promotion evidence must ignore speculative overlay state.

The first implementation milestone should add tests around the prediction data
model and reconciliation against `SurfaceUpdate` objects before wiring live TTY
painting. A later milestone can measure whether the quiet-polling window is
visible enough to justify enabling SLE for local workflows or whether an
input-correlated daemon fast-flush is a better fix.

## Licensing And Compatibility Notes

Mosh is useful prior art, but nmux must not copy GPL/AGPL implementation code.
The accepted boundary is architectural: predict locally, keep predictions
separate from confirmed server state, and reconcile against authoritative
server snapshots or patches.
