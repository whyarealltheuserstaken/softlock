# m12: Kit: Touch ID

**What to build:** The biometry branch of `LAPolicy.deviceOwnerAuthentication`, on a Mac that has a Touch ID sensor. The probe shows the overlay, calls `evaluatePolicy`, and he authenticates with Touch ID. The sheet records whether the system UI appears above the overlay and whether a finger press can be turned into a keystroke in another app. Password fallback is m09, not this ticket.

GitHub arm64 runners have no Touch ID sensor. An Intel Mac without a sensor cannot run this. Some 2018–2020 Intel portables have Touch ID via the T2 chip. If Henry's Mac is one of those, he runs this kit and the label below changes to Henry's Intel Mac. Until then the label stays Apple Silicon only, and the ticket waits on a machine the plan does not have.

**Blocked by:** m09, and a Mac with Touch ID.

**Blocks:** m19.

**Where:** Apple Silicon only. Not CI. Not an Intel Mac without a Touch ID sensor.

**Type:** Hardware kit. It does not block the Intel tracer bullet.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue. Blocked on hardware he has not identified.

- [ ] Touch ID success and cancel each have a row, and the log contains no biometric data (sheet)
- [ ] The Touch ID UI is above the overlay, or the sheet records that it was not (sheet)
- [ ] A finger on the sensor does not type into another app (sheet)
- [ ] The finding names the Mac model and says whether the "Apple Silicon only" label still applies

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
