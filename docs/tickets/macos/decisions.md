# Decisions Henry makes before macOS tickets become issues

Read against `23b5a39bc92c2ee32f470256d78be27d030dc59f`. The addendum is [`docs/platforms/macos.md`](../../platforms/macos.md). Nothing here is product code. These are not GitHub issues.

Each item has a recommendation until someone decides it. Changing an answer rewrites the tickets that name it. It does not rewrite the Windows spec.

Henry decided D1 and D5. The lead accepted D2, D3, D4, D6, and D7 on 16 November 2026, as written. The lead also decided D8 the same day. The test-machine paragraph is D1, and the other macOS files link to it. D9 is closed. Henry's decision is SPEC.md Decision 20: a remote viewer sees the SoftLock lock screen until a separate configurable pass, and remote-tool input is blocked until that pass except what the unlock screen needs. The earlier options and the lead recommendation are not the decision.

## D1. Universal 2, macOS 11, his Intel Mac, arm64 on macos-latest

**Decision, Henry, from the original grilling:** the macOS build is Universal 2 (`x86_64` and `arm64`). The minimum is macOS 11 or later, and ticket m01 keeps `MACOSX_DEPLOYMENT_TARGET=11.0`. His Intel Mac is the Intel machine. Apple silicon CI is `macos-latest`, which is arm64.

GitHub `macos-latest` is an arm64 image of macOS 15 or macOS 26 (the runner docs and the runner-images list disagreed on 16 November 2026). It will not boot macOS 11. Ticket m02 records the runner's `sw_vers`.

**Test machine, the only copy of this paragraph.** Recorded 16 November 2026, from his About This Mac. MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports). 2 GHz quad-core Intel Core i5. Intel Iris Plus Graphics. 16 GB LPDDR4X. macOS Tahoe 26.7.3. It is a laptop. It has Touch ID and the T2 chip; that model has both. Henry, 16 November 2026: he sometimes uses an external display. How many are attached at once is still unknown. Whether a second keyboard is available is still unknown. macOS 11 through 15 are untested: this machine is on 26.x, and arm64 CI is `macos-latest`. m02 still prints the runner's `sw_vers`. It does not close that gap. [Henry's sheet](henry-sheet.md) points here. Other tickets link here instead of copying this paragraph. The kits say "if the machine has one" for a second keyboard, and for a display, AirPlay receiver, iPad, or DisplayLink device that is not already connected.

## D2. Developer ID now, or unsigned like Windows

Windows v1 ships unsigned. Notarization needs an Apple Developer ID certificate, the Hardened Runtime, and `notarytool`. That is a paid program.

**Recommendation:** the tracer bullet stays unsigned or ad-hoc, and Developer ID waits for a public macOS release. The cost is that Accessibility and Input Monitoring grants are tied to the signature, so an ad-hoc rebuild asks again.

The other two options, if he wants the grants to survive a rebuild while he is testing:

- A stable local signing identity that is not a Developer ID. It works on his Mac. It is not something we ship.
- A Developer ID now, with notarization deferred. Grants stick. Gatekeeper is quieter. The private `login.framework` call D5 chose has to be checked against notarization before any notarized build.

Ticket m10 and the service ticket assume the recommendation.

**Accepted by the lead, 16 November 2026,** as written.

## D3. MDM-free TCC prompts

Accessibility and Input Monitoring are separate toggles in System Settings. A configuration profile can pre-grant them. Installing that profile needs MDM, or a profile the user approves by hand that is still an enterprise-shaped control.

The spec puts fleet and MDM management out of the public tree.

**Recommendation:** MDM-free. The wizard tells the owner which two panes to enable. No PPPC profile is written. Ticket m10 is that path.

**Accepted by the lead, 16 November 2026,** as written.

## D4. GitHub's Intel macOS runners

GitHub lists `macos-15-intel` and `macos-26-intel` as well as the arm64 `macos-latest` labels. D1 already puts the Intel slice on his Mac and the arm64 slice on `macos-latest`. This item is whether the plan also adds GitHub's Intel labels.

**Recommendation:** m01 builds both slices on `macos-latest` and does not add the Intel labels. He runs the x86_64 slice (m11). A Rosetta run on the arm64 runner is not that test. He can add the Intel labels later if he wants a second Intel data point that is not his hardware.

