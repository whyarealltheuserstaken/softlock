# m21: Spike: sharingType none versus a remote view

**What to build:** On `macos-latest`, the m04 marker window with `sharingType` at the default and at `.none`, at `NSWindow.Level.screenSaver` with `canJoinAllSpaces` and `fullScreenAuxiliary`. Count the marker on the m04 paths, then on each remote-view path the runner can open: Screen Sharing, a VNC viewer, AirPlay, TeamViewer, AnyDesk, and Chrome Remote Desktop. If a path cannot be opened, the log says so and does not invent a count. This ticket does not install those programs. Where a path does open, record whether its input arrives as a posted event the tap cannot tell from an agent's `CGEventPost`. DisplayLink's [macOS page](https://support.displaylink.com/knowledgebase/articles/1932214-displaylink-manager-app-for-macos-introduction-in) says the driver needs Screen Recording permission to read pixels and send them over USB. The runner has no DisplayLink device, so that path is a written miss here and a row on m23 if Henry has the device. Apple's `NSWindow.SharingType.none` page says the value is a legacy constant macOS no longer uses, and not to use it to hide content from capture. Screen Sharing and a local capture read the same window-server image. Henry's Decision 20 still requires the remote viewer to see the lock until the configurable pass, and local agent capture still follows the normal rule. This spike measures the gap. It does not weaken either half.

**Blocked by:** m04, for the probe (v1.x/v2).

**Blocks:** m22's v1.x/v2 slice

**Where:** CI only. Screen Sharing into Henry's Mac is m23.

**Type:** Prototype spike. Deliverable is a written finding plus the proof job.

**v1 framework:** The detection-hook result type exists, including a field for "remote input arrived as a posted event the tap cannot tell from an agent's." A seam test feeds a fake. v1 does not open Screen Sharing and does not enable the feature.

**v1.x or v2:** The checkboxes below. A posted event the tap cannot tell from an agent's is a risk. The rule does not weaken.

**Status:** v1 framework plus a v1.x or v2 probe. Not ready-for-agent. Not a GitHub issue. D9 is closed in SPEC.md Decision 20.

- [ ] v1: the detection-hook result type has the posted-event risk field, set from a fake. No remote view is opened (seam 1)
- [ ] v1.x/v2: Marker counts for each m04 path, sharing type default and `.none`, on this OS (job log)
- [ ] v1.x/v2: Screen Sharing, VNC, AirPlay, TeamViewer, AnyDesk, and Chrome Remote Desktop each have the same pair of counts, or the log names that path unavailable (job log)
- [ ] v1.x/v2: Where a path was available, the log says whether its input arrived as a posted event (job log)
- [ ] v1.x/v2: DisplayLink is recorded as unavailable on the runner (job log)
- [ ] v1.x/v2: Written finding lists which paths omitted the marker and which showed it, and names any remote input that arrived as a posted event. It does not recommend letting that input through, and it does not recommend blocking local agent input

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
