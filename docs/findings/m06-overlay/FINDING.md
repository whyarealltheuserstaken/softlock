# m06: overlay level, Spaces, full-screen, every display

The kit is the workspace-excluded probe `spikes/macos-overlay-kit`. It draws one borderless window per `NSScreen` at `NSScreenSaverWindowLevel` with `canJoinAllSpaces` and `fullScreenAuxiliary`. The windows ignore the mouse and cannot become key. `CGShieldingWindowLevel()` is not called. Teardown paths: Control-C / SIGINT, SIGHUP, SIGTERM, panic, the `--timeout-ms` watchdog (default 10 minutes), and Escape when Accessibility is granted. Escape is not claimed without Accessibility because the kit is an Accessory app. Control-C is honoured during prompts by polling stdin every 100 ms while pumping the run loop. The power-key row runs last, after the overlay is gone; on a Touch ID MacBook Pro that tap can lock or sleep the machine.

This finding does not let m14 claim a capture path. Decision D6: m14 may claim a path only after this Intel kit and the m04 runner agree that the marker left the image. Until Henry fills the sheet on the D1 machine, every path stays `do-not-claim`, including a runner row that prints `removed=true`.

## Runner

CI runs `--runner` on `macos-latest` (arm64), `macos-15-intel`, and `macos-26-intel`. That slice covers every attached display the VM has (one virtual panel), records marker counts for default sharing and `.none` on `CGWindowListCreateImage`, `CGDisplayCreateImage`, `screencapture`, ScreenCaptureKit (`SCScreenshotManager` with an empty exclude list), and one `SCStream` frame, and checks posted click/focus when Accessibility is granted. Without `CGPreflightScreenCaptureAccess` the capture rows are incomplete and never pass. It does not switch Spaces, open a full-screen app, tap Command-Tab, Force Quit, or the power key, or plug in a display. Those rows are `not-run-on-runner`. After `overlay.sync()` cover count is compared to screen count.

Paste the runner `result.txt` summary here after the first green job. Do not paste a hardware serial number.

## Henry's Intel Mac

The machine is the D1 paragraph in `docs/tickets/macos/decisions.md` and [Henry's sheet](../../tickets/macos/henry-sheet.md): MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports), Intel Core i5, macOS Tahoe 26.7.3, laptop, Touch ID, T2. Henry sometimes uses an external display. How many are attached at once is still unknown. macOS 11 through 15 stay untested on his hardware; `macos-15-intel` is a separate x86_64 point.

A one-display run does not prove hot-plug. Type `no-external-display` when a second panel was never attached. The kit rejects that skip if `max-screens` is greater than 1. Do not label a genuine one-display skip `ci` or `operator`.

## Remote viewer

Local capture is story 20. Decision 20 is unchanged: a remote viewer sees the lock until a separate configurable pass. v1 does not ship that pass. This kit does not run Screen Sharing, AirPlay, Sidecar, or DisplayLink. Those rows are m23 / m21 / m22.

## m14

| Path | Default | `.none` | Baseline | Removed on this OS | m14 |
|---|---:|---:|---:|---|---|
| `CGWindowListCreateImage` | | | | | do-not-claim |
| `CGDisplayCreateImage` | | | | | do-not-claim |
| `screencapture` | | | | | do-not-claim |
| ScreenCaptureKit | | | | | do-not-claim |
| `SCStream` | | | | | do-not-claim |

Fill the counts from Henry's `result.txt`. A path whose `.none` count is still the window stays unclaimed. Apple's page still says not to use `NSWindow.SharingType.none` to hide content.

Instructions: [spikes/macos-overlay-kit/README.md](../../../spikes/macos-overlay-kit/README.md).
