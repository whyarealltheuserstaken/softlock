# 38: Remote session lock and unlock

**What to build:** On Windows 10 and on Windows 11, when a remote terminal is attached, locking covers that remote view. The requirement is the same on both. The Windows 10 versus Windows 11 list (RDP, Hello, Entra) is the one in SPEC.md Decision 20. This ticket does not copy it. A disconnect from the Entra lock-screen limit, which that list records, is not Unlocked. The runners are Server SKUs. The client proof is tickets 39 and 40. Connecting to a machine that is already SoftLocked shows the SoftLock screen on the remote view. Unlock is the existing Broker: a `Verified` verdict with the nonce from the Broker launch this service started. The methods are the SoftLock PIN, Windows Hello where that remote stack supports it, and otherwise the PIN or the account password. No weaker path. Where the stack in front of the person does not support Hello, this ticket uses the PIN or the account password. By default the remote viewer sees the SoftLock lock screen. The real screen appears on that view only after the configurable remote pass, a separate credential from the Broker. Remote-tool input is blocked until that pass succeeds, except what the unlock screen needs. Local agent input still follows the normal rules. If ticket 37 finds that TeamViewer, AnyDesk, Chrome Remote Desktop, or an RDP shadow shares the agent's injected flag, that is a risk this ticket implements around. The rule does not weaken. A killed lock host, or both OS-lock attempts failing, keeps the guard up or reaches the real lock, and the state is not Unlocked. That wording is invariant 3 in Decision 5, plus the known gap in section 7 until ticket 13. A crash or failure ends locked.

**v1 framework:** A remote-session detection hook the state machine can call, the configurable remote-pass setting and its policy types (default: a remote viewer does not see the real screen), and the fail-closed path. A seam test feeds a fake connected session. A crash or failure with that fake ends locked: the state is not Unlocked. v1 does not detect a real remote tool, does not draw the pass on a remote view, and does not ship the feature.

**v1.x or v2:** Covering a real remote view, the pass UI, blocking real remote-tool input, and the client-SKU proof. The checkboxes that say v1.x/v2 are this slice. If ticket 37 finds a shared injected flag, that risk is implemented here. The rule does not weaken.

**Blocked by:** 15, 21, and 22 for the v1 framework. The v1.x or v2 slice also waits on 37. The owner decision is closed.

**Blocks:** 39 and 40 (the inbound RDP rows, which are v1.x or v2)

**Needs Henry's hardware:** The client-SKU case, where the console terminal shows the login screen and the remote terminal has the session, is ticket 39. CI covers the fail-closed rule and the Broker verdict with the test-only nonce. CI does not claim a second physical machine.

**Status:** v1 framework, plus a v1.x or v2 implementation. Not a GitHub issue. Not ready-for-agent. The display rows on 39 and 40 are not this ticket.

- [ ] v1: the detection hook reports a fake remote session, and the remote-pass policy type defaults to "remote viewer does not see the real screen." The setting is not enabled as a shipped feature (seam 1)
- [ ] v1: with that fake session, a killed lock host, or both OS-lock attempts failing, keeps the guard up or reaches the real lock, and the state is not Unlocked (seam 1)
- [ ] v1.x/v2: Locking while a remote terminal is connected covers that view (seam 3 if 37 found a remote path on the runner; otherwise the row is ticket 39 and this box says so)
- [ ] v1.x/v2: Connecting to an already locked session shows the SoftLock screen on the remote view before any desktop input
- [ ] v1.x/v2: Unlock accepts only a Broker `Verified` verdict. Hello is offered only where 37's stack supported it. Otherwise the PIN or the account password. The real screen on the remote view also requires the remote pass. A path with neither is a failure, not a success
- [ ] v1.x/v2: Remote-tool input before the remote pass reaches only what the unlock screen needs, and local agent input still reaches the probe window. If 37 could not tell those inputs apart, this box still holds, and the finding names the risk

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
