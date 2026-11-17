# m07 kit checklist (Henry's Intel Mac)

```bash
cargo run --locked --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --kit
```

Stay at the Mac. The first three lines say what will happen. The screen goes black for about 60 seconds; that is expected. Close the lid on power, then with a display if you have one, then tap Touch ID once. Enter skips a step. The kit measures App Nap, throttling, and sleep itself.

Send `keep-awake-output/result.json` and `keep-awake-output/result.txt`. Do not send a serial number or a full `pmset` dump.

The GitHub job is `--runner` only.
