# m02: macos-latest desktop proof

Measured on 16 November 2026 UTC by the workspace-excluded probe `spikes/macos-desktop-proof`.

The first proof run is [macOS desktop proof run 43943606013](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43943606013), job [macos desktop proof](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43943606013/job/133567274223), head `525018cfe1330629f7e9f122f106ca2b760d69c9`. That commit printed `result=pass`. The window line in that pass was a bug in the probe, not a runner capability miss: `macos_prove_window` built the window under ARC, left `releasedWhenClosed` at its default of YES, and then called `[window close]`. That is a double release and matches the SIGSEGV (exit 139). A crash in the probe now fails the job. This probe did not lock the session, so it does not say the runner comes back after a lock (decision D8).

The m03 and m04 "stays on the runner" lines from that run are withdrawn. Listen-only taps do not prove the active tap m03 and m14 need, and `opened 1024x768` does not say whether the image has non-black pixels or the probe window's marker colour. Those decisions wait on the re-run.

## What the runner is

`macos-latest`, `uname -m` `arm64`, uid 501, user `runner`.

```
ProductName:	macOS
ProductVersion:	26.7.2
BuildVersion:	25G332
uname -m
arm64
```

The screen the capture APIs opened is one 1024x768 display.

## What works

Listen-only taps were created and their callbacks fired. No TCC prompt appeared. `accessibility=true` on every tap means Accessibility was already granted for this runner. That says nothing about the TCC prompts on Henry's Mac (m10).

```
tap-kCGSessionEventTap-user=created callback=true tcc=created accessibility=true listen=true prompt=none error=none
tap-kCGHIDEventTap-user=created callback=true tcc=created accessibility=true listen=true prompt=none error=none
tap-kCGHIDEventTap-root=created callback=true tcc=created accessibility=true listen=true prompt=none error=none
```

All four capture calls returned a 1024x768 image. That only proves an image came back. The run did not record non-black pixels or the probe window's marker colour:

```
capture-CGWindowListCreateImage=opened width=1024 height=768
capture-CGDisplayCreateImage=opened width=1024 height=768
capture-screencapture=opened width=1024 height=768
capture-ScreenCaptureKit=opened width=1024 height=768
```

`CGDisplayCreateImage` is marked unavailable in the macOS 26 SDK. The probe looks the symbol up at runtime. On this runner the symbol is still present and returned an image.

A throwaway LaunchAgent and a throwaway LaunchDaemon were loaded and then unloaded. Labels: `com.runner.macosproof.agent` and `com.runner.macosproof.daemon`. The workflow's last step bootouts both labels with `if: always()`, including after the earlier compile and crash failures on this pull request.

```
job-agent-loaded=true
job-agent-running=true
job-agent-unloaded=true
job-agent-keep-alive=true
job-agent-session-limit=Aqua
job-agent-uid=501
job-agent-graphic-session=true
job-daemon-loaded=true
job-daemon-running=true
job-daemon-unloaded=true
job-daemon-keep-alive=true
job-daemon-session-limit=none
job-daemon-uid=0
job-daemon-graphic-session=false
job-daemon-window-error=no graphic session; NSWindow was not created
```

The agent is in the Aqua session. The daemon is root and has no graphic session, so it did not create a window. A daemon that needs a window in the Aqua session has to start the agent through launchd.

## What that run did not prove

The borderless window crashed inside the probe. The child exited 139 (`SIGSEGV`). The only stage line was `stage=NSWindow`, printed before the window call, so the log does not name `order-front`, `CGEventPost`, or `key-received`.

```
window-ordered-front=false
window-key-received=false
window-post-location=none
window-error=NSWindow proof process exited 139: stage=NSWindow
```

The same crash happened in-process on `7bb37642b23269dd992cb57d5723f5d73467b371` (`Segmentation fault: 11`). The agent job wrote `job-agent-window-error=NSWindow proof started` and then died in that call. Treating that as `result=pass` was wrong. The re-run has to record the window and the key after the double release is fixed.

There was no active `kCGEventTapOptionDefault` tap. All three taps were listen-only. m03 and m14 need an active tap that deletes a posted key.

## Which later tickets stay on the runner

These decisions use only the log above. m03 and m04 are not decided yet.

- **m03** does not stay on this evidence. The listen-only session and HID taps were created and their callbacks fired, with no TCC prompt. An active tap that swallows a posted key was not attempted. Whether m03 can stay depends on that re-run.
- **m04** does not stay on this evidence. Each API returned 1024x768. The log does not say whether those images contain non-black pixels or the probe window's marker colour. One virtual display would still not prove a second monitor.
- **m07** moves entirely to Henry's Intel Mac. The runner has no lid, no App Nap from a closed lid, and no real idle the way a laptop does.
- **m08** keeps its chooser rows on the existing verify job. Those rows are a pure function and do not lock. The sheet (lock timing, the screensaver, FileVault, and the password after the screensaver) is Henry's Mac. This spike did not lock the runner.
- **m11** stays on Henry's Intel Mac. This run is arm64. A Rosetta run is not the x86_64 slice test.
- **m15** can keep load/unload and the socket protocol on the runner. The daemon has no graphic session, so launchd has to start the agent at login for a window. Installing the product agent and locking from a second process stay on Henry's Mac. Window work stays off the runner until a re-run shows the window and the key.

## Henry's Intel Mac

This section is only for Henry's machine: a 2020 Intel MacBook Pro (13-inch, Four Thunderbolt 3 ports), laptop, Touch ID, T2, macOS Tahoe 26. No hardware serial belongs in this repo.

On that Mac:

- The runner's `accessibility=true` does not describe the TCC prompts on this Mac (m10).
- Window and `CGEventPost` on this Mac stay here until a runner re-run records them. The exit 139 line was a double release in the probe.
- m07: lid, power assertions, and real idle.
- m08: the lock sheet, the screensaver, and FileVault.
- m11: the x86_64 slice.
- m15: a product LaunchAgent in the Aqua session, and locking from a second process.
- m04: a second display, if one is attached. The runner only showed 1024x768.
