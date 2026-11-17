# m12: Kit: Touch ID

**What to build:** The biometry branch of `LAPolicy.deviceOwnerAuthentication`, on a Mac that has a Touch ID sensor. The probe shows the overlay, calls `evaluatePolicy`, and he authenticates with Touch ID. The sheet records whether the system UI appears above the overlay and whether a finger press can be turned into a keystroke in another app. Password fallback is m09, not this ticket.

GitHub arm64 runners have no Touch ID sensor. His Intel Mac has Touch ID and the T2 chip, so he runs this kit on that machine. The Apple Silicon only label is off.

**Blocked by:** m09. The Mac with Touch ID is his.

**Blocks:** m19.

**Where:** Henry's Intel Mac. Not CI.

**Test machine:** The paragraph in D1 of [`decisions.md`](decisions.md). [Henry's sheet](henry-sheet.md) points at the same paragraph.

**Type:** Hardware kit. It does not block the Intel tracer bullet.

**Status:** waiting for Henry to run the kit. Not ready-for-agent. Not a GitHub issue.

- [ ] Touch ID success and cancel each have a row, and the log contains no biometric data (sheet)
- [ ] The Touch ID UI is above the overlay, or the sheet records that it was not (sheet)
- [ ] A finger on the sensor does not type into another app (sheet)
- [ ] The finding names this MacBook Pro and records the Touch ID result on it

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
