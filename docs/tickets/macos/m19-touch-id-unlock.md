# m19: Unlock with Touch ID through the same Broker

**What to build:** The same Broker and the same nonce as m18, accepting a Touch ID success from `LAPolicy.deviceOwnerAuthentication` as `Verified`. No second unlock channel. The password fallback stays m18's path. His Intel Mac has Touch ID, so this ticket runs there. It is not started on a GitHub runner.

**Blocked by:** m12 and m18.

**Blocks:** nothing in this set.

**Where:** Henry's Intel Mac. Not CI. Off the first-demo critical path.

**Test machine:** MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports). 2 GHz quad-core Intel Core i5. Intel Iris Plus Graphics. 16 GB LPDDR4X. macOS Tahoe 26.7.3. Laptop, with Touch ID and the T2 chip. The number of external displays is still unknown. macOS 11 through 15 are untested: this machine is on 26.x, and arm64 CI is `macos-latest`. Full list: D1 and [Henry's sheet](henry-sheet.md).

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue. Off the first-demo critical path.

- [ ] Touch ID unlocks, and the nonce check is the same one m18 tests in CI (checklist on a Mac with Touch ID, plus the existing nonce test)
- [ ] Cancel falls through to the password sheet and does not unlock (checklist)
- [ ] The lock host still has no verdict of its own (CI)
- [ ] The finding names this MacBook Pro and records the Touch ID unlock on it

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
