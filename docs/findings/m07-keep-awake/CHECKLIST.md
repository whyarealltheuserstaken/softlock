# m07 checklist

Run the whole macOS v1 suite. One command, about 4 minutes after the first build:

```bash
./tests/hardware/macos/run.sh
```

The screen goes black for about 60 seconds. Then close the lid on power, plug a display and close the lid, and tap Touch ID. Enter skips a step. The Mac may lock; unlock with your password.

Send `suite-output/<timestamp>/result.json` and `result.txt`.
