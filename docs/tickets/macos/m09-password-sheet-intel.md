# m09: Kit: the LAContext password sheet above the overlay

**What to build:** Henry runs a user-session probe, not a root daemon, that shows the overlay and calls `LAContext.evaluatePolicy` with `LAPolicy.deviceOwnerAuthentication`. He unlocks with his account password. If Touch ID is offered, he cancels that branch and uses the password. This ticket does not accept a Touch ID success as its result (that is m12). He records whether the system sheet appears above the overlay, whether keys typed for the password reach any other app, and what the tap does while the sheet is up (disable event, silence, or keys still delivered). He also tries the call from a root process and records the error if it refuses to show UI.

**Blocked by:** m05, so the tap's normal classification is already known and a change during the sheet is visible.

**Blocks:** m12, m18.

**Where:** Henry's Intel Mac.

**Type:** Hardware kit. Deliverable is the sheet under `docs/findings/`.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] Password success and password failure each have a row, and the boolean is the only result the probe logs. The password is not in the log (sheet)
- [ ] The sheet is above the overlay, or the sheet is recorded as hidden behind it (sheet)
- [ ] Keys during the sheet do not reach another app, or the sheet names the app that received them (sheet)
- [ ] A root-process attempt is recorded: UI appeared, or the error (sheet)
- [ ] Written finding tells m18 whether the Broker must be a GUI agent, and tells m12 whether this Mac has a Touch ID sensor

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
