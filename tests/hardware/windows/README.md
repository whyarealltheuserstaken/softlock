# Windows hardware suite

One command, from a normal PowerShell window. It does not ask for administrator and it does not show a User Account Control prompt.

```powershell
powershell -ExecutionPolicy Bypass -File tests\hardware\windows\run.ps1
```

The command builds this folder's parser with `cargo test --locked --release`, then runs the rows. A full run finishes within 5 minutes. Results are `suite-output\<timestamp>\result.json` and `result.txt`. `result.json` uses schema `softlock-hardware-suite/1`. `result.txt` includes the summary line `pass=N fail=N skip=N`.

`--admin` adds the HKLM policy read-back and the fail-closed lock check. Without it, those rows are skipped. Pass `--admin` from a prompt that is already elevated. `--ci` skips every physical step and is what the `windows-2022` and `windows-2025` runners use. This command never installs the desktop-proof or input-hook service.

Physical steps are one line each, and only when this machine can do them. The suite decides those skips from the detected config before it asks. Press Enter after the step, or type `skip`. If the step was done and the suite does not see it, that row fails. For the lid and power-button steps, the suite sets both actions to Do nothing (AC and DC) and puts the original actions back when the run ends. Ctrl+C stops the run, writes the result, and exits non-zero. A closed prompt with no answer is a skip.

It puts back every value it changed, the power actions, and the keep-awake request, including after Ctrl+C. A failed row locks Windows once at the end. The suite does not unlock Windows. `--ci` is ignored unless `GITHUB_ACTIONS=true`.
