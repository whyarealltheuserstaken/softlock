# m07 kit checklist (Henry's Intel Mac)

Stay at the Mac. This kit does not lock it. It does not install a service. It does not change `pmset` sleep settings. It does not read a hardware serial number. Both idle assertions stay held through the lid and power-key steps, then are released. Control-C releases them, writes `interrupted=true`, and exits non-zero.

Build and run from the repo root:

```bash
cargo run --locked --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --kit
```

Follow the numbered `step=` lines. The overlay is fully opaque, ignores the mouse, and is not key. Escape ends it early **only if Terminal has Accessibility permission** (System Settings → Privacy & Security → Accessibility). Otherwise the overlay closes by itself after about 60 s. Keyboard input stays with whatever was focused.

1. Confirm the log's filtered `pmset-while-held` excerpt shows the `m07-keep-awake-probe-*` reason and `PreventUserIdleSystemSleep` / `PreventUserIdleDisplaySleep`. It must not name other apps, paths, or devices. Do not paste a full `pmset -g assertions` dump.
2. Write down `timer-overlay-down` and `timer-overlay-up`, `timer-occlusion-visible-*`, `overlay-opaque=true`, and the `timer-sentence`.
3. Hands-off App Nap (`step=4`): leave the overlay covering the timer for 300 s (`--sample-ms 300000`). Do not touch the Mac. In Activity Monitor, show the App Nap column and note whether the timer process naps. Write `app-nap-count`.
4. Lid closed on power, no external display if you can unplug it. Stay next to the machine. After about 20 seconds, open the lid. Type `slept`, `awake`, or `skip`. Do not type `no-lid`. Confirm `lid-on-power-assertion-held=true`.
5. If you use an external display, plug it in, wait until it shows the desktop, close the lid, wait, open the lid. Type `slept`, `awake`, `no-external-display`, or `skip`. Confirm `lid-external-assertion-held=true`.
6. Tap the power key once. Do not hold it. Type `slept`, `restart-panel`, `nothing`, or `skip`. Confirm `power-key-assertion-held=true`.
7. After that row the process releases both assertions. Confirm `assertion-released=true`, `reversible=true`, `interrupted=false`, and `step=7` in the log.
8. Copy `proof-output/m07.json` and the **filtered** `pmset-while-held` excerpt onto `SHEET.md`. Never commit another app's name, a path, a device name, or a hardware serial number.

The GitHub `macos-latest` job is not this checklist. It only runs `--runner`.
