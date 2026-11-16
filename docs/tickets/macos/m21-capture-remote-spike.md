# m21: Spike: sharingType none versus a remote view

**What to build:** On `macos-latest`, the m04 marker window with `sharingType` at the default and at `.none`, at `NSWindow.Level.screenSaver` with `canJoinAllSpaces` and `fullScreenAuxiliary`. Count the marker on the m04 paths, then on any remote-view path the runner can open (Screen Sharing, a VNC viewer, or an AirPlay receiver). If none of those can be opened, the log says so and does not invent a count. DisplayLink's [macOS page](https://support.displaylink.com/knowledgebase/articles/1932214-displaylink-manager-app-for-macos-introduction-in) says the driver needs Screen Recording permission to read pixels and send them over USB. The runner has no DisplayLink device, so that path is a written miss here and a row on m23 if Henry has the device. Apple's `NSWindow.SharingType.none` page says not to use that value to hide content from capture. This spike measures. It does not choose D9.

**Blocked by:** m04.

**Blocks:** m22, m23.

**Where:** CI only. Screen Sharing into Henry's Mac is m23.

**Type:** Prototype spike. Deliverable is a written finding plus the proof job.

**Status:** waiting for Henry to approve the macOS set. Not ready-for-agent. Not a GitHub issue. Does not wait on D9. Does not decide it.

- [ ] Marker counts for each m04 path, sharing type default and `.none`, on this OS (job log)
- [ ] Each remote-view path the runner could open has the same pair of counts, or the log names the path unavailable (job log)
- [ ] DisplayLink is recorded as unavailable on the runner (job log)
- [ ] Written finding lists which paths omitted the marker and which showed it, and tells m22 which D9 options the measurement still leaves open

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
