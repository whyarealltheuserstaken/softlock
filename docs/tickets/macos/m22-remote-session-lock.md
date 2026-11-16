# m22: Remote session lock and unlock

**What to build:** Locking while Screen Sharing, VNC, Chrome Remote Desktop, TeamViewer, AnyDesk, or a remote view of the same kind is connected covers that view. Connecting to a Mac that is already SoftLocked shows the SoftLock screen on the remote view. Unlock is the Broker: a `Verified` verdict with the 32-byte `LaunchNonce`. Touch ID is offered where that remote stack supports it. Otherwise the account password or the dev PIN, the same methods as m18 and m19, and no weaker path. By default the remote viewer sees the SoftLock lock screen. The real screen appears on that view only after the configurable remote pass, a separate credential from the Broker. Remote-tool input is blocked until that pass succeeds, except what the unlock screen needs. Local agent input still follows the normal rules. If m21 finds Screen Sharing input arrives as a posted event, that is a risk this ticket implements around. The rule does not weaken. A killed lock host, or both D5 paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked.

**Blocked by:** m14, m17, m18, m19, and m21. D9 is closed. This ticket does not wait on a choice.

**Blocks:** m23 and m24 (the inbound Screen Sharing rows).

**Where:** The remote view itself is Henry's Intel Mac, recorded on m23. CI keeps the D8 chooser tests green and does not claim Screen Sharing on `macos-latest`.

**Test machine:** The paragraph in D1 of [`decisions.md`](decisions.md). [Henry's sheet](henry-sheet.md) points at the same paragraph.

**Type:** Product ticket. The hardware rows are m23, not a second copy here.

**Status:** post-v1. Not ready-for-agent. Not a GitHub issue. The display rows on m23 and m24 are not this ticket and are not post-v1.

- [ ] Locking while a remote view is connected covers that view with the SoftLock screen
- [ ] An inbound connection to an already locked Mac shows the SoftLock screen before desktop input
- [ ] SoftLock unlock accepts only a Broker `Verified` verdict. Touch ID is offered only where the remote stack supports it. Otherwise the account password or the dev PIN. No weaker path. The real screen on the remote view also requires the configurable remote pass
- [ ] Remote-tool input before that pass reaches only what the unlock screen needs, and local agent input still reaches another app. If m21 could not tell those inputs apart, this box still holds
- [ ] A killed lock host, or both D5 paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
