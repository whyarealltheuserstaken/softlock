# OS-lock proof

Proves the three ways to reach the Windows sign-in screen while the lockdown values are set. The probe is not the product service. Its service name is `OsLockProbe`.

Do not run this on the school workstation. The script and the executable refuse when `SystemFamily`, `SystemVersion`, or `SystemProductName` names that machine or a `30F2`/`30F3` machine type, and they refuse when any of those strings cannot be read. That happens before they change a policy or lock the session.

## Hosted runners

`.github/workflows/os-lock-proof.yml` runs the proof on `windows-2022` and `windows-2025`. The proof job needs the check job, and nothing needs the proof job, because a disconnect ends that runner's session. On a GitHub-hosted runner (`GITHUB_ACTIONS=true` and `RUNNER_ENVIRONMENT=github-hosted`) the session-0 service may reconnect the console between the two disconnect measurements. That reconnect is `WTSConnectSessionW` from `OsLockProbe` running as SYSTEM, and it is installed only for that hosted case. `--allow-local` does not enable it. The laptop script never writes `reconnect.on`. After the report is written the run confirms a lock and, only if that confirmation fails, disconnects. A `LockWorkStation` return value by itself is not a lock.

## One laptop run

From an elevated command prompt, in `%USERPROFILE%\softlock`:

```bat
powershell -ExecutionPolicy Bypass -File spikes\os-lock\run.ps1
```

The elevated account has to be the console session's account. The machine needs the Rust toolchain already used for the other Windows checks. The script copies `os-lock-proof.exe` and `restore.ps1` into `C:\ProgramData\os-lock-probe`, which it creates with access only for SYSTEM and Administrators. It records the blocked lock, the lock after `DisableLockWorkstation` is cleared, and the background process. It does not time the two disconnects, because reconnecting afterwards is CI only. It then restores every value it changed and runs the confirmed lock chain. Sign in with the Windows Hello fingerprint. The report is `suite-output\<timestamp>\result.json` and `result.txt`.

If the prompt is not elevated, the session is remote, or there is no console session, the script skips and names that reason. It does not change policies in those cases. If the console user cannot be read, or is not the elevated user, the script stops without changing policies.

## If the script is interrupted

Sign in with the fingerprint if the sign-in screen is up. The next sign-in runs two restores. RunOnce `os-lock-restore` puts the current user's values back. The SYSTEM task `os-lock-restore` is what puts the HKLM values back; RunOnce does not. The task deletes itself only after those HKLM values match the journal. To restore immediately from an elevated prompt:

```bat
powershell -ExecutionPolicy Bypass -File C:\ProgramData\os-lock-probe\restore.ps1 -Journal C:\ProgramData\os-lock-probe\pending.json
schtasks /Delete /TN os-lock-restore /F
sc stop OsLockProbe
sc delete OsLockProbe
```

The repo copy of `restore.ps1` accepts that same journal path and no other. It is safe to run more than once. When the HKLM values match, it removes the RunOnce value, the task, and `OsLockProbe`. If the journal is missing it leaves the task in place.
