# m06: Kit: overlay level, Spaces, full-screen, every display

**What to build:** Henry runs the m04 probe on his Intel Mac, native x86_64. He records the marker counts for the same capture paths on his OS version. He switches Spaces, opens a full-screen app, and looks at every display he has, including one he plugs in while the windows are up if he has a second display. He checks that a click falls through (`ignoresMouseEvents`) and that the previously focused window keeps keyboard focus. He tries Command-Tab, Command-Option-Escape, and the power key, and records whether the presentation options did anything while the probe was not the active app.

**Blocked by:** m04.

**Blocks:** m14.

**Where:** Henry's Intel Mac. A one-display Mac does not prove hot-plug. The sheet says how many displays were attached.

**Type:** Hardware kit. Deliverable is the filled sheet and the probe log under `docs/findings/`.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] Marker counts on his OS for each capture path, sharing type default and `.none` (sheet)
- [ ] The overlay is visible on every display that was attached, and a display plugged in during the run is either covered or recorded as a miss (sheet)
- [ ] A Space switch and one full-screen app: the overlay stayed up, or the sheet names which one covered it (sheet)
- [ ] A click reached the window under the overlay, and keyboard focus did not move (sheet)
- [ ] Command-Tab, Force Quit, and the power key each have a row: swallowed, presentation option worked, or the desktop was reachable (sheet)
- [ ] Written finding tells m14 which capture paths it may claim on this OS (decision D6)

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