**Accepted by the lead, 16 November 2026,** as written.

On 16 November 2026 the hosted-runners page still listed `macos-15-intel` and `macos-26-intel`. Ticket m11's native job uses those labels to download and run the fat binaries the m01 build job uploaded. The m01 *build* job stays on `macos-latest` and still does not run on Intel.

**Lead, 16 November 2026:** m11's primary evidence is that GitHub Intel CI. Do not remove Henry's Intel Mac from m11. Keep an optional kit checkbox to run the same `--smoke` on his Intel MacBook Pro. Do not write that his Mac is not required.

## D5. Private API for the real lock

`SACLockScreenImmediate` has no Apple documentation page. It is a private symbol in `login.framework`. The public pages we have do not offer a "lock the screen now" call. The screen-saver-plus-password path is weaker and is the documented lock path.

**Decision, Henry, 16 November 2026:** use `SACLockScreenImmediate`, resolved with `dlopen` and `dlsym`. The binary does not link `login.framework`. If the symbol is missing, or the call fails, use the documented lock path (start the screen saver and require the account password). A missing symbol is not a crash. The login window is not the fallback. When that documented path cannot guarantee a password, D8 applies. His Intel Mac is the machine in D1.

## D6. Capture exclusion may be unprovable

Apple's current page for `NSWindow.SharingType.none` says not to use it to hide content from capture. ScreenCaptureKit's exclude-windows filter is chosen by the app that is capturing, not by the window.

**Recommendation:** do not promise capture exclusion on macOS until m04 and m06 show a path whose marker count is zero. If both say the marker is still visible, the macOS lock is still a physical-input lock, and the docs say agents may see the overlay. He does not have to decide that until the spikes return. The tickets are written so they fail the claim instead of inventing a replacement.

**Accepted by the lead, 16 November 2026,** as written.

## D7. No lid-close takeover on macOS

Apple's QA1340 says a power assertion cannot prevent forced sleep, and closing the lid is forced sleep.

**Recommendation:** phase 2 does not offer a lid-close takeover. Keep-awake is idle-sleep and display-sleep only. If his Mac is a laptop, m07 records what the lid actually does and does not turn that into a feature unless the measurement contradicts the QA.

**Accepted by the lead, 16 November 2026,** as written.

## D8. A failure never ends Unlocked

Windows does not return to Unlocked when arming fails or the real lock fails. macOS uses the same rule.

**Decision, the lead, 16 November 2026:**

- At arm time, if `SACLockScreenImmediate` is unavailable and the documented fallback cannot guarantee a password, refuse to arm. "Cannot guarantee" means password-after-screensaver is off, or the delay is not zero. The message tells the owner how to turn that setting on and set the delay to zero.
- At run time, if both lock paths fail, keep the guard and the overlay up, show an error, keep retrying, and release only on a verified unlock.

Tickets m08 and m17 add the chooser row: documented path unavailable or password not required → never Unlocked; refuse to arm. m08's sheet records the password-after-screensaver setting and the delay.

## D9. Remote view and remote input (macOS only)

**Decision, Henry, 16 November 2026.** Closed. The words are in SPEC.md Implementation Decision 20, and they apply on Windows 11, Windows 10, Linux, and both macOS slices. By default only a local viewer sees the real screen. A remote viewer sees the SoftLock lock screen, and sees the real screen only after a configurable pass, a separate remote credential. Remote-tool input is blocked until that pass succeeds, except what the unlock screen needs. Agent input on the local session still follows the normal rules. A crash or failure ends locked.

`NSWindow.SharingType.none` is still the flag under test (D6), on Intel and on Apple Silicon. Screen Sharing and a local capture read the same window-server image, and Apple says `.none` is a legacy constant not to be used to hide content from capture. m21's v1.x or v2 probe records whether a remote path shows the marker and whether its input arrives as a posted event. That measurement is a risk. It does not replace this decision. The display rows on m23 stay in scope. Screen Sharing hardware is v1.x or v2 with m22. v1 only frameworks the detection hook, the remote-pass types, and fail-closed.
