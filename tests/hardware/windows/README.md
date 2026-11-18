# Windows hardware suite

One command, from PowerShell or from cmd.exe. It does not ask for administrator and it does not show a User Account Control prompt. A normal run is labeled `bench`. `--school` labels the school profile and does not change which rows run.

```powershell
powershell -ExecutionPolicy Bypass -File tests\hardware\windows\run.ps1
```

The command builds this folder's parser with `cargo test --locked --release`, then runs the rows. A full run finishes within 5 minutes. Results are `suite-output\<timestamp>\result.json` and `result.txt`. `result.json` uses schema `softlock-hardware-suite/1`. `result.txt` includes the summary line `pass=N fail=N skip=N`.

From a prompt that is not elevated, the first line says admin rows will skip. To run them, open an elevated prompt and pass `--admin`. `--admin` adds the HKLM policy read-back and the fail-closed lock check. `--ci` skips every physical step and is what the `windows-2022` and `windows-2025` runners use. `--ci` is ignored unless `GITHUB_ACTIONS=true`. This command never installs the desktop-proof or input-hook service.

Physical steps are one line each. The plan picks the step from the detected displays, lid, sleep state, sign-in method, power source, elevation, and session, and skips with a precise reason when that machine cannot do the step. Press Enter after the step, or type `skip`. Attached displays are the active paths, split into internal and external. The detect row also reports how many displays are configured, including one that is not currently attached. If the only active display is an external monitor, the step is unplug, then plug it back in. When the internal and external counts are both known, a plug or unplug passes only when the external count changes, so opening the lid does not count as a connect. Otherwise a display step passes when the attached, internal, or external count changes. The result shows the before, during, and after counts. A device event with no count change does not pass. If the step was done and the suite does not see it, that row fails. Lid and power-button indexes are read for the active scheme and written back with that scheme's GUID, then checked again. The lid is prompted only after both the AC and DC indexes read back as Do nothing. The power button is not a prompted check: Windows does not deliver a language-independent press signal while that action is Do nothing. Ctrl+C stops the run, writes the result, and exits non-zero. A closed prompt with no answer is a skip.

It puts back every value it changed. The original value, and each AC and DC index, is written to `pending.json` before the change. Restore skips a value that still matches, deletes a value the suite created, and deletes that subkey once it is empty. A failed row locks Windows once at the end. The line before that lock names the failed row ids, the restore error, or the suite error. The suite does not unlock Windows.
