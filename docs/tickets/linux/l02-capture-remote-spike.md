# l02: Spike: capture versus a Linux remote view

**What to build:** A throwaway probe on `ubuntu-latest`, and only against a display server l01 found already running. Put a marker window on that server. Count the marker with the capture tool that is already installed. Then count it through a remote path only if that path is already installed. The paths the requirement names, and that this ticket does not install, are xrdp, a VNC server, GNOME Remote Desktop, and RustDesk. xrdp's README describes an RDP server that can sit on Xorg or Xvnc. GNOME's desktop-sharing help describes viewing and controlling the desktop. GNOME Connections connects with `rdp://`, `vnc://`, or `spice://`. RustDesk's README calls itself a remote desktop solution and its tree has a screen-capture library. If none of those servers is on the image, the log says each one is unavailable. The finding does not choose the Linux entry in Decision 20, and it does not add a capture API to the phase-3 sketch.

**Blocked by:** a Linux platform plan, and l01.

**Blocks:** l03.

**Where:** CI only, to the extent l01 found a display server. A remote server that is not installed is a written miss, not a package to add.

**Type:** Prototype spike. No product code.

**Status:** blocked on a Linux platform plan. Not ready-for-agent. Not a GitHub issue. Does not decide the Linux open entry.

- [ ] The finding names the display server l01 found, or says there was none (finding)
- [ ] Marker counts for a local capture, if a capture tool was already installed. Otherwise the log says no capture tool was installed
- [ ] Each of xrdp, VNC, GNOME Remote Desktop, and RustDesk is either measured or recorded unavailable (job log)
- [ ] The finding tells l03 which options in the Linux entry remain open. It does not pick one

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
