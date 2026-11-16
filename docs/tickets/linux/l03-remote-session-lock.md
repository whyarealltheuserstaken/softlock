# l03: Remote session lock and unlock

**What to build:** Once a Linux platform plan exists, locking while xrdp, VNC, GNOME Remote Desktop, RustDesk, or a remote view of the same kind is connected covers that view. Connecting to a locked machine shows the SoftLock screen. Unlock is the plan's unlock (the sketch names PAM) through the same Broker nonce rule, with no weaker path. Remote input before unlock is the open remote-input entry in Decision 20. This ticket does not name a flag that separates that input from an agent's, and it does not add one. A killed lock host, or both lock paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked. That is invariant 3 in SPEC.md Decision 5, plus the known gap in section 7 until ticket 13, once a Linux plan has those states. Until the plan exists, this sentence does not invent them. X11 and Wayland are both in scope. The surface that stays visible to the remote viewer is the option Henry picks in the Linux entry of Decision 20. This ticket does not pick it, and it does not write the plan.

**Blocked by:** a Linux platform plan, l02, and the Linux capture entry and the Linux remote-input entry. It stays blocked while either entry is open, and while the plan does not exist.

**Blocks:** l04 (the remote-view rows).

**Where:** Not a runner claim. `ubuntu-latest` does not become a lock test by installing a desktop.

**Type:** Product ticket, empty until the plan exists. No architecture in this file.

**Status:** blocked on a Linux platform plan and on the open Linux entry. Not ready-for-agent. Not a GitHub issue.

- [ ] A Linux platform plan exists, and this ticket points at it instead of the phase-3 sketch
- [ ] Locking while one of the named remote views is connected covers that view
- [ ] An inbound connection to an already locked session shows the SoftLock screen
- [ ] Unlock adds no method the plan does not already have
- [ ] Remote input before unlock reaches only the unlock UI. The mechanism is the option Henry picked in the Linux remote-input entry
- [ ] A killed lock host, or both lock paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
