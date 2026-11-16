# Decisions Henry makes before macOS tickets become issues

Read against `23b5a39bc92c2ee32f470256d78be27d030dc59f`. The addendum is [`docs/platforms/macos.md`](../../platforms/macos.md). Nothing here is product code. These are not GitHub issues.

Each item has a recommendation. The tickets assume the recommendation until Henry writes a different answer. Changing an answer rewrites the tickets that name it. It does not rewrite the Windows spec.

## D1. Minimum macOS version

The approved spec says macOS 11 or later. The deployment target in ticket m01 is 11.0 for both slices.

GitHub `macos-latest` is an arm64 image of macOS 15 or macOS 26 (the runner docs and the runner-images list disagreed on 16 November 2026). It will not boot macOS 11. Henry's Intel Mac is the only machine in this plan that might.

**Recommendation:** keep 11.0 as the deployment target. Do not raise it in this plan. Ticket m02 records the runner's `sw_vers`. If he wants the promise to be "the OS his Intel Mac is running, and whatever `macos-latest` is," he says so and m01's `MACOSX_DEPLOYMENT_TARGET` changes.

**Also needed, not a product choice:** the model, the macOS version, laptop or desktop, how many displays, whether it has Touch ID, and whether a second keyboard is available. The kits say "if the machine has one."

## D2. Developer ID now, or unsigned like Windows

Windows v1 ships unsigned. Notarization needs an Apple Developer ID certificate, the Hardened Runtime, and `notarytool`. That is a paid program.

**Recommendation:** the tracer bullet stays unsigned or ad-hoc, and Developer ID waits for a public macOS release. The cost is that Accessibility and Input Monitoring grants are tied to the signature, so an ad-hoc rebuild asks again.

The other two options, if he wants the grants to survive a rebuild while he is testing:

- A stable local signing identity that is not a Developer ID. It works on his Mac. It is not something we ship.
- A Developer ID now, with notarization deferred. Grants stick. Gatekeeper is quieter. The private `login.framework` call, if D5 allows it, has to be checked against notarization before any notarized build.

Ticket m10 and the service ticket assume the recommendation.

## D3. MDM-free TCC prompts

Accessibility and Input Monitoring are separate toggles in System Settings. A configuration profile can pre-grant them. Installing that profile needs MDM, or a profile the user approves by hand that is still an enterprise-shaped control.

The spec puts fleet and MDM management out of the public tree.

**Recommendation:** MDM-free. The wizard tells the owner which two panes to enable. No PPPC profile is written. Ticket m10 is that path.

## D4. GitHub's Intel macOS runners

GitHub lists `macos-15-intel` and `macos-26-intel` as well as the arm64 `macos-latest` labels. Henry's instruction for this plan is that his Intel Mac is the Intel machine, and Apple silicon is `macos-latest`.

**Recommendation:** m01 builds both slices on `macos-latest` and does not add the Intel labels. He runs the x86_64 slice (m11). A Rosetta run on the arm64 runner is not that test. He can add the Intel labels later if he wants a second Intel data point that is not his hardware.

## D5. Private API for the real lock

`SACLockScreenImmediate` has no Apple documentation page. It is a private symbol in `login.framework`. The public pages we have do not offer a "lock the screen now" call. The screen-saver-plus-password path is weaker and is the documented-shaped fallback.

**Recommendation:** the spike may `dlopen` the symbol. It does not link the framework. If the symbol is missing, the lock fails visibly and the spike tries the next candidate. If he wants private SPI forbidden, he says so now and m08 drops that candidate before anyone runs it.

## D6. Capture exclusion may be unprovable

Apple's current page for `NSWindow.SharingType.none` says not to use it to hide content from capture. ScreenCaptureKit's exclude-windows filter is chosen by the app that is capturing, not by the window.

**Recommendation:** do not promise capture exclusion on macOS until m04 and m06 show a path whose marker count is zero. If both say the marker is still visible, the macOS lock is still a physical-input lock, and the docs say agents may see the overlay. He does not have to decide that until the spikes return. The tickets are written so they fail the claim instead of inventing a replacement.

## D7. No lid-close takeover on macOS

Apple's QA1340 says a power assertion cannot prevent forced sleep, and closing the lid is forced sleep.

**Recommendation:** phase 2 does not offer a lid-close takeover. Keep-awake is idle-sleep and display-sleep only. If his Mac is a laptop, m07 records what the lid actually does and does not turn that into a feature unless the measurement contradicts the QA.
