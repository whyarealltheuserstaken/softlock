# m06 overlay kit

One window per display at the screen-saver level, with `canJoinAllSpaces` and `fullScreenAuxiliary`. The overlay ignores the mouse and does not become key.

Teardown paths that close every covering window and still write `result.json` / `result.txt`:

- Escape, only when Accessibility is granted so the global key monitor can fire. The kit is Accessory except for the click measurement: it saves the frontmost app, becomes Regular only for that post, then closes the target, returns to Accessory, and waits until the saved app is frontmost again before any stdin prompt. A local monitor does not see Terminal keystrokes.
- Control-C / SIGINT, SIGHUP, SIGTERM (stdin is polled every 100 ms while the run loop is pumped; Ctrl-C does not wait for Enter)
- process panic or crash
- the hard `--timeout-ms` watchdog (default 10 minutes), which sets a stop flag on a background thread; the main thread closes AppKit. If teardown has not happened after about five more seconds, the watchdog calls `_exit(3)`
- a second Control-C restores the default SIGINT handler so the process can die even if teardown is stuck

From the hardware suite:

```bash
./tests/hardware/macos/run.sh
```

Standalone, on Henry's Intel Mac (checkout at `/Users/admin/softlock`):

```bash
cd /Users/admin/softlock
git fetch origin
git checkout -B cursor/m06-overlay-intel-kit-feda origin/cursor/m06-overlay-intel-kit-feda
git rev-parse HEAD
cargo run --locked --release --manifest-path spikes/macos-overlay-kit/Cargo.toml -- --kit
```

`git rev-parse HEAD` must match the SHA in the pull request body. A stale local branch of the same name will not.

Magenta covering windows appear on every attached display. Each prompt is drawn on the overlay. Capture runs first after the covers are visible (occlusion / frame, not a fixed sleep). Then the kit asks for a Space switch, a full-screen app, a click under the overlay, Command-Tab, and Force Quit. The overlay is torn down before the power-key row. **The power key is last.** On a Touch ID MacBook Pro that tap can lock or sleep the machine.

Type `skip` to skip a row, `no-external-display` if a second panel was never attached. After `overlay.sync()` the kit compares cover count to `NSScreen` count. `no-external-display` is rejected if more than one screen was ever attached. Send `overlay-output/result.json` and `result.txt`.

`--runner` is the GitHub slice: capture counts, covering every attached display, posted click/focus when Accessibility is granted. Without `CGPreflightScreenCaptureAccess` capture rows are incomplete and never pass. Spaces, full-screen, chords, the power key, and hot-plug stay `not-run-on-runner`.
