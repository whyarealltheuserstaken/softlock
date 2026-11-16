# m16: App hotkey and status, the first macOS tracer bullet

**What to build:** The settings app on macOS gains the lock hotkey and a status line, the same shape as Windows ticket 14. The hotkey asks the service to lock. The overlay is the lock host's, not a web view. The dev PIN unlocks. The demo is done when an agent-like process keeps making progress while the overlay is up, which is m07's result, and a person at the keyboard cannot type into that process.

Seam 4 stays the Linux Playwright job. This ticket does not move it.

**Blocked by:** m15 and m07.

**Blocks:** nothing in this set. m17 and m18 proceed in parallel once m15 is done. This ticket is the demo.

**Where:** Henry's Intel Mac. CI stays green, including seam 1 on `macos-latest` and the m01 scan.

**Test machine:** MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports). 2 GHz quad-core Intel Core i5. Intel Iris Plus Graphics. 16 GB LPDDR4X. macOS Tahoe 26.7.3. Laptop, with Touch ID and the T2 chip. The number of external displays is still unknown. macOS 11 through 15 are untested: this machine is on 26.x, and arm64 CI is `macos-latest`. Full list: D1 and [Henry's sheet](henry-sheet.md).

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] The hotkey locks, the overlay is up on every display his m06 sheet covered, and the dev PIN unlocks (checklist on his Mac)
- [ ] A posted input stream keeps reaching the target app while locked (CI if m02's runner can host it, otherwise the same checklist)
- [ ] The m07 timer process does not stall beyond what that finding allowed (checklist)
- [ ] Status shows locked and unlocked. Quit does not stop the daemon (checklist)
- [ ] The universal app binary still passes `lipo -info` and the release scan (m01 job)

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
