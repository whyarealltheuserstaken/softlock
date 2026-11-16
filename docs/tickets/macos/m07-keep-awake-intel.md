# m07: Kit: power assertions, App Nap, and the lid

**What to build:** Henry runs a probe that holds an idle-system-sleep assertion and an idle-display-sleep assertion, with a reason string he can find in `pmset -g assertions`. A second process increments a counter on a short timer. He runs it with the overlay down and with the overlay covering that process's window, for the same length of time, and writes both counts down. His Mac is a laptop, so he closes the lid on power and records whether it sleeps. The product path is `IOPMAssertionCreateWithName`, not a dependency on `caffeinate`. `caffeinate` is only a control if the probe finds it.

QA1340 says a lid close is forced sleep and an assertion cannot prevent it. This kit records what his hardware does. Phase 2 does not grow a lid-takeover feature from a single surprise result (decision D7).

**Blocked by:** m02, so the probe window and the assertion names that compile are already known.

**Blocks:** m16. The tracer demo does not claim agents kept running until this sheet exists.

**Where:** Henry's Intel Mac. It is a laptop, so the lid row is in scope. GitHub runners have no lid.

**Test machine:** MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports). 2 GHz quad-core Intel Core i5. Intel Iris Plus Graphics. 16 GB LPDDR4X. macOS Tahoe 26.7.3. Laptop, with Touch ID and the T2 chip. The number of external displays is still unknown. macOS 11 through 15 are untested: this machine is on 26.x, and arm64 CI is `macos-latest`. Full list: D1 and [Henry's sheet](henry-sheet.md).

**Type:** Hardware kit. Deliverable is the sheet and the `pmset` excerpt under `docs/findings/`.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] `pmset -g assertions` shows the probe's reason string and the assertion names that compiled (sheet)
- [ ] Timer counts with the overlay down and with it up, and a sentence that says whether the covered process slowed (sheet)
- [ ] Lid row: slept or stayed awake. This Mac is a laptop, so "no lid" is not the row (sheet)
- [ ] Power-key row, if m06 did not already record it: slept, showed the restart panel, or did nothing (sheet)
- [ ] Written finding names m16 and says whether keep-awake may be claimed for an idle sleep only

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
