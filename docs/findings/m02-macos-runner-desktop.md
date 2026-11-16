# m02: macos-latest hosts an active tap and a marked capture; the posted key does not arrive

Measured on 16 November 2026 UTC by the workspace-excluded probe `spikes/macos-desktop-proof`.

The proof run is [macOS desktop proof run 43950358919](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43950358919), job [macos desktop proof](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43950358919/job/133590918186). The measured head is `fc4d964166a8957578ab7e19b043b6f4091bb285`. The job printed `result=pass`. This probe did not lock the session, so it does not say the runner comes back after a lock (decision D8).

An earlier head, `525018cfe1330629f7e9f122f106ca2b760d69c9`, printed `result=pass` with `window-error=NSWindow proof process exited 139`. That exit was a double release in the probe (`releasedWhenClosed` left at YES, then `[window close]`), not a runner capability. This run is the measurement after that fix. A crash in the probe now fails the job.

## What the runner is

`macos-latest`, `uname -m` `arm64`, uid 501, user `runner`.

```
ProductName:	macOS
ProductVersion:	26.7.2
BuildVersion:	25G332
uname -m
arm64
```

The captures are one 1024x768 display.

## Window and key

The borderless window ordered front. It was not the key window. `CGEventPost` did not deliver keyCode 0. Posting was allowed in the proof process (`CGPreflightPostEventAccess=true`). The stage lines completed through `key-received-done` and `order-out-done`. There was no crash.

```
stage=order-front
stage=order-front-done
stage=CGEventPost
stage=CGEventPost-done
stage=key-received
stage=key-received-done
stage=order-out
stage=order-out-done
window-ordered-front=true
window-key=false
window-key-received=false
window-post-location=none
window-stage=key-received-done
window-error=CGEventPost did not deliver keyCode 0 (tried kCGSessionEventTap, kCGAnnotatedSessionEventTap, kCGHIDEventTap); CGPreflightPostEventAccess=true
```

The LaunchAgent is in the Aqua session and also ordered a window front. In that process `CGPreflightPostEventAccess=false`, and the key was not delivered there either:

```
job-agent-window-ordered-front=true
job-agent-window-error=CGEventPost did not deliver keyCode 0 (tried kCGSessionEventTap, kCGAnnotatedSessionEventTap, kCGHIDEventTap); CGPreflightPostEventAccess=false
```

## Taps

The three listen-only taps were created and their callbacks fired. No TCC prompt appeared. `accessibility=true` means Accessibility was already granted for this runner. That says nothing about the prompts on Henry's Mac (m10).

```
tap-kCGSessionEventTap-user=created callback=true tcc=created accessibility=true listen=true prompt=none error=none
tap-kCGHIDEventTap-user=created callback=true tcc=created accessibility=true listen=true prompt=none error=none
tap-kCGHIDEventTap-root=created callback=true tcc=created accessibility=true listen=true prompt=none error=none
```

One active `kCGEventTapOptionDefault` session tap deleted the posted key. A listen-only tap created ahead of it did not see that key.

```
stage=active-listen
stage=active-default
stage=active-done
tap-active-kCGSessionEventTap-user=created deleted=true listen-saw=false accessibility=true listen=true prompt=none error=none
```

## Capture pixels

Each API returned a 1024x768 image. Every image had non-black pixels and the probe window's marker colour. ScreenCaptureKit's non-black count is 158684. The other three are 158731. The marker count is 76834 on all four. This is one virtual display.

```
capture-CGWindowListCreateImage=opened width=1024 height=768 nonblack=true pixels=158731 marker=true pixels=76834
capture-CGDisplayCreateImage=opened width=1024 height=768 nonblack=true pixels=158731 marker=true pixels=76834
capture-screencapture=opened width=1024 height=768 nonblack=true pixels=158731 marker=true pixels=76834
capture-ScreenCaptureKit=opened width=1024 height=768 nonblack=true pixels=158684 marker=true pixels=76834
```

`CGDisplayCreateImage` is marked unavailable in the macOS 26 SDK. The probe looks the symbol up at runtime. On this runner the symbol is present and returned an image.

## LaunchAgent and LaunchDaemon

Both throwaway jobs loaded and unloaded. Labels: `com.runner.macosproof.agent` and `com.runner.macosproof.daemon`. The workflow's last step bootouts both with `if: always()`.

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

The daemon is root and has no graphic session, so it did not create a window. A daemon that needs a window in the Aqua session has to start the agent through launchd.

## Which later tickets stay on the runner

- **m03** stays for the taps this log created. The listen-only session tap, the user HID tap, the root HID tap, and one active `kCGEventTapOptionDefault` session tap all came up. The active tap deleted keyCode 0 and the listen-only tap did not see it. `accessibility=true` is pre-granted on this runner and says nothing about the TCC prompts on Henry's Mac (m10).
- **m04** stays for this one 1024x768 display. All four capture paths showed non-black pixels and the probe window's marker colour. A second monitor is not present.
- **m07** moves entirely to Henry's Intel Mac. The runner has no lid, no App Nap from a closed lid, and no real idle the way a laptop does.
- **m08** keeps its chooser rows on the existing verify job. Those rows are a pure function and do not lock. The sheet (lock timing, the screensaver, FileVault, and the password after the screensaver) is Henry's Mac. This spike did not lock the runner.
- **m11** stays on Henry's Intel Mac. This run is arm64. A Rosetta run is not the x86_64 slice test.
- **m15** can keep load/unload and the socket protocol on the runner. The daemon has no graphic session, so launchd has to start the agent at login for a window. The window ordered front and did not receive the posted key, so window-and-key work stays on Henry's Mac, along with installing the product agent and locking from a second process.

## Henry's Intel Mac

This section is only for Henry's machine: a 2020 Intel MacBook Pro (13-inch, Four Thunderbolt 3 ports), laptop, Touch ID, T2, macOS Tahoe 26. No hardware serial belongs in this repo.

On that Mac:

- m10: the TCC prompts. `accessibility=true` here was already granted and does not describe those prompts.
- A posted key reaching a window. This runner ordered a window front and `CGEventPost` did not deliver keyCode 0. The window was not key.
- m07: lid, power assertions, and real idle.
- m08: the lock sheet, the screensaver, and FileVault.
- m11: the x86_64 slice.
- m15: a product LaunchAgent in the Aqua session, and locking from a second process.
- m04: a second display, if one is attached. The runner only showed 1024x768.
