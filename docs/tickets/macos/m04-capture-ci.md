# m04: Spike: sharingType none against the capture paths

**What to build:** On `macos-latest`, a borderless window with a known marker colour. Capture it with `sharingType` left at the default and with `sharingType` set to `.none`. Count marker pixels in the current display-image API, in `screencapture`, and in a ScreenCaptureKit stream of the display that does not pass an excluding-windows filter. Do not use `CGShieldingWindowLevel()`. The window level under test is `NSWindow.Level.screenSaver`, with `canJoinAllSpaces` and `fullScreenAuxiliary`, and `ignoresMouseEvents` set. Print `sw_vers`.

Apple's current `NSWindow.SharingType.none` page says not to use that value to hide content from capture. This spike measures what the OS on the runner actually does. A zero count is a path we may claim. A non-zero count is a path we do not claim (decision D6).

**Blocked by:** m02.

**Blocks:** m06, m14.

**Where:** CI only. One virtual display. It does not prove a second monitor, a Space switch, or a full-screen app. Those are m06.

**Type:** Prototype spike. Deliverable is a written finding plus the proof job.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] The proof job prints marker pixel counts for each capture path with sharing type default and with `.none` (job log)
- [ ] The proof job names the display-image API it actually called on that OS (job log)
- [ ] ScreenCaptureKit is either measured or reported unavailable, with the error (job log)
- [ ] Written finding lists which paths, if any, omit the marker, and tells m14 which of those paths it may claim

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
