# m21: Spike: sharingType none versus a remote view

**What to build:** On `macos-latest`, the m04 marker window with `sharingType` at the default and at `.none`, at `NSWindow.Level.screenSaver` with `canJoinAllSpaces` and `fullScreenAuxiliary`. Count the marker on the m04 paths, then on each remote-view path the runner can open: Screen Sharing, a VNC viewer, AirPlay, TeamViewer, AnyDesk, and Chrome Remote Desktop. If a path cannot be opened, the log says so and does not invent a count. This ticket does not install those programs. Where a path does open, record whether its input arrives as a posted event the tap cannot tell from an agent's `CGEventPost`. DisplayLink's [macOS page](https://support.displaylink.com/knowledgebase/articles/1932214-displaylink-manager-app-for-macos-introduction-in) says the driver needs Screen Recording permission to read pixels and send them over USB. The runner has no DisplayLink device, so that path is a written miss here and a row on m23 if Henry has the device. Apple's `NSWindow.SharingType.none` page says the value is a legacy constant macOS no longer uses, and not to use it to hide content from capture. Decision 20 marks macOS option A likely infeasible for that reason: Screen Sharing and a local capture read the same window-server image, and `.none` does not split it. This spike measures. It does not choose D9, and a measurement does not by itself make A feasible. The agent-side ScreenCaptureKit filter is option D in that decision. This spike does not implement it.

**Blocked by:** m04.

**Blocks:** m22, m23.

**Where:** CI only. Screen Sharing into Henry's Mac is m23.

**Type:** Prototype spike. Deliverable is a written finding plus the proof job.

**Status:** waiting for Henry to approve the macOS set. Not ready-for-agent. Not a GitHub issue. Does not wait on D9. Does not decide it.

- [ ] Marker counts for each m04 path, sharing type default and `.none`, on this OS (job log)
- [ ] Screen Sharing, VNC, AirPlay, TeamViewer, AnyDesk, and Chrome Remote Desktop each have the same pair of counts, or the log names that path unavailable (job log)
- [ ] Where a path was available, the log says whether its input arrived as a posted event (job log)
- [ ] DisplayLink is recorded as unavailable on the runner (job log)
- [ ] Written finding lists which paths omitted the marker and which showed it, and tells m22 which D9 options the measurement still leaves open. Option A stays marked likely infeasible unless the finding shows the window-server image actually splits

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
