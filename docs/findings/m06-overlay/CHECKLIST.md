# m06 checklist

Henry runs this only after review. Checkout at `/Users/admin/softlock`.

```bash
cd /Users/admin/softlock
git fetch origin
git checkout <branch>
./tests/hardware/macos/run.sh --build-only
./tests/hardware/macos/run.sh
```

Standalone overlay kit:

```bash
cd /Users/admin/softlock
cargo run --locked --release --manifest-path spikes/macos-overlay-kit/Cargo.toml -- --kit
```

Magenta covering windows appear on every attached display. Capture runs first. Then click through, Command-Tab, Force Quit, the power key, a Space switch, one full-screen app, and optionally plug in a display. Type `skip` to skip a row. Type `no-external-display` if you have no second panel. Escape or Control-C tears the overlay down.

Send `overlay-output/result.json` and `result.txt`, or the suite folder `suite-output/<timestamp>/`.
