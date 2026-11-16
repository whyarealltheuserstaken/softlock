# One laptop check

Run this only after a reviewer tells you to. Do not run it as part of reading this file.

While the check runs, Task Manager, sign-out, switch user, and change password are turned off. The Start menu shut-down options are turned off too. UAC asks for a password instead of a yes button. Save your work and close other apps before you start. The check takes about one minute once the program is already built. The first build takes several minutes.

If something is left behind, run the revert command in "Put the policies back". A reboot does not put the values back by itself. If you cannot open a prompt, hold the power button to reboot, sign in, and then run the revert command.

A helper process watches the program. If the program is killed, the helper puts every value back and removes the temporary service. The helper waits up to 15 minutes. If the program is still going after 15 minutes, the helper puts the values back anyway. After that, run the revert command yourself if `proof-output\journal.json` is still there.

## What you need first

Install these before the reviewer asks you to run the check:

- Git, from https://git-scm.com/download/win
- Rust 1.100.0 with rustup, from https://rustup.rs . After the installer finishes, open a new terminal and run `rustup toolchain install 1.100.0` and then `rustup default 1.100.0`.
- The Visual Studio C++ Build Tools, with the "Desktop development with C++" workload. The Rust Windows build needs that linker.

## Get this exact copy

Do not use `main`. Check out this program commit. A commit cannot contain its own id, so copy this command from the latest copy of this file:

```
git clone https://github.com/whyarealltheuserstaken/softlock.git
cd softlock
git checkout 81d8366560b3532d23e4a33a2b9e660096a042a4
```

`git status` should say `HEAD detached at` that commit.

## Run it from your own administrator account

Open PowerShell with "Run as administrator". Use your own account, the one you sign in to this laptop with, including Windows Hello. Do not switch to a different administrator. The policy values for Task Manager, change password, and the lock are written into that elevated account's user hive, `HKEY_USERS\<that account's SID>`. They are not written to some other user's hive. Three more values are written for the whole machine.

From the repository root:

```
cargo run --release --manifest-path spikes/fail-closed-proof/Cargo.toml -- --allow-local-service --output-dir proof-output
```

You will see the sign-in screen twice. Then Windows locks. Sign back in with Windows Hello (your face, fingerprint, or PIN) or your password. A window titled "Fail-closed check - still running" should keep counting the whole time. That window is the background app. Your other programs keep running.

When it finishes, `proof-output\report.txt` starts with `result=pass` when all three lock paths were confirmed in under one second and the background window kept running.

## Send these files back

Send all three. The paths are under the repository folder:

- `proof-output\report.txt`
- `proof-output\proof.log`
- `proof-output\service.log`

## Put the policies back

The program writes each old value to `proof-output\journal.json` before it changes anything. It puts those values back when it finishes, when you press Ctrl+C, and when it crashes. The helper process also puts them back if the program is killed. The helper gives up after 15 minutes, which is longer than this check.

If you want to put them back yourself, this is safe to run twice:

```
cargo run --release --manifest-path spikes/fail-closed-proof/Cargo.toml -- --revert --output-dir proof-output
```

Then remove the temporary service. Its name on this laptop is `FailClosedProbe`.

```
sc.exe stop FailClosedProbe
sc.exe delete FailClosedProbe
```

If the screen is still locked or the values are still changed after that, reboot, sign in, and run the revert command again.

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

Sign in with Windows Hello or your password. The three files in `proof-output` are still on disk. Then run the revert command and the two `sc.exe` commands above. Reboot if the policies are still in the way, and run the revert command after you sign in.
