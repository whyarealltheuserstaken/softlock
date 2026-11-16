# macOS addendum (phase 2)

| | |
|---|---|
| **Status** | Planning only. Not approved as a set, and the tickets are not GitHub issues. Henry decided D1 and D5. The lead accepted D2, D3, D4, D6, and D7, and decided D8 (a failure never ends Unlocked). The Intel Mac details in D1 are still open. No product code comes with it. |
| **Date** | 16 November 2026 |
| **Read against** | `23b5a39bc92c2ee32f470256d78be27d030dc59f` on `main` |
| **Supersedes** | The macOS bullets that used to live in SPEC.md Implementation Decision 16, and the short phase-2 note that used to be this file |
| **Does not change** | Windows v1, the lock state machine, the Broker nonce rule, the open-core boundary, or the Linux note |

This is the macOS half of the product, planned in parallel with Windows. Henry decided D1 in [`docs/tickets/macos/decisions.md`](../tickets/macos/decisions.md): Universal 2, macOS 11 or later, his Intel Mac for the Intel slice, and `macos-latest` for arm64. The machine details in that decision are still open.

Every claim below is tagged.

- **Verified.** The sentence is what an Apple page said when it was read on 16 November 2026. The link is that page. A manual page that ships with macOS is cited the same way when the text was read from a public mirror of that page.
- **Spike.** The sentence is a design choice or a guess. The named ticket has to measure it. A spike result replaces the guess. Until then the product must not promise it.

Windows lessons this addendum copies, from the findings already in the tree at the commit above:

- [`docs/findings/05-input-hook/README.md`](../findings/05-input-hook/README.md), measured at `e399397de95fe6699f7f094e82bb65fb289cfaf9`. A slow hook was not removed. On windows-2022 the timeout never fired. On windows-2025 it fired and the hook stayed. Liveness is a self-check. It is not "the operating system took the hook away."
- [`docs/findings/02-windows-runner-desktop.md`](../findings/02-windows-runner-desktop.md). Seam 3 stays on `windows-2022` and `windows-2025`. The macOS desktop probe is a different job.

## What stays from the Windows spec

The Core traits in SPEC.md Implementation Decision 4 are the macOS backend's interface. `InputGuard`, `Overlay`, `SessionControl`, `Power`, `Credentials`, and `EncryptionStatus` are already there. The state machine, the 250 ms heartbeat (`HEARTBEAT_INTERVAL` in `crates/softlock-core/src/heartbeat.rs`), the three-miss fail-closed rule, and the 32-byte `LaunchNonce` (`crates/softlock-core/src/state.rs`) do not grow a macOS special case.

The service's display name is the existing constant `SERVICE_DISPLAY_NAME`, which is "SoftLock Agent" (`crates/softlock-core/src/name.rs`). The launchd label is not that string. launchd's required `Label` is an identifier without spaces. The proposed labels, to be written next to `APP_BUNDLE_ID` when ticket m15 lands, are `com.softlock.agent` for the daemon and `com.softlock.agent.lockhost` for the agent. They are machine ids. The words the owner sees remain the one display-name constant.

The Mac App Store is still out. A sandboxed app cannot post or swallow session events. **Spike m15** confirms the unsandboxed Developer ID (or unsigned) build is the one that can create a tap. Distribution is a disk image or a pkg, not the store, and only after decision D2.

Out of scope, same as Windows: allowlists, rules, fleet management, remote triggers, the command-line tool, and MDM. A PPPC profile that pre-grants Accessibility is MDM. Decision D3.

## 1. Physical input

The guard is a `CGEventTap` that swallows events it classifies as physical and passes the rest. Where the tap sits, and how it classifies, are not settled by Apple's pages.

### Where the tap sits

