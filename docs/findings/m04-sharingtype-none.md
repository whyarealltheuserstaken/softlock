# m04: sharingType none on the four capture paths

Measured on 16 November 2026 UTC by the workspace-excluded probe `spikes/macos-desktop-proof`.

The proof run is [macOS desktop proof run 43978092654](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43978092654), job [macos desktop proof](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43978092654/job/133687166956). The measured head is `187d3e699cc204796cc2767431d6faf7f1ac2b6a`. The job printed `result=pass`. This is one 1024x768 virtual display. It does not prove a second monitor, a Space switch, or a full-screen app. Those stay on m06.

## What the runner is

`macos-latest`, `uname -m` `arm64`.

```
ProductName:	macOS
ProductVersion:	26.7.2
BuildVersion:	25G332
uname -m
arm64
```

## The window

Borderless, marker colour sRGB 0.86, 0.04, 0.63, 320 by 240. Level `NSWindow.Level.screenSaver`. Collection behaviour `canJoinAllSpaces` and `fullScreenAuxiliary`. `ignoresMouseEvents` set. `CGShieldingWindowLevel()` was not called.

```
sharing-window=level=screenSaver collection=canJoinAllSpaces,fullScreenAuxiliary ignoresMouseEvents=true shielding=false ordered-front=true error=none
sharing-readback-default=NSWindowSharingReadOnly
sharing-readback-none=NSWindowSharingNone
display-image-api=CGWindowListCreateImage
```

The display-image call is `CGWindowListCreateImage`. The symbol was present. `CGDisplayCreateImage` is the same runtime lookup the m02 harness uses, and on this OS it returned an image. ScreenCaptureKit is the harness entry `SCScreenshotManager` with `SCContentFilter initWithDisplay:excludingWindows:` and an empty window list. No path was unavailable.

## Marker counts

The window is 76800 pixels. Every path counted 76834 marker pixels with sharing left at the default (`NSWindowSharingReadOnly`), and 34 with `NSWindowSharingNone`. The drop is 76800 on every path, the size of the window. The 34 that remain are the same surplus the default image had over that size.

```
sharing-default-CGWindowListCreateImage=marker pixels=76834 width=1024 height=768
sharing-default-CGDisplayCreateImage=marker pixels=76834 width=1024 height=768
sharing-default-screencapture=marker pixels=76834 width=1024 height=768
sharing-default-ScreenCaptureKit=marker pixels=76834 width=1024 height=768 entry=SCScreenshotManager excludingWindows empty
sharing-none-CGWindowListCreateImage=marker pixels=34 width=1024 height=768
sharing-none-CGDisplayCreateImage=marker pixels=34 width=1024 height=768
sharing-none-screencapture=marker pixels=34 width=1024 height=768
sharing-none-ScreenCaptureKit=marker pixels=34 width=1024 height=768 entry=SCScreenshotManager excludingWindows empty
sharing-m14-CGWindowListCreateImage=do-not-claim
sharing-m14-CGDisplayCreateImage=do-not-claim
sharing-m14-screencapture=do-not-claim
sharing-m14-ScreenCaptureKit=do-not-claim
```

Apple's page for `NSWindow.SharingType.none` calls it a legacy constant and says not to use it to hide content from capture. On this OS it is not a pure no-op in the counts: the window's own pixels left every image. It is also not a zero. The claim rule is a zero marker count with the marker present at the default. 34 is not zero, so no path omits the marker, and m14 may claim none of these four paths (decision D6). m06 repeats the same counts on Henry's Intel Mac before any path is claimed.

## Remote viewer

SPEC says an agent's screenshot shows the real desktop because the lock screen is excluded from capture. With sharing left at the default, all four paths contain the marker window, so a remote viewer of those captures sees the lock screen. That is the picture until a path measures zero. `.none` removed one window of marker pixels here and left 34. The job does not turn that into a claim. Until m04 and m06 both show a zero, the macOS lock stays a physical-input lock, and the docs keep the remote viewer on the lock screen.

## Henry's Intel Mac

No hardware serial belongs in this repo. m06 runs this probe on his machine: MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports), macOS Tahoe 26.7.3. The runner does not stand in for that OS, a second display, a Space switch, or a full-screen app.
