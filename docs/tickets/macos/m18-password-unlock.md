# m18: Unlock with the Mac account password through the Broker

**What to build:** Replace the dev PIN as the owner-facing unlock with `LAPolicy.deviceOwnerAuthentication`, evaluated in the Broker, which is the user-session process m09 said can show the sheet. The service launches that Broker with a new `LaunchNonce` and accepts `Verified` only from that launch. The lock host never sees the password. Backoff stays the Core policy. Touch ID, if the sheet offers it, is not this ticket's success path. He uses the password. Touch ID is m19.

**Blocked by:** m09 and m15.

**Blocks:** m19.

**Where:** Henry's Intel Mac.

**Test machine:** MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports). 2 GHz quad-core Intel Core i5. Intel Iris Plus Graphics. 16 GB LPDDR4X. macOS Tahoe 26.7.3. Laptop, with Touch ID and the T2 chip. He unlocks this ticket with the account password. The number of external displays is still unknown. macOS 11 through 15 are untested: this machine is on 26.x, and arm64 CI is `macos-latest`. Full list: D1 and [Henry's sheet](henry-sheet.md).

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] The correct account password unlocks. A wrong one does not, and the delay matches the Core backoff (checklist)
- [ ] The log and the socket capture contain no password (CI for the socket test, checklist for the sheet)
- [ ] A `Verified` with the wrong nonce does not unlock (CI)
- [ ] While the sheet is up, a physical key does not reach another app, matching the m09 finding (checklist)
- [ ] The Broker is not running as root if m09 said a root call cannot show UI (checklist)

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