**Verified.** [`CGEvent.tapCreate`](https://developer.apple.com/documentation/coregraphics/cgevent/tapcreate(tap:place:options:eventsofinterest:callback:userinfo:)) says only a root process may locate a tap where HID events enter the window server. For anyone else the call returns `NULL`. The same page says the tap receives key-up and key-down events only if the process is root, or if access for assistive devices is enabled. The location constants in Apple's SDK header [`CGEventTypes.h`](https://github.com/phracker/MacOSX-SDKs/blob/master/MacOSX10.15.sdk/System/Library/Frameworks/CoreGraphics.framework/Versions/A/Headers/CGEventTypes.h) (macOS 10.15 SDK, Apple copyright) are `kCGHIDEventTap`, `kCGSessionEventTap`, and `kCGAnnotatedSessionEventTap`.

**Spike m02, m03, m05.** Which location a SoftLock process can actually create, and which one sees a physical key before the target app. The working proposal, to be thrown out if the spike disagrees:

- The lock host runs as the console user, not root, and creates `kCGSessionEventTap` at the head (`kCGHeadInsertEventTap`) as an active filter (`kCGEventTapOptionDefault`).
- A root `kCGHIDEventTap` is the fallback only if the session tap misses physical keys, trackpad events, or events headed to another process. It lives in the LaunchDaemon. The daemon does not draw.

Returning `NULL` from the callback deletes the event. That is the swallow. **Verified** from the callback comment in the same SDK header: the callback returns the event, a new event, or `NULL` if the event is to be deleted.

### Telling physical from injected

There is no documented "this event was injected" bit on macOS. Windows has `LLKHF_INJECTED`. The fields below are what Apple documents. None of them is defined as "a finger pressed this key."

**Verified.** Apple's SDK header [`CGEventSource.h`](https://github.com/phracker/MacOSX-SDKs/blob/master/MacOSX10.9.sdk/System/Library/Frameworks/CoreGraphics.framework/Versions/A/Headers/CGEventSource.h) (macOS 10.9 SDK, Apple copyright) defines three state tables:

- `kCGEventSourceStateHIDSystemState` is the combined state of hardware event sources posting from the HID system. The header tells a daemon or a user-space device driver that is interpreting hardware to create its source with this id.
- `kCGEventSourceStateCombinedSessionState` is the combined state of event sources posting into the login session. The header tells an ordinary in-session poster to use this one.
- `kCGEventSourceStatePrivate` is an independent table for a program that wants its own state, and the header names remote-control programs as the example.

`CGEventSourceCreate` is public, so a program can create a source with the HID system state. The field `kCGEventSourceStateID` is "the event source state ID used to create this event." `kCGEventSourceUnixProcessID` is "the event source Unix process ID." Both are `CGEventField` values in `CGEventTypes.h`. The enum page is [`CGEventSourceStateID`](https://developer.apple.com/documentation/coregraphics/cgeventsourcestateid).

**Spike m03 (posted events on the runner) and m05 (a real keyboard on Henry's Intel Mac).** The spike records, for each source, the state id, the source pid, and whether the target app received the event. The sources are at least: a physical key, a physical click, a trackpad scroll, `CGEventPost` with each of the three state ids, and a second process posting. The classification rule is whatever that table supports. It is not "HID state means physical." A user-space driver that posts with the HID state, which is what the header tells it to do, is expected to look like hardware. That is the same class of gap as a virtual HID device on Windows, and it is a documented limitation if the spike confirms it, not a surprise.

Until m05, the guard's draft rule, which the spike may replace, is:

- State id is HID system state and source pid is 0: treat as physical and swallow.
- Any other state id, or a non-zero source pid: treat as software and pass.
- A test-only build may also treat a posted event as physical when `kCGEventSourceUserData` equals a per-run random nonce, so CI can exercise the swallow path. Release builds do not contain that path. Ticket m14 adds the release-scan coverage in the same change, as SPEC.md Testing Decisions already require for the Windows feature. The marker strings already scanned are `softlock-simulated-physical-marker` and `softlock_simulated_physical_marker` in `crates/softlock-lockhost/tests/release_binary.rs` at the commit above. Once PR #48 merges, the same scan also names `softlock-dev-launch-marker` and `softlock_dev_launch_marker`.

### Permissions

**Verified.** Apple's [resetting access to protected resources](https://developer.apple.com/documentation/xcode/resetting-access-to-protected-resources-in-macos) lists three different services: `Accessibility`, `ListenEvent` ("Input Monitoring"), and `PostEvent` ("sending keystrokes"). `tccutil reset <service>` clears a remembered choice so the prompt can appear again.

**Spike m05 and m10.** Which of those three a swallowing session tap actually prompts for. The `tapCreate` page still says "assistive devices," which is the old name for the Accessibility grant. It does not mention Input Monitoring. A listen-only tap and a modifying tap may prompt for different services. SoftLock's tap changes the stream (it deletes events). An event tap that changes events usually needs only Accessibility. m10 confirms whether Input Monitoring is also needed. PostEvent is not required because SoftLock does not need to inject user-visible input except its own liveness marker. m10 writes the prompt the owner sees, with no MDM profile (decision D3, accepted by the lead).

Grants stick to the code signature. An ad-hoc signature changes when the binary changes, and the prompt comes back. That is decision D2, not a guess about Apple's policy.

### The tap being disabled

**Verified.** `CGEventTypes.h` says the out-of-band types "are delivered to the event tap callback to notify it of unusual conditions that disable the event tap." The values are `kCGEventTapDisabledByTimeout` (`0xFFFFFFFE`) and `kCGEventTapDisabledByUserInput` (`0xFFFFFFFF`). Apple's [`CGEventType`](https://developer.apple.com/documentation/coregraphics/cgeventtype) page names them "disabled because of timeout" and "disabled because of user input." The timeout duration is not on that page. The page does not say that "user input" means a secure password field.

**Spike m03 and m05.** Sleep inside the callback and record whether the disable event arrives, how long it takes, and whether a key pressed during the sleep reaches another process. Separately, focus a secure password field (and call `IsSecureEventInputEnabled` if the symbol is present) and record whether the tap goes quiet, receives `kCGEventTapDisabledByUserInput`, or keeps seeing keys. The existing one-line risk, "Secure Input can starve event taps," stays a spike until that row is filled in.

### Liveness, copied from the Windows finding

The disable callback is more than Windows documented, and the Windows measurement still applies: a timeout report is not the same as "the hook is gone," and a stuck run loop never delivers the callback at all.

The guard runs on its own thread. The callback does no I/O and no allocation. Every heartbeat period (250 ms, the existing constant), if the callback has seen no event, the guard posts a no-op marker and requires the callback to have seen it before the next heartbeat is due. Three misses is `hook_alive = false`. Either disable event sets `hook_alive = false` immediately. The state machine already fail-closes on `GuardUnhealthy`.

The guard does not re-enable the tap and keep going. Other programs do that. A disabled tap is a gap in which physical keys may have reached an app. **Spike m03** checks that guess. If the spike shows the system drops events while the tap is disabled, the fail-closed reaction can be revisited. Until then, fail closed.

## 2. The overlay

One borderless window per display, drawn by the lock host, not by the Tauri web view. The person at the Mac sees it. The design goal, which this section does not promise, is that an agent's capture does not.

### Capture exclusion

**Verified.** [`NSWindow.SharingType.none`](https://developer.apple.com/documentation/appkit/nswindow/sharingtype-swift.enum/none) currently says: "A legacy constant that macOS no longer uses." The discussion says `NSWindowSharingNone` can cause content not to be available in some sharing situations, and "Don't use this value to hide or omit content from being captured. Instead, use FairPlay Streaming." The page does not describe a window-server flag that omits an `NSWindow` from a screenshot.

**Verified.** ScreenCaptureKit's [`SCContentFilter` initializer `init(display:excludingWindows:)`](https://developer.apple.com/documentation/screencapturekit/sccontentfilter/init(display:excludingwindows:)) excludes windows the capturer names. The agent doing the capture chooses the filter. SoftLock cannot make an agent pass that filter. [Capturing screen content in macOS](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos) is the capturer's API, and it requires Screen Recording permission for that app.

**Spike m04 (runner) and m06 (Henry's Intel Mac).** Set `sharingType` to `.none` and to the default. Count a marker colour in each of: the current display-image API (`CGWindowListCreateImage` or whatever replaced it on that OS), the `screencapture` tool, and a ScreenCaptureKit `SCStream` of the whole display with no windows excluded. Record the OS from `sw_vers`. The Windows finding's shape is the model: marker present with exclusion off, marker absent with exclusion on, or a written miss. Paths that still show the marker are not claimed. The sharing-type page names FairPlay Streaming and does not describe it as a window flag. The spike does not adopt it.

A developer forum thread ([792152](https://developer.apple.com/forums/thread/792152)) reported that on macOS 15.4 a window with sharing none was still captured by ScreenCaptureKit, and a reply said there was no public API to prevent screen capture. This addendum fetched that thread and got HTTP 403, so the reply is not treated as a verified quote. m04 is the measurement.

### Level, Spaces, full-screen, every display

**Verified.** [`CGShieldingWindowLevel()`](https://developer.apple.com/documentation/coregraphics/cgshieldingwindowlevel()) returns the level of the shield window for a captured display. The page says positioning a window over a captured display may fail or draw illegible content, and "this technique is not recommended." The addendum does not use it.

**Verified.** [`NSWindow.Level.screenSaver`](https://developer.apple.com/documentation/appkit/nswindow/level-swift.struct/screensaver) is "the level for a screen saver." [`canJoinAllSpaces`](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/canjoinallspaces) means the window can appear in all spaces. [`fullScreenAuxiliary`](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/fullscreenauxiliary) means the window displays on the same space as the full-screen window. [`ignoresMouseEvents`](https://developer.apple.com/documentation/appkit/nswindow/ignoresmouseevents) makes the window transparent to mouse events, which is how an agent's click falls through to the window under it.

**Spike m04 and m06.** Whether `screenSaver` plus `canJoinAllSpaces` plus `fullScreenAuxiliary`, one window per `NSScreen`, stays above a full-screen app, stays up across a Space switch, and covers every display Henry has attached, including one plugged in while locked. A single built-in display does not prove hot-plug. The runner's virtual display does not prove a second monitor. `ignoresMouseEvents` is necessary and not sufficient: m06 also checks that the overlay does not become key and does not move keyboard focus. Presentation options are a separate row below, not a substitute for the tap.

**Verified.** These presentation options exist, and the pages say what they disable:

- [`disableForceQuit`](https://developer.apple.com/documentation/appkit/nsapplication/presentationoptions-swift.struct/disableforcequit): the Force Quit panel (Command-Option-Escape).
- [`disableProcessSwitching`](https://developer.apple.com/documentation/appkit/nsapplication/presentationoptions-swift.struct/disableprocessswitching): Command-Tab.
- [`disableSessionTermination`](https://developer.apple.com/documentation/appkit/nsapplication/presentationoptions-swift.struct/disablesessiontermination): the Restart, Shut Down, and Log Out panel from the power key.
- [`hideDock`](https://developer.apple.com/documentation/appkit/nsapplication/presentationoptions-swift.struct/hidedock) and [`hideMenuBar`](https://developer.apple.com/documentation/appkit/nsapplication/presentationoptions-swift.struct/hidemenubar).

**Spike m06.** Whether any of those options take effect while the lock host is not the active app. The Windows overlay never takes focus, so agents keep typing into the window that was focused. If a presentation option only works for the frontmost app, the tap has to swallow the chord instead, and the option is not the control. The power key itself sleeping the Mac is not what `disableSessionTermination` claims to stop. m07 records what the power key does.

## 3. Keeping agents running

**Verified.** [QA1340](https://developer.apple.com/library/archive/qa/qa1340/_index.html) says `IOPMAssertionCreateWithName` can prevent or delay idle sleep, and cannot prevent forced sleep. Forced sleep includes the user closing the lid, a thermal emergency, a low battery, and the user choosing sleep. The sample on that page uses the older assertion names `kIOPMAssertionTypeNoIdleSleep` and `kIOPMAssertionTypeNoDisplaySleep`. The [energy guide](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/PrioritizeWorkAtTheAppLevel.html) shows `pmset -g assertions` reporting `PreventUserIdleSystemSleep`, with the reason string the app passed to `beginActivityWithOptions:reason:`. `NSActivityUserInitiated` keeps that app out of App Nap for the length of the activity.

**Verified.** The [App Nap guide](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/AppNap.html) says the system may nap an app that is not in front, has not recently updated a visible window, is not playing audio, has not taken a power assertion, and is not using OpenGL. Nap reduces that app's priority and slows its timers. The assertion and the activity are properties of the calling process. The pages do not say SoftLock can turn App Nap off in someone else's process.

**Spike m07.** Hold `PreventUserIdleSystemSleep` and `PreventUserIdleDisplaySleep` if the SDK still has those names (the spike prints the names that compile). Run a small process that increments a counter on a timer, with the overlay up and with it down, and compare the counts. If the covered process slows, that is a product limit and the wizard says so. `caffeinate` is only a probe. The product calls `IOPMAssertionCreateWithName` so the reason string shows up in `pmset -g assertions`. The spike checks that `caffeinate` exists before using it as a control.

Lid close is forced sleep on the QA1340 page. Phase 2 does not offer lid-close takeover on macOS. That is decision D7, accepted by the lead. If his Intel Mac is a desktop, m07 records "no lid" and the lid case stays unproven on every machine this plan has.

## 4. Fail closed, and FileVault

Fail closed still means the real lock, within about a second, and agents pause until the owner signs in. The Windows primitive does not exist here.

**Decision D5, Henry, 16 November 2026.** The lock call is `SACLockScreenImmediate` in the private `login.framework`. The binary resolves it with `dlopen` and `dlsym`. It does not link `login.framework`. There is no Apple documentation page for this symbol. It is private SPI. A missing symbol does not crash. A call that returns failure does not crash. Either one takes the documented lock path: start the screen saver and require the account password to leave it. That path is what the public pages give us. It is the weaker path. It depends on the password-after-screensaver setting, and a bystander must not be able to click or type through it back to the desktop. The login window is not this fallback.

**Decision D8, the lead, 16 November 2026.** A failure never ends Unlocked, the same rule as Windows.

- At arm time, if the private call is unavailable and the documented fallback cannot guarantee a password, refuse to arm. "Cannot guarantee" means password-after-screensaver is off, or the delay is not zero. The message tells the owner how to turn that setting on and set the delay to zero.
- At run time, if both lock paths fail, keep the guard and the overlay up, show an error, keep retrying, and release only on a verified unlock.

**Spike m08.** On Henry's Intel Mac, time both paths. The private call has to show the real lock screen within about a second. The documented path is timed the same way, including whether a click or a key reached the desktop without the account password. The sheet records the password-after-screensaver setting and the delay. Forcing a missing symbol on his OS may be impossible. When it is, the sheet says so, and the CI rows below are the proof of that branch.

**Verified.** Apple's [FileVault deployment introduction](https://support.apple.com/guide/deployment/intro-to-filevault-dep82064ec40/web) says that after FileVault is on, the user's credentials are required during boot. On Apple silicon and on a Mac with the T2 chip, key handling stays in the Secure Enclave. A forced power-off or reboot is the accepted denial of service, and it lands at that prompt when FileVault is on. That is the same story as BitLocker on Windows.

**Verified from the `fdesetup(8)` manual page** (read from [this mirror](https://keith.github.io/xcode-man-pages/fdesetup.8.html)): `fdesetup` reports whether FileVault is on or off, and some status commands can be run from a non-root session. `fdesetup status` indicates On or Off. `fdesetup isactive` returns `true` or `false`.

**Spike m08.** Run `fdesetup status` and `fdesetup isactive` as the console user, not root, and record the output. The wizard's disk-encryption check uses whichever of those works without an admin password. An Intel Mac without a T2 chip does not prove the Secure Enclave path. GitHub runners are not rebooted into the FileVault prompt, so the pre-boot screen is Henry's checklist line if his disk has FileVault on, and it is unproven if it does not.

**Test, tickets m08 and m17, runs in CI.** The chooser is a pure function of three results: whether `dlsym` returned the symbol, whether that call succeeded, and whether the documented path can require a password with a zero delay. It lives with the macOS `SessionControl` work and the workspace test calls it. The test does not call the live symbol, so the job does not lock the runner. The existing verify job (`cargo test --workspace` on `ubuntu-latest`, `windows-latest`, and `macos-latest` in `.github/workflows/ci.yml`) runs these four rows:

1. Symbol missing, and the documented path can require an immediate password. The documented path is the one requested.
2. Symbol present, call failed, and the documented path can require an immediate password. The documented path is the one requested.
3. Call succeeded. The documented path is not requested.
4. Documented path unavailable or password not required → never Unlocked; refuse to arm. This is the private call unavailable while password-after-screensaver is off or the delay is not zero.

A row that skips the documented path, that takes it after a successful private call, or that arms when row 4 should refuse, fails the job. m08 lands the function and the test with the probe. m17 keeps the same test green when the service and the lock host call the chooser, and it keeps the guard and the overlay up when both paths fail at run time. Henry's Mac remains the proof that either path shows a lock screen, and the sheet there records the setting and the delay.

## 5. Unlock, against the lock core

The Lock Host does not decide that the owner authenticated. The Service does, and only a `Verified` verdict carrying the nonce from the Broker launch this service started will leave `Locked`. That is `LaunchNonce` at the commit above: 32 bytes from the system CSPRNG, compared with `eq_ct`, rejected if any byte differs. The macOS Broker is a short-lived process the service starts as the console user. The nonce crosses on the inherited private channel. The public socket has no unlock verb.

**Verified.** [`LAPolicy.deviceOwnerAuthentication`](https://developer.apple.com/documentation/localauthentication/lapolicy/deviceownerauthentication) is, on macOS, Touch ID, a paired Apple Watch, or the user's password. Biometry is used first when it is available and enrolled. The Watch is tried in parallel. Otherwise the system asks for the password. The [LocalAuthentication overview](https://developer.apple.com/documentation/localauthentication) says the app gets a boolean and never sees fingerprint data or the password. The Broker calls `LAContext.evaluatePolicy`. The Lock Host does not collect the password into its own field for this method.

**Spike m09, on Henry's Intel Mac.** The system sheet has to appear above the overlay. While it is up, physical keys must not reach other apps. If the sheet's secure input disables the tap, m09 records that, and the guard treats the disable as fail-closed unless the sheet is the only place keys are going. A root daemon calling `evaluatePolicy` is not the documented sample (the sample is an app). The guess is that the Broker has to be a GUI LaunchAgent. m09 confirms or rejects that.

Touch ID is the biometry branch of the same policy. **Spike m12, Apple Silicon only.** GitHub's arm64 runners have no Touch ID sensor. An Intel Mac without a Touch ID sensor cannot run it either. Some 2018–2020 Intel portables have Touch ID through the T2 chip. If Henry's Mac is one of those, m12 moves to his machine and the "Apple Silicon only" label comes off. Apple Watch unlock is not in that bucket: the policy page includes it for macOS generally, and m09 records it if a Watch is paired with his Intel Mac.

The dev-PIN tracer (m14, m15, m16) uses the existing Argon2id check in Core, the same shape as Windows ticket 12. The Mac password is m18, after the Broker exists. Touch ID is m19, and it does not block the Intel tracer.

## 6. Service model

**Verified.** [Creating Launch Daemons and Agents](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html): a daemon plist in `/Library/LaunchDaemons` is a system job. An agent plist in `/Library/LaunchAgents` (or the user's own `LaunchAgents`) runs only while that user is logged in. `Label` is required. `KeepAlive` is how a job is asked to stay running. The page recommends on-demand jobs. SoftLock's service cannot be on-demand, because the watchdog has to be alive before anyone asks for a lock. Both jobs use `KeepAlive`. **Spike m02 and m15** show that a KeepAlive daemon and a KeepAlive agent stay up on the runner and on Henry's Mac, and that killing the daemon while locked still ends in the real lock (the lock host's own fall-back, same rule as Windows).

The daemon is the authority: state machine, journal, socket, power assertion, FileVault status. The agent is the lock host: tap, overlay, heartbeat. The Broker is the third process, spawned for one `LAContext` evaluation. **Spike m15** if a daemon can start a GUI process in the console user's session. If it cannot, the agent is started by launchd at login and the daemon talks to it. That is the Windows "service launches the lock host into the session" split, with launchd as the session launcher when the daemon cannot do it.

The socket is the named-pipe analogue. Same length-prefixed JSON, same verbs, still no public unlock. The id in the path is the existing `SERVICE_ID` (`softlock-agent`). **Spike m15** for the directory, the mode, and how the server learns the client's pid (`LOCAL_PEERPID` is the guess). The Windows pipe path stays Windows.

### Signing, notarization, hardened runtime

**Verified.** [Notarizing macOS software before distribution](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution): notarization wants a Developer ID certificate, the Hardened Runtime, a secure timestamp, and `notarytool` (not `altool`). The ticket is stapled so Gatekeeper can see it offline. [Hardened Runtime](https://developer.apple.com/documentation/security/hardened-runtime) blocks code injection and JIT unless an entitlement turns one protection off. Notarization requires the Hardened Runtime.

**Spike, only if decision D2 says to sign now.** Which entitlements the Tauri shell needs (JIT is the likely one, and it is not added until a signed build fails without it). Whether `dlopen` of `login.framework` survives notarization. Unsigned or ad-hoc builds are the tracer-bullet default, matching the Windows decision to ship unsigned first. Gatekeeper's right-click Open is the SmartScreen analogue. Decision D2 is the cost of that: TCC grants reset when the ad-hoc signature changes.

## 7. Universal 2

**Verified.** [Building a universal macOS binary](https://developer.apple.com/documentation/apple-silicon/building-a-universal-macos-binary): a universal binary contains `arm64` and `x86_64`. `lipo` merges the two executables. `lipo -archs` on a universal binary prints both names. The page's example is `x86_64 arm64`. An Intel Mac cannot run or debug the `arm64` slice. On Apple silicon the `x86_64` slice runs only under Rosetta, and that is not a native Intel run. Daemons and launch agents are on the page's list of binaries that should be universal.

**Verified.** The Tauri 2 CLI documents `--target universal-apple-darwin` as the universal macOS target, and says both `aarch64-apple-darwin` and `x86_64-apple-darwin` must be installed. The page is the [Tauri CLI reference](https://v2.tauri.app/reference/cli/). Tauri's own note, from the CLI maintainer on the docs issue, is that this target is built on a Mac.

**Verified, read 16 November 2026.** GitHub's [hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) list `macos-latest` as arm64 (Apple silicon, M1), in the same row as `macos-15` and `macos-26`. The runner-images README and the 1 October 2026 changelog disagree about whether `macos-latest` is still macOS 15 or has moved to macOS 26. Ticket m02 prints `sw_vers` and does not assume which. The same GitHub page lists Intel labels `macos-15-intel` and `macos-26-intel`. This plan does not use them. Henry's D1 puts the Intel slice on his Mac and the arm64 slice on `macos-latest`. The lead accepted D4, which is the choice not to add the Intel labels.

**Spike m01.** On `macos-latest`:

- `rustup target add x86_64-apple-darwin aarch64-apple-darwin`
- `cargo build --release --locked` of the product crates for each target, with `MACOSX_DEPLOYMENT_TARGET=11.0`
- `lipo -create` of each shipped executable
- `lipo -info` and `lipo -archs` each contain `x86_64` and `arm64`
- `lipo -thin` each slice into its own directory, and the existing release-binary test scans each directory
- `npm run tauri build -- --target universal-apple-darwin`, then the same `lipo` checks on the executable inside the app bundle

The arm64 slice is executed on that runner. The x86_64 slice is executed on Henry's Intel Mac (m11), not under Rosetta.

## 8. Seams and the release-binary scan

| Seam | What happens on macOS |
|---|---|
| **1** | Already runs on `macos-latest` in `.github/workflows/ci.yml` at the commit above (`cargo test --workspace`). It stays the state machine, the nonce, the journal, and the heartbeat. No new OS calls in Core. |
| **2** | Stays the Windows named pipe. The macOS socket gets the same JSON contract when m15 builds it, as its own test, not as a change to the Windows pipe test. |
| **3** | Stays `windows-2022` and `windows-2025`. The macOS probe is a separate workflow. It does not grow GDI, DXGI, or Windows.Graphics.Capture assumptions, and seam 3 does not grow ScreenCaptureKit assumptions. |
| **4** | Stays the Linux Playwright job. The unsupported-platform string stays until a macOS backend reports a capability as supported. |
| **Release scan** | The test `product_release_binaries` in `crates/softlock-lockhost/tests/release_binary.rs` reads `SOFTLOCK_RELEASE_SCAN_DIR` and prints `scanned release binaries`. It does not understand a fat Mach-O. m01 thins each slice and points the variable at each thinned directory, then scans the fat file as well so a marker that is not inside a slice still fails the job. At the commit above the markers are `softlock-simulated-physical-marker` and `softlock_simulated_physical_marker`. Once PR #48 merges, the scan also names `softlock-dev-launch-marker` and `softlock_dev_launch_marker`. A later ticket that adds a macOS test-only feature adds the scan in that same change. |

Simulated physical input on a runner is the nonce in `kCGEventSourceUserData`, and only in the test-only build. It is not a claim that the runner has a keyboard. Physical keys are m05 and the m14 checklist on Henry's Mac.

## 9. What this addendum will not pretend

- That `kCGEventSourceStateHIDSystemState` means a human pressed a key.
- That `NSWindow.SharingType.none` hides the overlay from ScreenCaptureKit.
- That `CGShieldingWindowLevel()` is the overlay level.
- That an assertion keeps a Mac awake with the lid shut.
- That `SACLockScreenImmediate` is a public API.
- That a missing symbol or a failed call leaves the session unlocked. Both take the documented screen-saver path when that path can require an immediate password. Otherwise D8 refuses to arm, or keeps the guard and the overlay up.
- That Touch ID can be tested on an Intel Mac that has no sensor, or on a GitHub runner.
- That a Rosetta run of the x86_64 slice replaces Henry's Mac.
