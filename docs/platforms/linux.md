# Linux backend (phase 3)

The Linux crate compiles and links today. It does not lock the machine. Every platform capability reports Unsupported, and the settings app shows "This platform isn't supported yet".

Phase 3 targets mainstream distributions under Wayland and X11. The shape below is the spec's decision, recorded so the core does not have to change when the backend is written.

## Processes

A root daemon plus a session client.

## Input guard

The leading design is an evdev exclusive grab (`EVIOCGRAB`) of physical input devices. Virtual `uinput` devices used by agents stay ungrabbed. The daemon reads the grabbed devices for the unlock prompt. This works under both Wayland and X11.

`ext-session-lock-v1` was evaluated for the lock surface. Compositors route all input to the lock client, including `uinput` devices, and may stop presenting other surfaces. That likely conflicts with agent passthrough, so the protocol may serve only "lock for real".

## Overlay

A `wlr-layer-shell` overlay where the compositor provides it, with an X11 override-redirect window as the fallback.

## Unlock

PAM.

## Open risks

- Capture exclusion has no portable Wayland mechanism.
- `ext-session-lock-v1` conflicts with letting agent input through.
- An evdev grab has to keep `uinput` devices ungrabbed or agents stop.

No lock behavior belongs in this crate until those risks have a tested answer.

## Connected displays and remote views

Henry's Decision 20 in `docs/SPEC.md` applies here: Windows 11, Windows 10, Linux, and the macOS Universal build. This file is the phase-3 sketch above. It is not a ticketed Linux platform plan. There are no other Linux product tickets.

The display and remote-display tickets are [`docs/tickets/linux/`](../tickets/linux/README.md). They are blocked on a Linux platform plan. They name X11 and Wayland, and the remote paths xrdp, VNC, GNOME Remote Desktop, and RustDesk, because those are the paths the requirement covers. They do not choose a compositor, a capture API, or a lock-surface protocol. Henry closed that behavior in Decision 20. Wayland still has no portable capture exclusion, and this sketch does not add one. The remote rows are post-v1. A measurement that cannot separate a remote viewer from local capture is a risk on l02. It does not change the rule.
