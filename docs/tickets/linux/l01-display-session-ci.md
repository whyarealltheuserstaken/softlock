# l01: Probe: what display session ubuntu-latest has

**What to build:** On `ubuntu-latest`, print `XDG_SESSION_TYPE`, whether `DISPLAY` is set, whether `WAYLAND_DISPLAY` is set, and the output of `uname -m`. If an X server is already running and `xrandr` is already installed, log the connected outputs and attempt one mode change, then restore it. If a Wayland compositor is already running, log the outputs it exposes and do not reconfigure them unless the compositor's own tool is already on the image. If neither is running, the log says so. That is a result, not a failure of the job. This ticket does not install a desktop environment, a compositor, xrdp, or a VNC server. Installing one would be a design, and there is no Linux platform plan to put it in.

**What the runner cannot prove:** a physical monitor hot-plug, a closed lid, a USB display, mirroring to a second panel, or any remote view. Those rows are l04, once a machine is named.

**Blocked by:** a Linux platform plan. [`docs/platforms/linux.md`](../../platforms/linux.md) is a phase-3 sketch, not that plan.

**Blocks:** l02.

**Where:** CI only, `ubuntu-latest`.

**Type:** Probe. Deliverable is a written finding. No product code, and no new lock behavior.

**Status:** blocked on a Linux platform plan. Not ready-for-agent. Not a GitHub issue. This probe is the display session. It is not a remote-view ticket and it is not marked post-v1 for that reason.

- [ ] The job prints the session type and whether `DISPLAY` and `WAYLAND_DISPLAY` are set (job log)
- [ ] An X11 mode change is logged only when an X server and `xrandr` were already there. Otherwise the log says X11 was not running (job log)
- [ ] Wayland outputs are logged only when a compositor was already there. Otherwise the log says Wayland was not running (job log)
- [ ] The finding states that the runner did not prove physical hot-plug or a remote view
- [ ] The job does not install a display server

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
