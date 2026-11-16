# l04: Kit: Linux displays and remote views

**What to build:** A sheet on a named Linux machine. No Linux machine is recorded. This ticket does not invent one. Rows, separately for X11 and for Wayland when the machine can run each: external display hot-plug, a mode or scale change, mirroring if the machine can mirror, a closed lid with an external display if the machine is a laptop, a USB display if one is available, and an inbound remote view while locked. The remote views are xrdp, VNC, GNOME Remote Desktop, and RustDesk, each marked not available when the machine does not have it. The requirement matches Windows and macOS. The mechanism does not get designed here. The sheet records the distribution, the session type, and the compositor or X server. It does not record a serial number.

**Blocked by:** a Linux platform plan, and a named Linux machine. The remote-view rows are also blocked by l03.

**Blocks:** nothing else.

**Where:** A Linux machine. None is recorded. `ubuntu-latest` is l01 and is not this kit.

**Type:** Hardware kit.

**Status:** blocked. No machine is named, and there is no Linux platform plan. Not ready-for-agent. Not a GitHub issue. The display rows are not post-v1. The remote-view rows are post-v1 and also wait on l03.

- [ ] The machine is named, and the sheet records the distribution and whether the session is X11 or Wayland (sheet)
- [ ] Hot-plug of an external display while locked covers the new display, on each session type the machine can run (sheet)
- [ ] A mode or scale change while locked leaves every display covered (sheet)
- [ ] Closed lid with an external display, if the machine has a lid. If it has no lid, the row says not runnable (sheet)
- [ ] A USB display, if one is available (sheet)
- [ ] Inbound xrdp, VNC, GNOME Remote Desktop, and RustDesk: the viewer sees the lock, or the sheet says that server is not on the machine (sheet)
- [ ] The sheet does not contain a serial number

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
