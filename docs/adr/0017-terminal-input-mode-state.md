# ADR 0017: Terminal Input Mode State

## Status

Accepted.

## Date

2026-05-22

## Context

M13 routes local input through daemon-owned terminal state instead of letting
clients inject raw PTY bytes for terminal-aware operations. Paste, focus,
keypad, cursor-key, and mouse behavior depends on modes negotiated by the
application running inside the pane. If clients guess those modes from cached
state or frontend assumptions, reconnects and multiple clients can diverge.

`libghostty-vt` exposes enough safe API to observe bracketed paste, focus
reporting, application keypad, application cursor, origin, wraparound, mouse
tracking mode, and mouse encoding format. It can also encode named keys and
mouse events from the daemon-owned terminal state.

Broader physical-key and text-event forwarding is still a frontend protocol
decision. It needs a model for keyboard layouts, text composition, modifiers,
and platform-specific key identity before nmux can expose it as a stable public
contract.

## Decision

Carry terminal input modes in `TerminalModeState` on pane surface snapshots and
patches. The state includes:

- bracketed paste;
- focus reporting;
- application keypad;
- application cursor;
- origin and wraparound;
- a compatibility mouse-tracking boolean;
- detailed mouse tracking mode: `None`, `X10`, `Normal`, `Button`, or `Any`;
- mouse encoding format: `X10`, `Utf8`, `Sgr`, `Urxvt`, or `SgrPixels`.

The daemon remains the authority for input gating:

- paste input is wrapped by the daemon in bracketed-paste delimiters only when
  the current pane mode reports bracketed paste enabled;
- the historical `PasteInput.bracketed` field is not a client authority. Local
  serving paths ignore it and derive delimiter selection from daemon-owned pane
  mode so current-surface reconnects and multiple clients cannot disagree about
  bracketed paste state;
- focus gained/lost input is sent to the daemon, which forwards it only when
  focus reporting is enabled and otherwise reports a structured error;
- keypad and cursor named keys are encoded through the live pane terminal
  engine from daemon-owned modes;
- mouse input is forwarded according to the daemon-owned tracking mode:
  `None` blocks, `X10` allows press, `Normal` allows press and release,
  `Button` allows press/release plus motion with a button, and `Any` allows all
  explicit mouse actions.

The mouse format is preserved as terminal state and passed to the terminal
engine for byte encoding. Public `MouseInput` carries zero-based cell
coordinates and may also carry explicit pixel coordinates for terminals using
`MouseFormat::SgrPixels`. When pixel coordinates are absent, the daemon encodes
the event at the top-left of the referenced cell. Clients should not recover
from unsupported mode state by replaying raw PTY bytes.

## Consequences

Reconnects and multiple clients can make input decisions from synchronized
terminal-state objects rather than local terminal assumptions. The protocol
keeps a coarse mouse-tracking boolean for compatibility while giving newer
clients enough state to reason about tracking policy and encoding format.

Physical-key and text-event forwarding remain withheld until nmux has a
frontend-facing input object model that can represent platform key identity and
text composition without baking in one terminal UI's assumptions. Future input
schema changes must keep the daemon-owned gating rule.
