# Windows hardware suite

One command, from a normal PowerShell window. It does not ask for administrator and it does not show a User Account Control prompt.

```powershell
powershell -ExecutionPolicy Bypass -File tests\hardware\windows\run.ps1
```

The command builds this folder's parser with `cargo test --locked --release`, then runs the rows. A full run finishes within 5 minutes. Results are `suite-output\<timestamp>\result.json` and `result.txt`.

`--admin` adds the HKLM policy read-back and the fail-closed lock check. Without it, those rows are skipped. Pass `--admin` from a prompt that is already elevated. `--ci` skips every physical step and is what the `windows-2022` and `windows-2025` runners use. This command never installs the desktop-proof or input-hook service.

Physical steps are one line each, and only when this machine can do them. Press Enter, or type `skip`. The suite reads the result itself (display count, power messages, session notifications).

It puts back every HKCU value it changed, drops the hook, and clears the keep-awake request, including after Ctrl+C. A failed run does not unlock Windows.
