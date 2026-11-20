# m06 checklist

Henry runs this only after review. Checkout at `/Users/admin/softlock`.

```bash
cd /Users/admin/softlock
git fetch origin
git checkout -B cursor/m06-overlay-intel-kit-feda origin/cursor/m06-overlay-intel-kit-feda
git rev-parse HEAD
./tests/hardware/macos/run.sh --build-only
./tests/hardware/macos/run.sh
```

Standalone overlay kit:

```bash
cd /Users/admin/softlock
cargo run --locked --release --manifest-path spikes/macos-overlay-kit/Cargo.toml -- --kit
```

Magenta covering windows appear on every attached display. Capture runs first. Then click through, Command-Tab, Force Quit, a Space switch, one full-screen app, and optionally plug in a display. The overlay comes down before the power-key row. **Power key is last** and can lock or sleep a Touch ID MacBook Pro. Type `skip` to skip a row. Type `no-external-display` only if a second panel was never attached. Ctrl-C, the timeout, panic, and Escape (needs Accessibility) tear the overlay down.

Send `overlay-output/result.json` and `result.txt`, or the suite folder `suite-output/<timestamp>/`.
