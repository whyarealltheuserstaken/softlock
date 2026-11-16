# m10: Kit: Accessibility and Input Monitoring, no MDM

**What to build:** The permission path ticket m14's checklist will follow. No configuration profile and no MDM (decision D3, accepted by the lead). Henry resets `Accessibility` and `ListenEvent` with `tccutil` for the probe's bundle id, runs the swallowing session tap, and writes down which System Settings panes appear and which grants the tap actually needs before it can delete a physical key. An event tap that changes events usually needs only Accessibility. This spike confirms whether Input Monitoring is also needed. He rebuilds the ad-hoc binary once and records whether the grant survives. `PostEvent` is reset and recorded only if the probe prompted for it.

**Blocked by:** m05. D3 is accepted by the lead: MDM is out of scope, and this ticket does not wait on it.

**Blocks:** m14. The lock host is not tested on his Mac until this sheet says how the grants are made.

**Where:** Henry's Intel Mac. The runner's TCC result from m02 is context, not this ticket.

**Test machine:** MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports). 2 GHz quad-core Intel Core i5. Intel Iris Plus Graphics. 16 GB LPDDR4X. macOS Tahoe 26.7.3. Laptop, with Touch ID and the T2 chip. The number of external displays is still unknown. macOS 11 through 15 are untested: this machine is on 26.x, and arm64 CI is `macos-latest`. Full list: D1 and [Henry's sheet](henry-sheet.md).

**Type:** Hardware kit. Deliverable is the sheet, which becomes the wizard's permission page later. This ticket does not write the wizard.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] The sheet names each prompt that appeared, in order, and which grant was required before a physical key was swallowed (sheet)
- [ ] The sheet says whether Input Monitoring was required in addition to Accessibility, or whether Accessibility alone let the tap delete a physical key (sheet)
- [ ] An ad-hoc rebuild either keeps the grant or asks again (sheet). That row is the evidence behind decision D2
- [ ] No profile was installed. The finding says so
- [ ] Written finding is the steps m14's checklist cites

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
