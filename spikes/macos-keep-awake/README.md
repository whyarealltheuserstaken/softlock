# m07 keep-awake kit

```bash
cargo run --locked --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --kit
```

The first three lines of output are the plan. The kit holds idle-sleep assertions, times a timer with the overlay down, then blacks the screen for about 60 seconds while it scores App Nap and throttling from timer-tick drift (no Activity Monitor). It then asks for three physical steps only: lid on power, lid with an external display, Touch ID tap. Enter skips any of those. Control-C ends it; nothing is locked and no settings change.

Send `keep-awake-output/result.json` and `keep-awake-output/result.txt`.

`--runner` is the GitHub slice (assertions, filtered `pmset`, optional `caffeinate`, short timers). Lid and App Nap stay `not-run-on-runner`.
