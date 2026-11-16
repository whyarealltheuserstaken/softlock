# l03: Remote session lock and unlock

**What to build:** Once a Linux platform plan exists, locking while xrdp, VNC, GNOME Remote Desktop, RustDesk, or a remote view of the same kind is connected covers that view. Connecting to a locked machine shows the SoftLock screen. Unlock is the plan's unlock (the sketch names PAM) through the same Broker nonce rule, with no weaker path. By default the remote viewer sees the SoftLock lock screen. The real screen appears on that view only after the configurable remote pass. Remote-tool input is blocked until that pass succeeds, except what the unlock screen needs. Local agent input still follows the normal rules. This ticket does not name a flag that separates that input from an agent's, and it does not add one. If l02 cannot tell them apart, that is a risk. The rule does not weaken. A killed lock host, or both lock paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked. That is invariant 3 in SPEC.md Decision 5, plus the known gap in section 7 until ticket 13, once a Linux plan has those states. Until the plan exists, this sentence does not invent them. X11 and Wayland are both in scope. This ticket does not write the plan.

**Blocked by:** a Linux platform plan, and l02. The owner decision is closed.

**Blocks:** l04 (the remote-view rows).

**Where:** Not a runner claim. `ubuntu-latest` does not become a lock test by installing a desktop.

**Type:** Product ticket, empty until the plan exists. No architecture in this file.

**Status:** post-v1, and still blocked on a Linux platform plan. Not ready-for-agent. Not a GitHub issue.

- [ ] A Linux platform plan exists, and this ticket points at it instead of the phase-3 sketch
- [ ] Locking while one of the named remote views is connected covers that view
- [ ] An inbound connection to an already locked session shows the SoftLock screen
- [ ] Unlock adds no method the plan does not already have
- [ ] Remote-tool input before the remote pass reaches only what the unlock screen needs. Local agent input still follows the plan's normal rule. If l02 could not tell those inputs apart, this box still holds
- [ ] A killed lock host, or both lock paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
