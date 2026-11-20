# m05 checklist

From `/Users/admin/softlock`:

```bash
cd /Users/admin/softlock
git fetch origin
git checkout cursor/m05-physical-input-intel-adaf
uname -m
./tests/hardware/macos/run.sh --build-only
./tests/hardware/macos/run.sh
```

`uname -m` must print `x86_64` on this Intel Mac. The physical-input kit is the native host slice. Accessibility and Input Monitoring must be granted for Terminal (or iTerm). The tap is listen-only: keys still reach apps.

Or run only this kit:

```bash
cd /Users/admin/softlock
cargo run --locked --release --manifest-path spikes/macos-physical-input/Cargo.toml -- --kit
```

Type one key, click, scroll the trackpad, then follow the password-field prompt. Type `skip` if a row does not apply. Plug in a spare USB keyboard only if you have one. Control-C releases the tap.

Send `suite-output/<timestamp>/result.json` and `result.txt`, or `physical-input-output/result.json` and `result.txt` from the kit-only command.
