# l02: Spike: capture versus a Linux remote view

**What to build:** A throwaway probe on `ubuntu-latest`, and only against a display server l01 found already running. Put a marker window on that server. Count the marker with the capture tool that is already installed. Then count it through a remote path only if that path is already installed. The paths the requirement names, and that this ticket does not install, are xrdp, a VNC server, GNOME Remote Desktop, and RustDesk. xrdp's README describes an RDP server that can sit on Xorg or Xvnc. GNOME's desktop-sharing help describes viewing and controlling the desktop. GNOME Connections connects with `rdp://`, `vnc://`, or `spice://`. RustDesk's README calls itself a remote desktop solution and its tree has a screen-capture library. If none of those servers is on the image, the log says each one is unavailable. The finding records the paths against the closed rule in Decision 20. It does not add a capture API to the phase-3 sketch, and it does not weaken the rule.

**Blocked by:** a Linux platform plan, and l01, for the probe (v1.x/v2).

**Blocks:** l03's v1.x/v2 slice

**Where:** CI only, to the extent l01 found a display server. A remote server that is not installed is a written miss, not a package to add.

**Type:** Prototype spike. No product code.

**v1 framework:** The detection-hook result type exists, including a field for "remote input could not be told from an agent's." A seam test feeds a fake. v1 does not install a remote server and does not enable the feature. This slice still waits on a Linux platform plan, because the plan is what the type will sit in. The sketch is not that plan.

**v1.x or v2:** The checkboxes below. A path that cannot be told from an agent's post is a risk. The rule does not weaken.

**Status:** v1 framework plus a v1.x or v2 probe, both still blocked on a Linux platform plan. Not ready-for-agent. Not a GitHub issue. Henry closed the rule in Decision 20.

- [ ] v1: the detection-hook result type has the "could not tell apart" field, set from a fake. No remote server is installed (seam 1)
- [ ] v1.x/v2: The finding names the display server l01 found, or says there was none (finding)
- [ ] v1.x/v2: Marker counts for a local capture, if a capture tool was already installed. Otherwise the log says no capture tool was installed
- [ ] v1.x/v2: Each of xrdp, VNC, GNOME Remote Desktop, and RustDesk is either measured or recorded unavailable (job log)
- [ ] v1.x/v2: The finding names any remote input it could not tell from an agent's post. It does not recommend letting that input through, and it does not recommend blocking local agent input

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
