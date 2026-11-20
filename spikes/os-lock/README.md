# OS-lock proof

Proves the three ways to reach the Windows sign-in screen while the lockdown values are set. The probe is not the product service. Its service name is `OsLockProbe`.

Do not run this on the school workstation. The script and the executable refuse when the BIOS product name is that machine, before they change a policy or lock the session.

## Hosted runners

`.github/workflows/os-lock-proof.yml` runs the proof on `windows-2022` and `windows-2025`. The proof job needs the check job, and nothing needs the proof job, because a disconnect ends that runner's session. `--ci` reconnects between the two disconnect measurements so the job can record both, then locks the desktop once the report is written. It does not call an unlock API.

## One laptop run

From an elevated command prompt, in `C:\Users\henry\softlock`:

```bat
powershell -ExecutionPolicy Bypass -File spikes\os-lock\run.ps1
```

The machine needs the Rust toolchain already used for the other Windows checks. The script builds `spikes\os-lock\target\release\os-lock-proof.exe`, records the three timings, and checks that a background process kept writing. It then restores every value it changed. The last action disconnects the console session and does not reconnect and does not unlock. Sign in with the Windows Hello fingerprint. The report is `suite-output\<timestamp>\result.json` and `result.txt`.

If the prompt is not elevated, the session is remote, or there is no console session, the script skips and names that reason. It does not change policies in those cases.

## If the script is interrupted

Sign in with the fingerprint if the sign-in screen is up. The next sign-in runs two restores: the current user's RunOnce value `os-lock-restore`, and the SYSTEM task `os-lock-restore`. Either one is enough to put the values back. To restore immediately from an elevated prompt:

```bat
powershell -ExecutionPolicy Bypass -File spikes\os-lock\restore.ps1
schtasks /Delete /TN os-lock-restore /F
sc stop OsLockProbe
sc delete OsLockProbe
```

`restore.ps1` is safe to run more than once. When nothing is left to put back it removes the RunOnce value, the task, and `OsLockProbe`.
