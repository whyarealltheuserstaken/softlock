# m02: macos-latest hosts a key window, an active tap, and a marked capture

Measured on 16 November 2026 UTC by the workspace-excluded probe `spikes/macos-desktop-proof`.

The proof run is [macOS desktop proof run 43959209214](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43959209214), job [macos desktop proof](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43959209214/job/133621767075). The measured head is `f663253c174bc65ee00db3ee67dfb96c057bb233`. The job printed `result=pass`. This probe did not lock the session, so it does not say the runner comes back after a lock (decision D8).

An earlier head, `525018cfe1330629f7e9f122f106ca2b760d69c9`, printed `result=pass` with `window-error=NSWindow proof process exited 139`. That exit was a double release in the probe, not a runner capability. A later head blamed `CGEventPost` for missing the window. This run is after the activation change: the proof process waits until the app is active and the window is key, using `activateIgnoringOtherApps` and `NSRunningApplication activateWithOptions`. A crash in the probe still fails the job.

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

The borderless window ordered front, became key, and received keyCode 0 from `CGEventPost` to `kCGSessionEventTap`. The stage lines include `activate` and `activate-done`.

```
stage=order-front
stage=activate
stage=activate-done
stage=order-front-done
stage=CGEventPost
stage=CGEventPost-done
stage=key-received
stage=key-received-done
stage=order-out
stage=order-out-done
window-ordered-front=true
window-key=true
window-key-received=true
window-post-location=kCGSessionEventTap
window-stage=key-received-done
window-error=none
```

The LaunchAgent is in the Aqua session. Its window was also key. In that process `CGPreflightPostEventAccess=false`, and the local monitor did not see keyCode 0:

```
job-agent-window-ordered-front=true
job-agent-window-error=the window was key and CGEventPost did not deliver keyCode 0 to the local monitor; CGPreflightPostEventAccess=false
```

## Taps

The three listen-only taps were created and their callbacks fired for keyCode 0. No TCC prompt appeared. `accessibility=true` means Accessibility was already granted for this runner. That says nothing about the prompts on Henry's Mac (m10).

```
tap-kCGSessionEventTap-user=created callback=true tcc=created accessibility=true listen=true prompt=none error=none
tap-kCGHIDEventTap-user=created callback=true tcc=created accessibility=true listen=true prompt=none error=none
tap-kCGHIDEventTap-root=created callback=true tcc=created accessibility=true listen=true prompt=none error=none
```

Before the active tap, the same process installed only a listen-only session tap and posted keyCode 0. That control saw the key (`control=true`). The `kCGEventTapOptionDefault` tap then deleted the next posted key, and the listen-only tap did not see it.

```
stage=active-control
stage=active-control-done
stage=active-listen
stage=active-default
stage=active-done
tap-active-kCGSessionEventTap-user=created control=true deleted=true listen-saw=false accessibility=true listen=true prompt=none error=none
```

## Capture pixels

Each API returned a 1024x768 image. Every image had non-black pixels and the probe window's marker colour. This is the positive capture only: the window was on screen, and capture exclusion was not tested. ScreenCaptureKit's non-black count is 158684. The other three are 158731. The marker count is 76836 for `CGWindowListCreateImage` and 76834 for the other three. This is one virtual display.

```
capture-CGWindowListCreateImage=opened width=1024 height=768 nonblack=true pixels=158731 marker=true pixels=76836
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

- **m03** stays for the taps this log created. The listen-only session tap, the user HID tap, the root HID tap, and one active `kCGEventTapOptionDefault` session tap all came up. A listen-only tap saw keyCode 0 when no Default tap was installed. The active tap then deleted keyCode 0 and the listen-only tap did not see it. `accessibility=true` is pre-granted on this runner and says nothing about the TCC prompts on Henry's Mac (m10).
- **m04** stays for this one 1024x768 display only as a positive capture. All four paths showed non-black pixels and the marker colour. Capture exclusion was not tested. A second monitor is not present.
- **m07** moves entirely to Henry's Intel Mac. The runner has no lid, no App Nap from a closed lid, and no real idle the way a laptop does.
- **m08** keeps its chooser rows on the existing verify job. Those rows are a pure function and do not lock. The sheet (lock timing, the screensaver, FileVault, and the password after the screensaver) is Henry's Mac. This spike did not lock the runner.
- **m11** stays on Henry's Intel Mac. This run is arm64. A Rosetta run is not the x86_64 slice test.
- **m15** can keep load/unload and the socket protocol on the runner. The daemon has no graphic session, so launchd has to start the agent at login for a window. The proof process's window became key and received the posted key. The LaunchAgent's window was key and did not receive it (`CGPreflightPostEventAccess=false`), so a posted key reaching the product agent's window stays on Henry's Mac, along with installing the product agent and locking from a second process.

## Henry's Intel Mac

This section is only for Henry's machine: a 2020 Intel MacBook Pro (13-inch, Four Thunderbolt 3 ports), laptop, Touch ID, T2, macOS Tahoe 26. No hardware serial belongs in this repo.

On that Mac:

- m10: the TCC prompts. `accessibility=true` here was already granted and does not describe those prompts.
- A posted key reaching a LaunchAgent window. The runner's proof process became key and received keyCode 0. The LaunchAgent window was key and still did not receive it, because `CGPreflightPostEventAccess` was false in that process.
- m07: lid, power assertions, and real idle.
- m08: the lock sheet, the screensaver, and FileVault.
- m11: the x86_64 slice.
- m15: a product LaunchAgent in the Aqua session, and locking from a second process.
- m04: capture exclusion, which this runner did not test, and a second display if one is attached. The runner only showed the positive capture on 1024x768.
