# l03: Remote session lock and unlock

**What to build:** Once a Linux platform plan exists, locking while xrdp, VNC, GNOME Remote Desktop, RustDesk, or a remote view of the same kind is connected covers that view. Connecting to a locked machine shows the SoftLock screen. Unlock is the plan's unlock (the sketch names PAM) through the same Broker nonce rule, with no weaker path. By default the remote viewer sees the SoftLock lock screen. The real screen appears on that view only after the configurable remote pass. Remote-tool input is blocked until that pass succeeds, except what the unlock screen needs. Local agent input still follows the normal rules. This ticket does not name a flag that separates that input from an agent's, and it does not add one. If l02 cannot tell them apart, that is a risk. The rule does not weaken. A killed lock host, or both lock paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked. That is invariant 3 in SPEC.md Decision 5, plus the known gap in section 7 until ticket 13, once a Linux plan has those states. Until the plan exists, this sentence does not invent them. X11 and Wayland are both in scope. This ticket does not write the plan.

**v1 framework:** Once a Linux plan exists, the same three seams as ticket 38: a detection hook, the remote-pass setting and policy types, and fail-closed with a fake remote session. This slice does not choose a compositor or a capture API, and it does not ship the feature.

**v1.x or v2:** A real xrdp, VNC, GNOME Remote Desktop, or RustDesk view. The checkboxes marked v1.x/v2 are this slice.

**Blocked by:** a Linux platform plan for both slices. The v1.x or v2 slice also waits on l02. The owner decision is closed.

**Blocks:** l04 (the remote-view rows).

**Where:** Not a runner claim. `ubuntu-latest` does not become a lock test by installing a desktop.

**Type:** Product ticket. The v1 slice is the three seams. It is not a Linux architecture, and it is empty until the plan exists.

**Status:** v1 framework plus a v1.x or v2 implementation, both still blocked on a Linux platform plan. Not ready-for-agent. Not a GitHub issue.

- [ ] v1: a Linux platform plan exists, and this ticket points at it instead of the phase-3 sketch
- [ ] v1: the detection hook and the remote-pass policy type exist. The default is that a remote viewer does not see the real screen. The setting is not enabled as a shipped feature
- [ ] v1: with a fake remote session, a killed lock host, or both lock paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked
- [ ] v1.x/v2: Locking while one of the named remote views is connected covers that view
- [ ] v1.x/v2: An inbound connection to an already locked session shows the SoftLock screen
- [ ] v1.x/v2: Unlock adds no method the plan does not already have. The real screen on the remote view also requires the remote pass
- [ ] v1.x/v2: Remote-tool input before the remote pass reaches only what the unlock screen needs. Local agent input still follows the plan's normal rule. If l02 could not tell those inputs apart, this box still holds

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
