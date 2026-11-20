# macOS hardware suite

```bash
./tests/hardware/macos/run.sh --build-only
./tests/hardware/macos/run.sh
```

`--build-only` is the cold compile. A 2020 Intel i5 does not finish that compile plus the hardware steps in five minutes, so it is its own step. The next `./tests/hardware/macos/run.sh` reuses `target/hardware-suite`. `result.txt` prints `build=` and `test=` seconds. A warm run stays inside five minutes.

Needs Accessibility (posted input), Input Monitoring (event tap), and Screen Recording (sharing). Microphone is not used. The suite asks once. A missing grant skips that row.

1. Lock-kit read-only, posted input, physical vs posted input (m05), sharing, the m06 overlay (magenta covering windows on every display; Escape tears them down), and the x86_64 slice of the universal smoke binary.
2. The screen goes black for about 60 seconds. That is the App Nap sample.
3. Close the lid on power. The Mac sleeps and locks. Unlock, then press Enter. Type skip to skip.
4. Plug in a display, then close the lid. Unlock, then press Enter. Type skip to skip.
5. Tap Touch ID once. Press Enter after you do it. Type skip to skip.
6. The Mac may lock twice. Unlock with your password.

A step you confirm that is not detected is a failure. `--ci` skips steps 3–6. Send `suite-output/<timestamp>/result.json` and `result.txt`. The files use schema `softlock-hardware-suite/1`. No serial number is recorded. Ctrl-C releases power assertions and the suite exits non-zero. A failure ends with the session locked.
