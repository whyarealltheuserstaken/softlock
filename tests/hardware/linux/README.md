# Linux hardware suite

One command. Builds `--locked --release`, runs unit tests, then the probes.

```sh
./tests/hardware/linux/run.sh
```

`--ci` skips prompts, admin rows, and display changes. `--admin` runs the evdev grab and uinput create. Physical rows (lid, hot-plug, power key, password unlock) are one line each: Enter, or `skip`. EOF is `no-input` and does not count as Enter. Ctrl-C stops the prompt and exits non-zero.

Writes `suite-output/<utc>/result.json` and a short `result.txt`. Schema [`docs/hardware-suite-schema.md`](../../../docs/hardware-suite-schema.md) (`softlock-hardware-suite/1`). A failure while armed runs `loginctl lock-session` and never unlocks. Ctrl-C releases grabs, uinput, and inhibitors, and reapplies a registered xrandr mode with `--rate`. Closing the lid cannot drop the lid inhibit while the lid is still closed. No DMI serials or device-uniq ioctls.

Does not install a desktop or a remote server. Rows cover the phase-3 sketch and l01/l02/l04: session, DRM/xrandr/wlr-randr, logind inhibit, suspend modes (read-only), remote binaries present or not.

Under five minutes. Not a workspace member.
