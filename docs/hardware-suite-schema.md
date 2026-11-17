# Hardware suite result schema

`softlock-hardware-suite/1` is the shared result for the Linux, macOS, and Windows hardware suites. A suite writes `result.json` (this schema) and a short `result.txt`.

Check a file with:

```sh
python3 tests/hardware/check-schema.py path/to/result.json
```

## result.json

| Field | Type | Rule |
|---|---|---|
| `schema` | string | `softlock-hardware-suite/1` |
| `suite` | string | `linux`, `macos`, or `windows` |
| `started_utc` | string | suite clock label |
| `finished_utc` | string | suite clock label |
| `duration_ms` | number | whole run |
| `mode` | string | how it was invoked (`ci`, `admin`, `interactive`, …) |
| `config` | object | detected machine. No serials, UUIDs, or MAC addresses |
| `tests` | array | one object per row |
| `summary` | object | `pass`, `fail`, `skip` counts matching `tests` |
| `fail_closed` | bool | `true` when a failure requests the OS lock and never an unlock |
| `serial_collected` | bool | must be `false` |
| `restored` | bool | grabs, inhibitors, and registered display modes were put back |
| `interrupted` | bool | Ctrl-C / EOF-style stop |
| `state` | string | `never-unlocked` or `os-locked` |

### tests[]

| Field | Type | Rule |
|---|---|---|
| `id` | string | stable row id |
| `name` | string | short label |
| `status` | string | `pass`, `fail`, or `skip` |
| `skip_reason` | string or null | required for `skip`, null otherwise |
| `physical` | bool | a person had to move hardware |
| `needs_admin` | bool | root or an admin flag |
| `duration_ms` | number | this row |
| `detail` | string | one line for the table |
| `measurements` | object | numbers and short strings. No serials |

`skip_reason` is one of:

- `no-input` — EOF, or a prompt with nothing to read
- `budget` — the five-minute budget dropped the row
- `needs-admin` — not run without the admin flag
- `not-applicable` — this machine cannot run the row (no lid, no session, no journal)
- `ci` — `--ci` does not prompt or change the display
- `operator` — the person typed `skip`
- `interrupted` — Ctrl-C before the row ran

A step that was attempted and not observed is `fail`, not `skip`. Skips that depend on the machine are decided before any prompt.

## result.txt

Plain text, about one screen. First lines:

```text
<Suite> hardware suite  schema=softlock-hardware-suite/1  suite=<suite>
pass=N fail=N skip=N duration=Ns state=<state> serial-collected=false restored=<bool> interrupted=<bool>
```

Then one row per test: id, status, and a short note.
