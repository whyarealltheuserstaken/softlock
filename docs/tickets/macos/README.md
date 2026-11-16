# macOS phase 2 tickets

Planning only, 16 November 2026. Read against `23b5a39bc92c2ee32f470256d78be27d030dc59f`. The spec addendum is [`docs/platforms/macos.md`](../../platforms/macos.md). Decisions are [`decisions.md`](decisions.md). These files are not GitHub issues. Henry approves them first.

Style matches the Windows tickets on the v1 roadmap ([issue 33](https://github.com/whyarealltheuserstaken/softlock/issues/33)): what to build, blocked by, where it can be proven, acceptance checkboxes. Spikes come first. The tracer bullet is m14, then m15, then m16.

**Where** uses three labels:

- **CI only.** GitHub `macos-latest` (arm64). No Intel Mac.
- **Henry's Intel Mac.** His machine. A Rosetta run on an arm64 runner does not count.
- **Apple Silicon only.** Cannot be proven on Intel. The arm64 runners also cannot prove these, because they have no Touch ID sensor.

A ticket whose tests are CI and whose checklist line is his Mac is labeled with both.

## Numbered list

| Id | Title | Blocked by | Where |
|---|---|---|---|
| [m01](m01-universal-ci.md) | Universal 2 CI job: both slices, `lipo`, release scan on each | none | CI only |
| [m02](m02-runner-desktop.md) | Spike: can `macos-latest` host a window, posted events, capture, and a LaunchAgent? | none | CI only |
| [m03](m03-tap-classify-ci.md) | Spike: posted-event fields, tap disable, liveness marker | m02 | CI only |
| [m04](m04-capture-ci.md) | Spike: `sharingType` none against the capture paths | m02 | CI only |
| [m05](m05-physical-input-intel.md) | Kit: physical vs posted input on the Intel Mac | m03 | Henry's Intel Mac |
| [m06](m06-overlay-intel.md) | Kit: overlay level, Spaces, full-screen, every display | m04 | Henry's Intel Mac |
| [m07](m07-keep-awake-intel.md) | Kit: power assertions, App Nap, lid if there is one | m02 | Henry's Intel Mac |
| [m08](m08-fail-closed-intel.md) | Kit: the two D5 lock paths, the fallback test, and FileVault status | m02 for the sheet; the CI rows wait on nothing | Henry's Intel Mac, and CI for the chooser |
| [m09](m09-password-sheet-intel.md) | Kit: `LAContext` password sheet above the overlay | m05 | Henry's Intel Mac |
| [m10](m10-tcc-prompts-intel.md) | Kit: Accessibility and Input Monitoring, no MDM | m05, D3 | Henry's Intel Mac |
| [m11](m11-x86-slice-intel.md) | Run the x86_64 slice natively | m01, m02 | Henry's Intel Mac |
| [m12](m12-touch-id.md) | Kit: Touch ID | m09, an Apple silicon Mac with Touch ID | Apple Silicon only |
| [m14](m14-lock-host.md) | Lock Host: block physical input, pass posted input, dev PIN | m01, m03, m04, m05, m06, m10, m11 | CI only and Henry's Intel Mac |
| [m15](m15-service.md) | Service: LaunchDaemon, LaunchAgent, socket, Broker nonce | m14, m02 | CI only and Henry's Intel Mac |
| [m16](m16-app-tracer.md) | App hotkey and status, first macOS tracer bullet | m15, m07 | Henry's Intel Mac |
| [m17](m17-fail-closed.md) | Fail closed on `SACLockScreenImmediate`, then the documented path | m08, m15 | CI and Henry's Intel Mac |
| [m18](m18-password-unlock.md) | Unlock with the Mac password through the Broker | m09, m15 | Henry's Intel Mac |
| [m19](m19-touch-id-unlock.md) | Unlock with Touch ID through the same Broker | m12, m18 | Apple Silicon only |

m13 is unused so the tracer bullet can keep the numbers m14, m15, m16 in the same order as the Windows first bullet (lock host, service, app).

## Can start immediately

m01 and m02, in parallel, after Henry approves this set. They do not wait on his Mac.

## Waves

1. m01, m02
2. m03, m04, and m11 once m01's binary and m02's probe exist
3. m05, m06, m07, m08 (his Mac; m05 waits on m03, m06 on m04)
4. m09, m10
5. m14
6. m15
7. m16, and in parallel m17 and m18
8. m12 and m19 when an Apple silicon Mac with Touch ID exists

Critical path for the first demo on his Intel Mac, the longest chain in the Blocked-by lines: m02 → m03 → m05 → m10 → m14 → m15 → m16.

m14 is also blocked by m04 → m06, by m01, and by m11. Those chains are shorter and join at m14: m02 → m04 → m06 → m14, and m01 → m11 → m14. m07 joins at m16 (m02 → m07 → m16) and is shorter than the chain through m15. D3 is already accepted, so it adds no ticket. Touch ID stays off this path.

## Blocking edges

```text
m01 --> m11
m01 --> m14
m02 --> m03
m02 --> m04
m02 --> m07
m02 --> m08
m02 --> m11
m02 --> m15
m03 --> m05
m03 --> m14
m04 --> m06
m04 --> m14
m05 --> m09
m05 --> m10
m05 --> m14
m06 --> m14
m07 --> m16
m08 --> m17
m09 --> m12
m09 --> m18
m10 --> m14
m11 --> m14
m12 --> m19
m14 --> m15
m15 --> m16
m15 --> m17
m15 --> m18
m18 --> m19
```

D3 is accepted by the lead and does not block m10. Henry decided D1 (Universal 2, macOS 11, his Intel Mac, arm64 on `macos-latest`) and D5 (`SACLockScreenImmediate` via `dlopen`, then the documented screen-saver path). D5 does not block m08. The lead accepted D2, D4, D6, and D7. The lead decided D8: a failure never ends Unlocked. The Intel Mac details in D1 are still open. D6 blocks any ticket that would claim capture exclusion beyond what m04 and m06 measured. D2 blocks a signing ticket, and this set does not contain one.
