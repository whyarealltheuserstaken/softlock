# m05 checklist

Kit only. From `/Users/admin/softlock`:

```bash
cd /Users/admin/softlock
git fetch origin
git checkout cursor/m05-physical-input-intel-adaf
uname -m
cargo run --locked --release --manifest-path spikes/macos-physical-input/Cargo.toml -- --kit
```

`uname -m` must print `x86_64` on this Intel Mac. Accessibility and Input Monitoring must be granted for Terminal (or iTerm). If a grant is missing, the kit names it and says to restart Terminal. The tap is listen-only: keys still reach apps.

Do **not** run bare `./tests/hardware/macos/run.sh` for this ticket. That is the full suite (black screen, lid, lock). `run.sh --build-only` is only the compile step.

Type one key, click, scroll the trackpad. The kit then posts from a second process (the probe window must be key). Leave any password field until asked, then focus a password field when asked, then leave it when that row is done. Type `skip` if a row does not apply. Plug in a spare USB keyboard only if you have one. Control-C releases the tap.

Send `physical-input-output/result.json` and `result.txt` from the kit-only command.
