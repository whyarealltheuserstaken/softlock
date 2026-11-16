# m07: power assertions, App Nap, and the lid

Planning ticket: `docs/tickets/macos/m07-keep-awake-intel.md`. Issue 56. Blocked m16.

The kit is the workspace-excluded probe `spikes/macos-keep-awake`. The product path is `IOPMAssertionCreateWithName`, with a reason string the operator can find in `pmset -g assertions`. `caffeinate` is only a control if that binary exists. The kit does not lock the session, does not swallow the keyboard, does not change sleep settings, and does not read a hardware serial number. Assertions are released on exit.

m02 (`docs/findings/m02-macos-runner-desktop.md`) already showed that `macos-latest` can host a window. This kit reuses that shape for a covering overlay. m02 also said m07 moves entirely to Henry's Intel Mac for lid, real idle, and App Nap. The GitHub job here is the slice that can still run on the runner: create and release the idle assertions, print the names that compile, grep `pmset` for the reason string, optionally run `caffeinate` as a control, and take two equal-length timer counts with the overlay down and covering the timer window.

## What the runner can prove

Filled from the macOS keep-awake workflow on this PR. The job prints `sw_vers` and `uname -m`. `uname -m` must be `arm64`. The runner has no lid and no power key. A short timer sample is not a real idle, so App Nap of a covered process is not claimed from CI.

| Claim | Runner |
|---|---|
| `PreventUserIdleSystemSleep` and `PreventUserIdleDisplaySleep` compile | job log `assertion-compiled-*` |
| `IOPMAssertionCreateWithName` holds both, reason string in `pmset -g assertions` | job log `pmset-shows-reason=true` plus the excerpt |
| Assertions release, and `pmset` no longer shows the reason | `assertion-released=true`, `pmset-after-shows-reason=false` |
| `caffeinate` present or absent, used only as a control | `caffeinate-*` |
| Timer counts overlay-down and overlay-up, same duration | `timer-overlay-down`, `timer-overlay-up`, `timer-sentence` |
| Lid closed on power | `not-run-on-runner` |
| Lid closed with an external display | `not-run-on-runner` |
| Power key tap | `not-run-on-runner` |

Keep-awake for **idle sleep only** may be wired later if this runner slice stays green and Henry's sheet shows the assertions on his Mac. m16 must not claim agents kept running with the lid shut until the lid row is filled, and phase 2 still does not grow a lid-close takeover from one surprise (decision D7).

## What only Henry's Intel Mac can prove

Test machine header: [Henry's sheet](../../tickets/macos/henry-sheet.md). MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports), Intel Core i5, macOS Tahoe 26.7.3, laptop, Touch ID, T2. The number of external displays is still open. macOS 11 through 15 stay untested.

He runs:

```bash
cargo run --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --kit
```

The binary prints numbered steps. The overlay ignores the mouse, does not become key, and quits on Escape or when the sample ends. He stays next to the machine for the lid rows. He taps the power key; he does not hold it.

Rows that CI cannot fill:

1. **Lid closed on power**, no external display if he can unplug it: slept or stayed awake. This Mac is a laptop, so the row is one of those two (or skip).
2. **Lid closed with an external display** (clamshell): slept, stayed awake, or no-external-display. He sometimes uses an external display, so this row is in the kit even while the display count on the sheet is still open.
3. **Power key tap**: slept, showed the restart panel, or did nothing. m06 has not recorded this yet, so the row lives here.
4. **App Nap**: whether the covered timer process slowed on a real idle, for the same duration as the uncovered sample. A runner sample that does not slow does not prove the Intel Mac will not nap.

QA1340 says a lid close is forced sleep and an assertion cannot prevent it. Clamshell-with-display is the extra measurement. Neither result turns into a takeover feature in this phase.

Paste `proof-output/m07.json` and the `pmset-while-held` excerpt onto [SHEET.md](SHEET.md). Leave acceptance boxes unticked until that run.

## m16

m16 is the first macOS tracer bullet. It must not claim agents kept running until this sheet exists. After Henry fills it, m16 may claim keep-awake for idle sleep only if the assertions held and the uncovered timer kept ticking. It must not claim lid-closed keep-awake unless the lid rows contradict QA1340, and even then D7 says phase 2 does not grow that feature from a single result.
