# m07 keep-awake kit

Henry runs this on his Intel Mac. The GitHub `macos-latest` job runs only the
automated slice: assertion create and release, the `pmset` reason string, the
`caffeinate` control if that binary exists, and a short timer sample with the
overlay down and up. It does not close a lid, tap the power key, or claim App
Nap.

This probe does not lock the session, does not install launchd jobs, does not
change sleep settings, and does not read a hardware serial number. Power
assertions are released on exit, including SIGINT and SIGTERM. The overlay
ignores mouse events, does not become key, and quits on Escape or when the
sample ends. Keyboard input stays with whatever was focused.

The product path is `IOPMAssertionCreateWithName`. `caffeinate` is a control
only if it is on `PATH`.

```bash
cargo run --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --kit
```

The kit prints numbered steps, then a `key=value` result. On the runner:

```bash
cargo run --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --runner
```
