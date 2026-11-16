# m22: Remote session lock and unlock

**What to build:** Locking while Screen Sharing, VNC, Chrome Remote Desktop, TeamViewer, AnyDesk, or a remote view of the same kind is connected covers that view. Connecting to a Mac that is already SoftLocked shows the SoftLock screen on the remote view. Unlock is the Broker: a `Verified` verdict with the 32-byte `LaunchNonce`. Touch ID is offered where that remote stack supports it. Otherwise the account password or the dev PIN, the same methods as m18 and m19, and no weaker path. Keys and clicks from the remote view before unlock are the open remote-input entry in Decision 20. Screen Sharing input arrives as posted events, and this ticket does not claim the tap can tell them from an agent's. A killed lock host, or both D5 paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked. The surface that stays visible to the remote viewer is the option Henry picks in D9 (SPEC.md Decision 20). Option A there is likely infeasible. This ticket does not pick.

**Blocked by:** m14, m17, m18, m19, m21, and D9, including the macOS remote-input entry. It stays blocked while D9 is open.

**Blocks:** m23 and m24 (the inbound Screen Sharing rows).

**Where:** The remote view itself is Henry's Intel Mac, recorded on m23. CI keeps the D8 chooser tests green and does not claim Screen Sharing on `macos-latest`.

**Test machine:** The paragraph in D1 of [`decisions.md`](decisions.md). [Henry's sheet](henry-sheet.md) points at the same paragraph.

**Type:** Product ticket. The hardware rows are m23, not a second copy here.

**Status:** blocked on D9. Not ready-for-agent. Not a GitHub issue.

- [ ] Locking while a remote view is connected covers that view with the SoftLock screen
- [ ] An inbound connection to an already locked Mac shows the SoftLock screen before desktop input
- [ ] Unlock accepts only a Broker `Verified` verdict. Touch ID is offered only where the remote stack supports it. Otherwise the account password or the dev PIN. No weaker path
- [ ] Remote input before unlock reaches the unlock UI and does not reach another app
- [ ] A killed lock host, or both D5 paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
