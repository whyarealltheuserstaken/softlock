# softlock-hardware-suite/1

Shared result for the macOS and Linux hardware suites. `result.json` is this object. `result.txt` starts with the schema name and a summary line `pass=N fail=N skip=N`.

Statuses are `pass`, `fail`, and `skip`. A skip carries `skip_reason`:

- `no-input` — stdin was already closed
- `budget` — the five-minute budget was already spent
- `needs-admin` — the step needs a privilege the run does not have
- `not-applicable` — the step does not apply, or the operator typed skip
- `ci` — `--ci` does not prompt

Skips are decided before the prompt. A physical step the operator confirms, that the suite does not detect, is `fail`. Detection uses events after the prompt. Ctrl-C during a prompt exits non-zero. A failure on a machine that can lock ends locked and does not unlock. An assertion or setting that cannot be taken is `fail`.

```json
{
  "schema": "softlock-hardware-suite/1",
  "suite": "macos",
  "started_utc": "0000000000000000",
  "finished_utc": "0000000000000001",
  "duration_ms": 0,
  "mode": "ci",
  "config": {},
  "tests": [
    {
      "id": "example",
      "name": "Example",
      "status": "skip",
      "skip_reason": "ci",
      "physical": false,
      "needs_admin": false,
      "duration_ms": 0,
      "detail": "",
      "measurements": {}
    }
  ],
  "summary": { "pass": 0, "fail": 0, "skip": 1 },
  "fail_closed": true,
  "serial_collected": false,
  "restored": true,
  "interrupted": false,
  "state": "never-unlocked"
}
```
