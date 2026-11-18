# Windows hardware suite

One command, from PowerShell or from cmd.exe. It does not ask for administrator and it does not show a User Account Control prompt. A normal run is labeled `bench`. `--school` labels the school profile and does not change which rows run.

```powershell
powershell -ExecutionPolicy Bypass -File tests\hardware\windows\run.ps1
```

The command builds this folder's parser with `cargo test --locked --release`, then runs the rows. A full run finishes within 5 minutes. Results are `suite-output\<timestamp>\result.json` and `result.txt`. `result.json` uses schema `softlock-hardware-suite/1`. `result.txt` includes the summary line `pass=N fail=N skip=N`.

From a prompt that is not elevated, the first line says admin rows will skip. To run them, open an elevated prompt and pass `--admin`. `--admin` adds the HKLM policy read-back and the fail-closed lock check. `--ci` skips every physical step and is what the `windows-2022` and `windows-2025` runners use. `--ci` is ignored unless `GITHUB_ACTIONS=true`. This command never installs the desktop-proof or input-hook service.

Physical steps are one line each. The plan picks the step from the detected displays, lid, sleep state, sign-in method, power source, elevation, and session, and skips with a precise reason when that machine cannot do the step. Press Enter after the step, or type `skip`. If the only active display is an external monitor, the step is unplug, then plug it back in. A display step counts an active-path change, including zero paths when no display is attached, and a monitor add, remove, or mode-change event. If the step was done and the suite does not see it, that row fails. The lid is prompted only after the lid action is confirmed as Do nothing (AC and DC). The power button is not a prompted check: Windows does not deliver a language-independent press signal while that action is Do nothing. Ctrl+C stops the run, writes the result, and exits non-zero. A closed prompt with no answer is a skip.

It puts back every value it changed. A value is recorded only after the write succeeds. A value the suite created is deleted on restore. A failed row prints a line and then locks Windows once at the end. The suite does not unlock Windows.
