# m09: Kit: the LAContext password sheet above the overlay

**What to build:** Henry runs a user-session probe, not a root daemon, that shows the overlay and calls `LAContext.evaluatePolicy` with `LAPolicy.deviceOwnerAuthentication`. He unlocks with his account password. If Touch ID is offered, he cancels that branch and uses the password. This ticket does not accept a Touch ID success as its result (that is m12). He records whether the system sheet appears above the overlay, whether keys typed for the password reach any other app, and what the tap does while the sheet is up (disable event, silence, or keys still delivered). He also tries the call from a root process and records the error if it refuses to show UI.

**Blocked by:** m05, so the tap's normal classification is already known and a change during the sheet is visible.

**Blocks:** m12, m18.

**Where:** Henry's Intel Mac.

**Test machine:** MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports). 2 GHz quad-core Intel Core i5. Intel Iris Plus Graphics. 16 GB LPDDR4X. macOS Tahoe 26.7.3. Laptop, with Touch ID and the T2 chip, so the password sheet may offer Touch ID. This ticket still uses the password. The number of external displays is still unknown. macOS 11 through 15 are untested: this machine is on 26.x, and arm64 CI is `macos-latest`. Full list: D1 and [Henry's sheet](henry-sheet.md).

**Type:** Hardware kit. Deliverable is the sheet under `docs/findings/`.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] Password success and password failure each have a row, and the boolean is the only result the probe logs. The password is not in the log (sheet)
- [ ] The sheet is above the overlay, or the sheet is recorded as hidden behind it (sheet)
- [ ] Keys during the sheet do not reach another app, or the sheet names the app that received them (sheet)
- [ ] A root-process attempt is recorded: UI appeared, or the error (sheet)
- [ ] Written finding tells m18 whether the Broker must be a GUI agent. This Mac has a Touch ID sensor; the finding says whether the sheet offered it

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
