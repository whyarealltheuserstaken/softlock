# m04: sharingType none removed the marker window

Measured on 16 November 2026 UTC by the workspace-excluded probe `spikes/macos-desktop-proof`.

The proof run is [macOS desktop proof run 44010674202](https://github.com/whyarealltheuserstaken/softlock/actions/runs/44010674202), job [macos desktop proof](https://github.com/whyarealltheuserstaken/softlock/actions/runs/44010674202/job/133796661508). The measured head is `5faa4f7660b9f58b25fbf89bae3599c601426056`. The job printed `result=pass`. macOS 26.7.2, arm64, one 1024x768 display. `CGShieldingWindowLevel()` was not called. The window stayed visible, unoccluded, and listed by `CGWindowListCopyWindowInfo` after `sharingType` was set to `.none`.

```
ProductName:	macOS
ProductVersion:	26.7.2
BuildVersion:	25G332
display-image-api=CGWindowListCreateImage,CGDisplayCreateImage
sharing-presence=visible=true occlusion=true listed=true area=76800
```

The display-image calls were `CGWindowListCreateImage` and `CGDisplayCreateImage`. ScreenCaptureKit was `SCScreenshotManager` with an empty `excludingWindows` list, plus one `SCStream` frame of the display with the same empty list.

The marker window is 320 by 240, 76800 pixels. `baseline` is the same capture with both windows ordered out. `control` is a second window, a different colour, left at the default sharing type, and it is only on screen for the `.none` captures.

On this runner `.none` removed the window from every measured path. Apple's page says not to rely on that value to hide content. The counts still say the window left the image. m14 may not claim any path until m06 on Henry's Intel Mac agrees, so every row stays `do-not-claim`.

| Path | Default | `.none` | Baseline | Control in `.none` | Removed |
|---|---:|---:|---:|---:|---|
| `CGWindowListCreateImage` | 76834 | 34 | 34 | 77615 | yes |
| `CGDisplayCreateImage` | 76834 | 34 | 34 | 77615 | yes |
| `screencapture` | 76834 | 34 | 34 | 77615 | yes |
| ScreenCaptureKit | 76834 | 34 | 34 | 77615 | yes |
| `SCStream` | 76831 | 34 | 31 | 77615 | yes |

The four still images match the baseline exactly. The `SCStream` frame is 3 predicate pixels off the ordered-out frame (34 against 31). That is not the window: default minus baseline is 76800, the window area. The comparison allows 16 such pixels. A window that stayed in the image would miss by tens of thousands.

The control window is in every `.none` capture (77615 green pixels). The ordered-out frames have 815 of that colour, and the `SCStream` ordered-out frame has 813. 77615 is the baseline control count plus the window area, within the 16-pixel noise.

## Remote viewer

Local capture is story 20: an agent's screenshot. On this runner `.none` took the marker window out of those screenshots.

A remote viewer is a different decision. Under Decision 20, a remote viewer sees the lock screen by default until the separate remote pass, whatever capture does. Screen Sharing reads the same window-server image these paths read, so `.none` probably hides the lock from Screen Sharing too. That would conflict with Decision 20. m14 and m22 apply `.none` only when the m22 detection hook positively reports no remote session. Unknown or failed detection means no `.none`. That holds until the remote pass. Measuring that detection is m21. The real Screen Sharing test is m23.

## Henry's Intel Mac

No hardware serial belongs in this repo. His machine is a MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports), macOS Tahoe 26.7.3. Still on that Mac:

- m06: the same marker counts on his OS, including a Space switch and a full-screen app.
- The TCC Screen Recording prompt. This runner already had the permission.
- A real Screen Sharing viewer (m23).
- An external display he plugs in while the windows are up (m06 and m23).
