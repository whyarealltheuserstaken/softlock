# m07: power assertions, App Nap, and the lid

Planning ticket: `docs/tickets/macos/m07-keep-awake-intel.md`. Issue 56. Blocked m16.

The kit is the workspace-excluded probe `spikes/macos-keep-awake`. The product path is `IOPMAssertionCreateWithName`, with a reason string the operator can find in `pmset -g assertions`. Both idle assertions stay held through the lid and power-key rows (QA1340); they are released afterwards. `caffeinate` is only a control (`caffeinate -i -t 5 -w <kit pid>`) if that binary exists. The kit does not lock the session, does not swallow the keyboard, does not change sleep settings, and does not read a hardware serial number. Assertions are released on exit, including SIGINT without `SA_RESTART`. Control-C ends it any time; Terminal still has keyboard focus behind the black screen.

The kit never prints or stores the full `pmset -g assertions` dump. stdout, `m07.json`, and the sheet excerpt keep only the system-wide summary counts plus this probe's own lines (filtered by pid or reason). Other apps' names, paths, and device names must not be committed.

m02 (`docs/findings/m02-macos-runner-desktop.md`) already showed that `macos-latest` can host a window. This kit reuses that shape for a fully opaque covering overlay and records the timer window's occlusion state. The 60 s overlay-up sample can cover the display; the 300 s App Nap sample covers **only the timer window**, so Activity Monitor stays visible. m02 also said m07 moves entirely to Henry's Intel Mac for lid, real idle, and App Nap. The GitHub job here is the slice that can still run on the runner: create the idle assertions, print the names that compile, grep a **filtered** `pmset` excerpt for the reason string, optionally run `caffeinate` as a control, take two equal-length timer counts with the overlay down and covering the timer window, skip the 300 s hands-off App Nap step, leave hardware rows `not-run-on-runner` while the assertions are still held, then release them.

Each timer tick pumps the run loop for at most 10 ms (`untilDate` capped at `min(remaining, 0.01)`), then sleeps `interval-ms` (50). `timer-expected-ticks` is `sample_ms / (interval_ms + 10)`: **133** in an 8 s runner sample. If the uncovered count is below ~80% of that, `timer-sentence` is `not claimed: baseline below expected`. Jobs before the pump cap printed ~50 ticks because `nextEventMatchingMask` used a 0.05 s `untilDate` inside `pump(0.01)`; that was the kit.

## What the runner can prove

Filled from GitHub Actions run [44008672549](https://github.com/whyarealltheuserstaken/softlock/actions/runs/44008672549/job/133790098867) (`macos keep-awake runner` on this PR after the pump cap, head `c205326`). The job prints `sw_vers` and `uname -m`. `uname -m` must be `arm64`. The runner has no lid and no power key.

Quoted from that job log:

```
sw_vers-product=macOS
sw_vers-version=26.7.2
sw_vers-build=25G332
uname-m=arm64
assertion-reason=m07-keep-awake-probe-2163
assertion-created=true
assertion-released=true
pmset-shows-reason=true
timer-overlay-down=73
timer-overlay-up=75
timer-slowed=false
timer-sentence=not claimed: baseline below expected (73 uncovered ticks, expected 133)
timer-expected-ticks=133
overlay-opaque=true
timer-occlusion-visible-down=true
timer-occlusion-visible-up=false
app-nap-ran=false
lid-on-power-assertion-held=true
power-key-assertion-held=true
interrupted=false
result=pass
```

73 uncovered ticks is below 80% of 133, so slowing is not claimed. That is not evidence the runner was throttled, and it does not prove "did not slow".

| Claim | Runner |
|---|---|
| `PreventUserIdleSystemSleep` and `PreventUserIdleDisplaySleep` compile | job log `assertion-compiled-*` |
| `IOPMAssertionCreateWithName` holds both, reason string in filtered `pmset` | `pmset-shows-reason=true` plus the filtered excerpt |
| Assertions stay held through skipped hardware rows | `*-assertion-held=true` measured from filtered `pmset`, then `step=7` releases them |
| Assertions release, and filtered `pmset` no longer shows the reason | `assertion-released=true`, `pmset-after-shows-reason=false` |
| `caffeinate -i -t 5 -w <pid>` present or absent, used only as a control | `caffeinate-*` |
| Timer counts overlay-down and overlay-up, same duration, opaque overlay, occlusion recorded | `timer-overlay-*`, `overlay-opaque`, `timer-occlusion-visible-*`, `timer-expected-ticks` |
| Short sample is not App Nap | `--runner` is 8 s; log `timer-sentence=not claimed: baseline below expected (73 uncovered ticks, expected 133)`; `--kit` is 60 s then 300 s over the timer window only |
| Lid closed on power | `not-run-on-runner` |
| Lid closed with an external display | `not-run-on-runner` |
| Power key tap | `not-run-on-runner` |

Keep-awake for **idle sleep only** may be wired later if this runner slice stays green and Henry's sheet shows the assertions on his Mac, held through the lid row. m16 must not claim agents kept running with the lid shut until the lid row is filled, and phase 2 still does not grow a lid-close takeover from one surprise (decision D7).

## What only Henry's Intel Mac can prove

Test machine header: [Henry's sheet](../../tickets/macos/henry-sheet.md). MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports), Intel Core i5, macOS Tahoe 26.7.3, laptop, Touch ID, T2. The number of external displays is still open. macOS 11 through 15 stay untested.

He runs:

```bash
cargo run --locked --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --kit
```

That command is 60 s overlay-down, 60 s overlay-up, then 300 s App Nap covering only the timer window. Do not pass `--sample-ms 300000`; `--kit` already schedules those durations.

The binary prints numbered `step=` lines. The overlay is fully opaque, ignores the mouse, does not become key, and quits on Escape **only if Terminal has Accessibility permission**; otherwise it closes by itself after the sample. Control-C ends it any time; Terminal still has keyboard focus behind the black screen. He stays next to the machine for the lid rows. On this 2020 MacBook Pro the power key is the Touch ID button. A tap may sleep the display or lock the screen: unlock with Touch ID or your password, then answer. He taps once; he does not hold it. He does not paste a full `pmset` dump or any hardware serial number.

Rows that CI cannot fill:

1. **Lid closed on power**, no external display if he can unplug it: slept or stayed awake, with `assertion-held=true` from filtered `pmset`. This Mac is a laptop, so the row is one of those two (or skip).
2. **Lid closed with an external display** (clamshell): slept, stayed awake, or no-external-display.
3. **Power key tap** (Touch ID button): slept, showed the restart panel, or did nothing. Unlock first if the tap locked the screen, then answer. Assertions are still held for this row, then released (`step=7`).
4. **App Nap**: numbered `step=4`, overlay covering **only the timer window** for 300 s so Activity Monitor stays visible. Compare the 60 s overlay-down vs overlay-up counts on a real idle.

QA1340 says a lid close is forced sleep and an assertion cannot prevent it. The product condition for the m16 idle-sleep claim is that both assertions are held during that measurement. Clamshell-with-display is the extra measurement. Neither result turns into a takeover feature in this phase.

Paste `proof-output/m07.json` and the **filtered** `pmset-while-held` excerpt onto [SHEET.md](SHEET.md). Leave acceptance boxes unticked until that run.

## m16

m16 is the first macOS tracer bullet. It must not claim agents kept running until this sheet exists. After Henry fills it, m16 may claim keep-awake for idle sleep only if the assertions were held through the lid row and the uncovered timer kept ticking. It must not claim lid-closed keep-awake unless the lid rows contradict QA1340, and even then D7 says phase 2 does not grow that feature from a single result.
