# Linux display and remote-view tickets

16 November 2026. These files are not GitHub issues, and they are not a Linux platform plan.

[`docs/platforms/linux.md`](../../platforms/linux.md) is a phase-3 sketch: a root daemon, an evdev grab, a layer-shell or X11 overlay, and PAM. The crate does not lock the machine. There are no other Linux product tickets. Henry's Decision 20 in `docs/SPEC.md` applies the display and remote-display requirements to Linux under X11 and under Wayland. These four tickets are blocked on a Linux platform plan, the way the macOS phase-2 tickets were blocked on an addendum. They do not choose a compositor, a capture API, or a lock surface.

The Linux capture-versus-remote entry in Decision 20 is open, and it is separate from the Windows and macOS entries. The Linux remote-input entry is open too. l03 waits on both. l01 and l02 do not choose either.

## Numbered list

| Id | Title | Blocked by | Where |
|---|---|---|---|
| [l01](l01-display-session-ci.md) | Probe: what display session `ubuntu-latest` has | a Linux platform plan | CI, `ubuntu-latest` |
| [l02](l02-capture-remote-spike.md) | Spike: capture versus xrdp, VNC, GNOME Remote Desktop, RustDesk | the plan, l01 | CI for whatever the runner already has |
| [l03](l03-remote-session-lock.md) | Remote session lock and unlock | the plan, l02, and both Linux open entries | Not runnable until the plan exists |
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

Both Linux open entries in Decision 20 also block l03. l01 and l02 do not choose them. They still wait on the plan, because a probe that installed a desktop would be an architecture, and this set does not add one.
