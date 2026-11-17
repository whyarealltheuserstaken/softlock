# Linux hardware suite

One command. Builds `--locked --release`, runs unit tests, then the live probes.

```sh
./tests/hardware/linux/run.sh
```

`--ci` is the GitHub ubuntu job: no prompts, skip physical and admin rows, skip when the machine has no seat/display. `--admin` adds evdev grab and uinput create. Physical rows (lid, plug/unplug display, power key, password unlock) are one line each: Enter to wait for logind/DRM/journal, `skip` to skip.

Writes `suite-output/<utc>/result.json` and a short `result.txt`, and prints a table. Schema `softlock-hardware-suite/1` (same as the macOS and Windows suites). Failures OS-lock (`loginctl lock-session`) and never unlock. Ctrl-C restores grabs, uinput, inhibitors, and xrandr. No DMI serials, EDID, or device uniq ioctls.

Does not install a desktop, xrdp, VNC, or compositor. Rows from the phase-3 sketch and l01/l02/l04: session type, DRM/xrandr/wlr-randr, inhibit, suspend modes (read-only), remote binaries present-or-not.

Whole run is under five minutes. Not a workspace member; not shipped.
