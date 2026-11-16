# Linux display and remote-view tickets

16 November 2026. These files are not GitHub issues, and they are not a Linux platform plan.

[`docs/platforms/linux.md`](../../platforms/linux.md) is a phase-3 sketch: a root daemon, an evdev grab, a layer-shell or X11 overlay, and PAM. The crate does not lock the machine. There are no other Linux product tickets. Henry's Decision 20 in `docs/SPEC.md` applies the display and remote-display requirements to Linux under X11 and under Wayland. These four tickets are blocked on a Linux platform plan, the way the macOS phase-2 tickets were blocked on an addendum. They do not choose a compositor, a capture API, or a lock surface.

Henry closed both Linux entries in Decision 20. A remote viewer sees the SoftLock lock screen until the configurable pass, and remote-tool input is blocked until that pass except what the unlock screen needs. l02 and l03 are post-v1. l01 and the display rows of l04 are not. l03 waits on the plan and on l02, not on an open choice.

## Numbered list

| Id | Title | Blocked by | Where |
|---|---|---|---|
| [l01](l01-display-session-ci.md) | Probe: what display session `ubuntu-latest` has | a Linux platform plan | CI, `ubuntu-latest` |
| [l02](l02-capture-remote-spike.md) | Spike: capture versus xrdp, VNC, GNOME Remote Desktop, RustDesk. Post-v1 | the plan, l01 | CI for whatever the runner already has |
| [l03](l03-remote-session-lock.md) | Remote session lock, the remote pass, and remote-tool input. Post-v1 | the plan, l02 | Not runnable until the plan exists |
| [l04](l04-display-kit.md) | Hardware kit | the plan, and a named Linux machine | No machine recorded |

## Blocking edges

```text
plan --> l01
plan --> l02
plan --> l03
plan --> l04
l01 --> l02
l02 --> l03
l03 --> l04
```

l01 and l02 still wait on the plan, because a probe that installed a desktop would be an architecture, and this set does not add one. The owner decision does not remove that block.
