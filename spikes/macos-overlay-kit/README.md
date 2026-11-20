# m06 overlay kit

One window per display at the screen-saver level, with `canJoinAllSpaces` and `fullScreenAuxiliary`. The overlay ignores the mouse and does not become key. Escape or Control-C tears it down and still writes `result.json` / `result.txt`.

From the hardware suite:

```bash
./tests/hardware/macos/run.sh
```

Standalone, on Henry's Intel Mac (checkout at `/Users/admin/softlock`):

```bash
cd /Users/admin/softlock
git fetch origin
git checkout <branch>
cargo run --locked --release --manifest-path spikes/macos-overlay-kit/Cargo.toml -- --kit
```

Magenta covering windows appear on every attached display. Capture runs first (the screen stays covered). Then the kit asks for a Space switch, a full-screen app, a click under the overlay, Command-Tab, Force Quit, the power key, and an optional external display. Type `skip` to skip a row, `no-external-display` if none is available. Send `overlay-output/result.json` and `result.txt`.

`--runner` is the GitHub slice: capture counts, covering every attached display, posted click/focus when Accessibility is granted. Spaces, full-screen, chords, the power key, and hot-plug stay `not-run-on-runner`.
