# One laptop check

Run this only after a reviewer tells you to. Do not run it as part of reading this file.

You need one Windows laptop and one PowerShell window opened as Administrator. From the repository root, the check itself takes about one minute once the program is already built. The first build takes several minutes.

You will see the sign-in screen twice. Then Windows locks. A window titled "Fail-closed check - still running" should keep counting the whole time. That window is the background app. Sign back in when the lock screen appears. Your other programs keep running.

## Run

```
cargo run --release --manifest-path spikes/fail-closed-proof/Cargo.toml -- --allow-local-service --output-dir proof-output
```

When it finishes, open `proof-output\report.txt`. `result=pass` means all three lock paths were confirmed in under one second and the background window kept running.

## Put the policies back

The program writes each old value to `proof-output\journal.json` before it changes anything. It puts those values back when it finishes, when you press Ctrl+C, and when it crashes. A helper process also puts them back if the program is killed.

If you want to put them back yourself, this is safe to run twice:

```
cargo run --release --manifest-path spikes/fail-closed-proof/Cargo.toml -- --revert --output-dir proof-output
```

Then remove the throwaway service:

```
sc.exe stop RunnerFailClosedProbe
sc.exe delete RunnerFailClosedProbe
```

## Values that must be restored

Cleanup puts back the exact value that was there before the run. If a value was missing, cleanup deletes it.

For your user, under `HKEY_USERS\<your SID>`:

- `Software\Microsoft\Windows\CurrentVersion\Policies\System`
  - `DisableLockWorkstation`
  - `DisableTaskMgr`
  - `DisableChangePassword`
- `Software\Microsoft\Windows\CurrentVersion\Policies\Explorer`
  - `NoLogoff`
  - `NoClose`

For the machine, under `HKEY_LOCAL_MACHINE`:

- `SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System`
  - `HideFastUserSwitching`
  - `ConsentPromptBehaviorAdmin`
  - `ConsentPromptBehaviorUser`

After a successful restore the journal file is gone. Run the revert command again if it is still there.

## If the screen stays on the sign-in page

Sign in. `proof-output\report.txt` is still on disk. Then run the revert command and the two `sc.exe` commands above.
