# macOS hardware suite

```bash
./tests/hardware/macos/run.sh
```

About 4 minutes after the first build, on Intel or Apple silicon. It builds the kits, then:

1. Posted-input and sharing probes, the lock-kit read-only check, and a short uncovered timer.
2. The screen goes black for about 60 seconds. That is the App Nap sample.
3. Close the lid on power. Enter skips.
4. Plug in a display, then close the lid. Enter skips.
5. Tap Touch ID once. Enter skips.
6. The Mac may lock twice. Unlock with your password. The suite reads the session; it does not ask three questions.

`--ci` skips steps 3–6. Send `suite-output/<timestamp>/result.json` and `result.txt`. No serial number is recorded. Ctrl-C releases power assertions. A failed lock stays locked.
