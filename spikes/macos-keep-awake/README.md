# m07 keep-awake kit

Henry runs this on his Intel Mac. The GitHub `macos-latest` job runs only the
automated slice: assertion create, a filtered `pmset` reason string, the
`caffeinate -i -t 5 -w <pid>` control if that binary exists, an 8 s timer
sample with the overlay down and up, hardware rows left `not-run-on-runner`
while the assertions are still held, then release. It does not close a lid,
tap the power key, or claim App Nap.

`--kit` is 60 s overlay-down, 60 s overlay-up, then 300 s App Nap covering
only the timer window so Activity Monitor stays visible. Before step 4, open
Activity Monitor and add the App Nap column (View → Columns), then keep your
hands off. All three processes are named `macos-keep-awake`; find the timer
by the printed `timer-pid=`. Control-C ends it any time; Terminal still has
keyboard focus behind the black screen (steps 1 and 3). Closing Terminal
sends SIGHUP and records `interrupted=true`.

This probe does not lock the session, does not install launchd jobs, does not
change sleep settings, and does not read a hardware serial number. Power
assertions stay held through the lid and power-key steps (QA1340) and are
released afterwards, including SIGINT/SIGHUP/SIGTERM without `SA_RESTART`. The
overlay is fully opaque, ignores mouse events, does not become key, and quits
on Escape only if Terminal has Accessibility permission; otherwise it closes
by itself after the sample. On Henry's 2020 MacBook Pro the power key is the
Touch ID button; a tap may sleep the display or lock the screen.

The product path is `IOPMAssertionCreateWithName`. `caffeinate` is a control
only if it is on `PATH`.

```bash
cargo run --locked --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --kit
```

The kit prints numbered steps, then a `key=value` result. On the runner:

```bash
cargo run --locked --release --manifest-path spikes/macos-keep-awake/Cargo.toml -- --runner
```
