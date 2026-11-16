# m02: macos-latest can host taps, capture, and launchd; the window probe crashes

Measured on 16 November 2026 UTC by the workspace-excluded probe `spikes/macos-desktop-proof`.

The proof run is [macOS desktop proof run 43943606013](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43943606013), job [macos desktop proof](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43943606013/job/133567274223). The measured head is `525018cfe1330629f7e9f122f106ca2b760d69c9`. That commit printed `result=pass`. The window result inside that pass is a crash, recorded below. This probe did not lock the session, so it does not say the runner comes back after a lock (decision D8).

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

Listen-only taps were created and their callbacks fired. No TCC prompt appeared.

```
tap-kCGSessionEventTap-user=created callback=true tcc=created accessibility=true listen=true prompt=none error=none
tap-kCGHIDEventTap-user=created callback=true tcc=created accessibility=true listen=true prompt=none error=none
tap-kCGHIDEventTap-root=created callback=true tcc=created accessibility=true listen=true prompt=none error=none
```

All four capture calls opened that 1024x768 display:

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

## What does not work

The borderless window crashed. The child exited 139 (`SIGSEGV`) at the `NSWindow` stage, before it could report `CGEventPost`.

```
window-ordered-front=false
window-key-received=false
window-post-location=none
window-error=NSWindow proof process exited 139: stage=NSWindow
```

The same crash happened in-process on the previous head (`7bb37642b23269dd992cb57d5723f5d73467b371`), which died with `Segmentation fault: 11` during the window step. The agent job wrote its marker first (`job-agent-window-error=NSWindow proof started`) and then died in the same call. The key was not delivered. This is a negative window result with a named API error, not a shown window.

## Which later tickets stay on the runner

- **m03** stays. `kCGSessionEventTap` and `kCGHIDEventTap` were live for the user, and the root HID tap was live too. Callbacks fired. There was no TCC denial and no prompt the job could not click.
- **m04** stays for the single 1024x768 display. `CGWindowListCreateImage`, `CGDisplayCreateImage`, `screencapture`, and ScreenCaptureKit all opened. Pixel checks on that one virtual display can run here. A second monitor is not present on this runner.
- **m07** moves entirely to Henry's Intel Mac. The runner has no lid, no App Nap from a closed lid, and no real idle the way a laptop does.
- **m08** keeps its chooser rows on the existing verify job. Those rows are a pure function and do not lock. The sheet (lock timing, the screensaver, FileVault, and the password after the screensaver) is Henry's Mac. This spike did not lock the runner.
- **m11** stays on Henry's Intel Mac. This run is arm64. A Rosetta run is not the x86_64 slice test.
- **m15** can keep load/unload and the socket protocol on the runner. The daemon has no graphic session, so launchd has to start the agent at login for a window. Installing the product agent and locking from a second process stay on Henry's Mac, along with any window work, because `NSWindow` crashes on this runner.

## Henry's Intel Mac

This section is only for Henry's machine: a 2020 Intel MacBook Pro (13-inch, Four Thunderbolt 3 ports), laptop, Touch ID, T2, macOS Tahoe 26. No hardware serial belongs in this repo.

On that Mac:

- Retry the borderless window and `CGEventPost`. The runner dies inside `NSWindow` before a key is posted.
- m07: lid, power assertions, and real idle.
- m08: the lock sheet, the screensaver, and FileVault.
- m11: the x86_64 slice.
- m15: a product LaunchAgent in the Aqua session, and locking from a second process.
- m04: a second display, if one is attached. The runner only showed 1024x768.
