# macOS hardware suite

```bash
./tests/hardware/macos/run.sh
```

About 4 minutes after the first build, on Intel or Apple silicon. It builds the kits, then:

1. Posted-input and sharing probes, the lock-kit read-only check, and a short uncovered timer.
2. The screen goes black for about 60 seconds. That is the App Nap sample.
3. Close the lid on power. Press Enter after you do it. Type skip to skip.
4. Plug in a display, then close the lid. Press Enter after you do it. Type skip to skip.
5. Tap Touch ID once. Press Enter after you do it. Type skip to skip.
6. The Mac may lock twice. Unlock with your password. The suite reads the session; it does not ask three questions.

A step you confirm that is not detected is a failure. `--ci` skips steps 3–6. Send `suite-output/<timestamp>/result.json` and `result.txt`. The files use schema `softlock-hardware-suite/1` (pass, fail, or skip). No serial number is recorded. Ctrl-C releases power assertions and the suite exits non-zero. A failure ends with the session locked.
