# m22: Remote session lock and unlock

**What to build:** Locking while Screen Sharing, VNC, Chrome Remote Desktop, TeamViewer, AnyDesk, or a remote view of the same kind is connected covers that view. Connecting to a Mac that is already SoftLocked shows the SoftLock screen on the remote view. Unlock is the Broker: a `Verified` verdict with the 32-byte `LaunchNonce`. Touch ID is offered where that remote stack supports it. Otherwise the account password or the dev PIN, the same methods as m18 and m19, and no weaker path. By default the remote viewer sees the SoftLock lock screen. The real screen appears on that view only after the configurable remote pass, a separate credential from the Broker. Remote-tool input is blocked until that pass succeeds, except what the unlock screen needs. Local agent input still follows the normal rules. If m21 finds Screen Sharing input arrives as a posted event, that is a risk this ticket implements around. The rule does not weaken. A killed lock host, or both D5 paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked. m14 and m22 apply `.none` only when the m22 detection hook positively reports no remote session. Unknown or failed detection means no `.none`. That holds until the remote pass.

**v1 framework:** The same three seams as ticket 38: a detection hook, the remote-pass setting and policy types, and fail-closed with a fake remote session so the state is not Unlocked. v1 does not open Screen Sharing and does not ship the feature. Here **v1** is macOS phase 2's first release (SPEC.md Decision 20), not the Windows v1 ticket set.

**v1.x or v2:** The real remote view, the pass on that view, and remote-tool input. If m21 finds posted events, that risk is implemented here. The rule does not weaken.

**Blocked by:** m15 and m17 for the v1 framework. The v1.x or v2 slice also waits on m14, m18, m19, and m21. D9 is closed.

**Blocks:** m23 and m24 (the inbound Screen Sharing rows).

**Where:** The remote view itself is Henry's Intel Mac, recorded on m23. CI keeps the D8 chooser tests green and does not claim Screen Sharing on `macos-latest`.

**Test machine:** The paragraph in D1 of [`decisions.md`](decisions.md). [Henry's sheet](henry-sheet.md) points at the same paragraph.

**Type:** Product ticket. The hardware rows are m23, not a second copy here.

**Status:** v1 framework, plus a v1.x or v2 implementation. Not ready-for-agent. Not a GitHub issue. The display rows on m23 and m24 are not this ticket.

- [ ] v1: the detection hook and the remote-pass policy type exist. The default is that a remote viewer does not see the real screen. The setting is not enabled as a shipped feature (seam 1)
- [ ] v1: with a fake remote session, a killed lock host, or both D5 paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked (seam 1)
- [ ] v1.x/v2: Locking while a remote view is connected covers that view with the SoftLock screen
- [ ] v1.x/v2: An inbound connection to an already locked Mac shows the SoftLock screen before desktop input
- [ ] v1.x/v2: SoftLock unlock accepts only a Broker `Verified` verdict. Touch ID is offered only where the remote stack supports it. Otherwise the account password or the dev PIN. No weaker path. The real screen on the remote view also requires the configurable remote pass
- [ ] v1.x/v2: Remote-tool input before that pass reaches only what the unlock screen needs, and local agent input still reaches another app. If m21 could not tell those inputs apart, this box still holds

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
