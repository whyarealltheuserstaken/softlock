# m07 keep-awake kit

Henry runs this on his Intel Mac. The GitHub `macos-latest` job runs only the
automated slice: assertion create, a filtered `pmset` reason string, the
`caffeinate -i -t 5 -w <pid>` control if that binary exists, a short timer
sample with the overlay down and up, hardware rows left `not-run-on-runner`
while the assertions are still held, then release. It does not close a lid,
tap the power key, or claim App Nap.

This probe does not lock the session, does not install launchd jobs, does not
change sleep settings, and does not read a hardware serial number. Power
assertions stay held through the lid and power-key steps (QA1340) and are
released afterwards, including SIGINT/SIGTERM without `SA_RESTART`. The
overlay is fully opaque, ignores mouse events, does not become key, and quits
on Escape only if Terminal has Accessibility permission; otherwise it closes
by itself after about 60 s. Keyboard input stays with whatever was focused.

The product path is `IOPMAssertionCreateWithName`. `caffeinate` is a control
only if it is on `PATH`.

```bash
cargo run --locked --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --kit
```

The kit prints numbered steps, then a `key=value` result. On the runner:

```bash
cargo run --locked --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --runner
```
