# l03: Remote session lock and unlock

**What to build:** Once a Linux platform plan exists, locking while xrdp, VNC, GNOME Remote Desktop, RustDesk, or a remote view of the same kind is connected covers that view. Connecting to a locked machine shows the SoftLock screen. Unlock is the plan's unlock (the sketch names PAM) through the same Broker nonce rule, with no weaker path. Remote input before unlock reaches only the unlock UI. A crash or any other failure ends in the operating system's lock for that session and does not enter Unlocked. X11 and Wayland are both in scope. The surface that stays visible to the remote viewer is the option Henry picks in the Linux entry of Decision 20. This ticket does not pick it, and it does not write the plan.

**Blocked by:** a Linux platform plan, l02, and the Linux open entry. It stays blocked while that entry is open, and while the plan does not exist.

**Blocks:** l04 (the remote-view rows).

**Where:** Not a runner claim. `ubuntu-latest` does not become a lock test by installing a desktop.

**Type:** Product ticket, empty until the plan exists. No architecture in this file.

**Status:** blocked on a Linux platform plan and on the open Linux entry. Not ready-for-agent. Not a GitHub issue.

- [ ] A Linux platform plan exists, and this ticket points at it instead of the phase-3 sketch
- [ ] Locking while one of the named remote views is connected covers that view
- [ ] An inbound connection to an already locked session shows the SoftLock screen
- [ ] Unlock adds no method the plan does not already have
- [ ] Remote input before unlock reaches only the unlock UI
- [ ] A crash ends in the OS lock for that session and the state is not Unlocked

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
