# m22: Remote session lock and unlock

**What to build:** Locking while Screen Sharing, VNC, Chrome Remote Desktop, or a remote view of the same kind is connected covers that view. Connecting to a Mac that is already SoftLocked shows the SoftLock screen on the remote view. Unlock is the Broker: a `Verified` verdict with the 32-byte `LaunchNonce`. Touch ID is offered where that remote stack supports it. Otherwise the account password or the dev PIN, the same methods as m18 and m19, and no weaker path. Keys and clicks from the remote view before unlock reach only the unlock UI. A crash or any other failure takes the D5 path and then D8: the session does not end Unlocked. The surface that stays visible to the remote viewer is the option Henry picks in D9 (SPEC.md Decision 20). This ticket does not pick it.

**Blocked by:** m14, m18, m19, m21, and D9. It stays blocked while D9 is open.

**Blocks:** m23 (the inbound Screen Sharing row).

**Where:** The remote view itself is Henry's Intel Mac, recorded on m23. CI keeps the D8 chooser tests green and does not claim Screen Sharing on `macos-latest`.

**Test machine:** MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports). 2 GHz quad-core Intel Core i5. Intel Iris Plus Graphics. 16 GB LPDDR4X. macOS Tahoe 26.7.3. Laptop, with Touch ID and the T2 chip. He sometimes uses an external display. How many are attached at once is still unknown. Whether a second keyboard is available is still unknown. macOS 11 through 15 are untested: this machine is on 26.x, and arm64 CI is `macos-latest`. Full list: D1 and [Henry's sheet](henry-sheet.md).

**Type:** Product ticket. The hardware rows are m23, not a second copy here.

**Status:** blocked on D9. Not ready-for-agent. Not a GitHub issue.

- [ ] Locking while a remote view is connected covers that view with the SoftLock screen
- [ ] An inbound connection to an already locked Mac shows the SoftLock screen before desktop input
- [ ] Unlock accepts only a Broker `Verified` verdict. Touch ID is offered only where the remote stack supports it. Otherwise the account password or the dev PIN. No weaker path
- [ ] Remote input before unlock reaches the unlock UI and does not reach another app
- [ ] A killed lock host, or both D5 paths failing, keeps the guard up or reaches the real lock, and the state is not Unlocked

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
