# m06: Kit: overlay level, Spaces, full-screen, every display

**What to build:** Henry runs the m04 probe on his Intel Mac, native x86_64. He records the marker counts for the same capture paths on his OS version. He switches Spaces, opens a full-screen app, and looks at every display already attached when the probe starts. He checks that a click falls through (`ignoresMouseEvents`) and that the previously focused window keeps keyboard focus. He tries Command-Tab, Command-Option-Escape, and the power key, and records whether the presentation options did anything while the probe was not the active app. A display plugged in during the run, a closed lid with an external display, AirPlay, Sidecar, DisplayLink, and inbound Screen Sharing are m23. This ticket does not repeat those rows.

**Blocked by:** m04.

**Blocks:** m14, m23.

**Where:** Henry's Intel Mac. The displays already attached at the start. Hot-plug is m23. The sheet says how many displays were attached at the start. The machine paragraph is D1.

**Test machine:** The paragraph in D1 of [`decisions.md`](decisions.md). [Henry's sheet](henry-sheet.md) points at the same paragraph.

**Type:** Hardware kit. Deliverable is the filled sheet and the probe log under `docs/findings/`.

**Status:** kit in tree. Waiting for Henry's Intel Mac run. GitHub issue #55.

- [ ] Marker counts on his OS for each capture path, sharing type default and `.none` (sheet)
- [ ] The overlay is visible on every display that was already attached when the probe started (sheet). A display plugged in during the run is m23
- [ ] A Space switch and one full-screen app: the overlay stayed up, or the sheet names which one covered it (sheet)
- [ ] A click reached the window under the overlay, and keyboard focus did not move (sheet)
- [ ] Command-Tab, Force Quit, and the power key each have a row: swallowed, presentation option worked, or the desktop was reachable (sheet)
- [ ] Written finding tells m14 which capture paths it may claim on this OS (decision D6)

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
