# m07 kit checklist (Henry's Intel Mac)

Stay at the Mac. This kit does not lock it. It does not install a service. It does not change `pmset` sleep settings. It does not read a hardware serial number. Assertions are released when the process exits, including Control-C.

Build and run from the repo root:

```bash
cargo run --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --kit
```

Follow the numbered `step=` lines. The overlay ignores the mouse and is not key. Escape ends it early. Keyboard input stays with whatever was focused.

1. Confirm `pmset -g assertions` in the log shows the `m07-keep-awake-probe-*` reason and `PreventUserIdleSystemSleep` / `PreventUserIdleDisplaySleep`.
2. Write down `timer-overlay-down` and `timer-overlay-up` and the `timer-sentence` (whether the covered process slowed).
3. Lid closed on power, no external display if you can unplug it. Stay next to the machine. After about 20 seconds, open the lid. Type `slept`, `awake`, or `skip`. Do not type `no-lid`.
4. If you use an external display, plug it in, wait until it shows the desktop, close the lid, wait, open the lid. Type `slept`, `awake`, `no-external-display`, or `skip`.
5. Tap the power key once. Do not hold it. Type `slept`, `restart-panel`, `nothing`, or `skip`.
6. Confirm the process printed `assertion-released=true` and `reversible=true`.
7. Copy `proof-output/m07.json` and the `pmset-while-held` excerpt onto `SHEET.md`.

The GitHub `macos-latest` job is not this checklist. It only runs `--runner`.
