# m07 kit checklist (Henry's Intel Mac)

Stay at the Mac. This kit does not lock it. It does not install a service. It does not change `pmset` sleep settings. It does not read a hardware serial number. Both idle assertions stay held through the lid and power-key steps, then are released. Control-C ends it any time, writes `interrupted=true`, and exits non-zero. Terminal still has keyboard focus behind the black screen.

Build and run from the repo root (`--kit` is 60 s overlay-down, 60 s overlay-up, then 300 s App Nap over the timer window only):

```bash
cargo run --locked --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --kit
```

Follow the numbered `step=` lines. The overlay is fully opaque, ignores the mouse, and is not key. Escape ends it early **only if Terminal has Accessibility permission** (System Settings → Privacy & Security → Accessibility). Otherwise the overlay closes by itself after the sample (about 60 s on step=3, 300 s on step=4).

1. Stay at the Mac. Confirm the kit does not lock it and does not swallow the keyboard. Control-C ends it any time; Terminal still has keyboard focus behind the black screen.
2. Confirm the log's filtered `pmset-while-held` excerpt shows the `m07-keep-awake-probe-*` reason and `PreventUserIdleSystemSleep` / `PreventUserIdleDisplaySleep`. It must not name other apps, paths, or devices. Do not paste a full `pmset -g assertions` dump.
3. 60 s overlay-down, then 60 s covering that window. Write down `timer-overlay-down`, `timer-overlay-up`, `timer-occlusion-visible-*`, `overlay-opaque=true`, and the `timer-sentence`. If the uncovered count is below ~80% of `timer-expected-ticks`, the log prints `timer-slowed=not-claimed`. A kit run is `result=incomplete` if App Nap did not run, the App Nap count is missing, or a hardware row was skipped.
4. Before this step, open Activity Monitor and add the App Nap column (View → Columns), then keep your hands off. `--kit` covers **only the timer window** for 300 s. All three processes are named `macos-keep-awake`; find the timer by the printed `timer-pid=`. Write `app-nap-count`. Control-C ends it any time.
5. Lid closed on power, no external display if you can unplug it. Stay next to the machine. After about 20 seconds, open the lid. Type `slept`, `awake`, or `skip`. Do not type `no-lid`. Confirm `lid-on-power-assertion-held=true`.
6. If you use an external display, plug it in, wait until it shows the desktop, close the lid, wait, open the lid. Type `slept`, `awake`, `no-external-display`, or `skip`. Confirm `lid-external-assertion-held=true`.
7. On this 2020 MacBook Pro the power key is the Touch ID button. A tap may sleep the display or lock the screen: unlock with Touch ID or your password, then answer. Tap once; do not hold it. Type `slept`, `restart-panel`, `nothing`, or `skip`. Confirm `power-key-assertion-held=true`. After this row the process releases both assertions. Confirm `assertion-released=true`, `reversible=true`, `interrupted=false`. Copy `proof-output/m07.json` and the **filtered** `pmset-while-held` excerpt onto `SHEET.md`. Never commit another app's name, a path, a device name, or a hardware serial number.

The GitHub `macos-latest` job is not this checklist. It only runs `--runner`.
