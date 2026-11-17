# m08: the lock-path chooser is tested; the lock screen is not

Measured from `b1af758` (m02 on main). The kit is `spikes/macos-lock-kit`. The chooser is `choose` in `crates/softlock-macos/src/lock_choice.rs`.

m02's finding says that spike did not lock the session, so it does not say the runner comes back after a lock. This ticket therefore does not lock `macos-latest`. The verify job runs the four chooser rows and does not call `SACLockScreenImmediate`. A separate read-only job resolves the symbol with `dlopen` and `dlsym`, reads FileVault status, and reads the screen-saver password keys. It does not call the symbol and it does not start the screen saver.

## What the verify job proves

These tests run under `cargo test --workspace` on `ubuntu-latest`, `windows-latest`, and `macos-latest`. They pass facts in. They do not call the live symbol.

| Row | Test | Result |
|---|---|---|
| Symbol missing, and the documented path can require an immediate password | `symbol_missing_requests_the_documented_path` | Documented path is requested |
| Symbol present, call failed, and the documented path can require an immediate password | `a_failed_call_requests_the_documented_path` | Documented path is requested |
| Call succeeded | `a_successful_private_call_does_not_request_the_documented_path` | Documented path is not requested |
| Documented path unavailable, or password not required | `documented_path_unavailable_or_password_not_required_refuses_to_arm` | Refuse to arm. Not Unlocked. The message tells the owner to turn on "Require password after screen saver begins or display is turned off" and set the delay to zero |

`fail_closed_after_error` is the exercise rule for a later error: lock by the private call if that call has not already failed, otherwise the documented path if it can require an immediate password, otherwise `never-unlocked`. None of those ends are Unlocked. `permit` in `spikes/macos-lock-kit/src/kit_gate.rs` refuses an exercise on a GitHub runner and refuses every mode as root. A read-only run stays allowed on the runner. The product crate keeps `choose`, `fail_closed_after_error`, and the parsers. It does not keep the kit's run gate.

The product crate does not spawn `fdesetup` and does not call `dlopen`. Parsing the status text is separate from running the tool. A non-zero exit, a permission error, or On and Off disagreeing becomes Unknown, not Off.

## What the read-only runner job proves

The job is the `macOS lock kit` workflow. It prints `mode=read-only`, `private-call=not-called`, `documented-path=not-started`, `state=never-unlocked`, `runner-lock-allowed=false`, and `filevault-pre-boot-observed=false`.

That FileVault reading is the runner's disk, as the console user. It is not Henry's Mac. The job does not reboot, so a FileVault pre-boot screen was not observed. `uname -m` has to be `arm64`. A missing symbol on that OS is recorded and is not a crash. Forcing the symbol to be missing is not part of the job. The first chooser row is the proof of that branch.

The read-only job on `8924571e2c0d8315a8a4f08c4915d97b2239d60e` is that commit's own job, [run 44020650978](https://github.com/whyarealltheuserstaken/softlock/actions/runs/44020650978), job `macos lock kit read-only`. This commit changes only the finding, so the kit code here is identical to that parent and the measurement below is the parent's own run. `sw_vers` printed ProductVersion 26.7.2 and BuildVersion 25G332, and `id -u` printed `uid=501`. The kit printed:

```
mode=read-only
result=pass
state=never-unlocked
private-symbol=present
private-symbol-detail=resolved
private-call=not-called
documented-path=not-started
documented-engine=/System/Library/CoreServices/ScreenSaverEngine.app/Contents/MacOS/ScreenSaverEngine
screen-saver-engine-present=true
immediate-lock-required=false
ask-for-lock=missing
ask-for-lock-delay=missing
screen-lock-status=delayed
screen-lock-detail=2026-11-17 04:57:46.889 sysadminctl[6764:25144] screenLock delay is 300 seconds
screen-lock-disagreement=false
arm=not-applied
chooser-if-symbol-missing=refuse
chooser-if-call-failed=refuse
chooser-if-call-succeeded=private
fdesetup-status=FileVault is Off.
fdesetup-status-exit=0
fdesetup-isactive=false
fdesetup-isactive-exit=1
filevault=off
filevault-pre-boot-observed=false
settings-changed=false
filevault-changed=false
serial-collected=false
linked-login-framework=false
otool-checked=true
uid=501
uid-is-root=false
runner-lock-allowed=false
product-name=macOS
product-version=26.7.2
build-version=25G332
uname-m=arm64
hw-model=VirtualMac2,1
session-lock-check=available
session-lock-key-present=false
session-lock-key-false=false
lock-check=not-checked
helper-end=not-armed
lock-screen-appeared=not-asked
external-display-covered=not-asked
input-reached-desktop-without-login=not-asked
fail-closed=not-needed
```

There is no `ms-until-locked` line. The read-only run does not start a lock, so that field stays empty. The result keys that used to contain the word password are now `immediate-lock-required`, `ask-for-lock`, and `ask-for-lock-delay`, so the log shows `immediate-lock-required=false` with `ask-for-lock=missing` and `ask-for-lock-delay=missing`. The two chooser rows that are not a successful call both printed `refuse`, and `screen-lock-status=delayed` with `screen-lock-disagreement=false`. `session-lock-check=available` means `CGSessionCopyCurrentDictionary` returned a dictionary. On this unlocked runner the `CGSSessionScreenIsLocked` key was absent, so `session-lock-key-present=false` and `session-lock-key-false=false`. `fdesetup isactive` printed `false` and exited 1, so that exit is not itself a successful Off. `fdesetup status` exited 0 with `FileVault is Off.`, and the combined reading is `filevault=off`. This is the runner's disk, not Henry's Mac. A missing symbol was not forced.

This commit changes only the finding. The kit code is identical to parent `8924571e2c0d8315a8a4f08c4915d97b2239d60e`, and the log above is that parent's own run.

## What only hardware proves

Henry's Intel Mac is the only proof of the rest. The machine is the one in [Henry's sheet](../tickets/macos/henry-sheet.md): MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports), 2 GHz quad-core Intel Core i5, Intel Iris Plus Graphics, 16 GB LPDDR4X, macOS Tahoe 26.7.3, laptop, Touch ID, T2. External displays are still unknown. macOS 11 through 15 stay untested.

Only that Mac can show:

- The private call reached the real lock screen. `ms-until-locked` is how long until the session reported locked. `private-call-returned-ms` is only how long the function took to return.
- The documented path reached a lock screen, and whether one click or one key reached the desktop without the account password.
- Whether the lock covered both screens when an external display was attached. The kit records that answer as `external-display-covered`.
- His password-after-screensaver keys and `sysadminctl -screenLock status`. A disagreement, or both readings missing, refuses to arm. The kit does not write the setting and does not use sudo.
- `fdesetup status` and `fdesetup isactive` as his console user.
- Whether a FileVault-on pre-boot screen appeared. The T2 chip means that screen is the Secure Enclave path. If FileVault is off, it stays unproven. The kit does not reboot and does not change FileVault.

Instructions: [spikes/macos-lock-kit/README.md](../../spikes/macos-lock-kit/README.md). Run read-only first. Exercise the private path, unlock with the account password, then exercise the documented path. The kit never unlocks. If the symbol is present, a missing-symbol row cannot be forced; write that here and point at `symbol_missing_requests_the_documented_path`. Commit the kit's `result.json`. It does not contain an account name, a home directory, an Apple ID, a hardware serial, other apps, or other devices. Do not paste About This Mac or `system_profiler` beside it.

## Sheet

| Row | Result |
|---|---|
| Private call: lock screen and time, or the error, including a missing symbol | Unproven. Henry's Mac. |
| Documented path: lock screen, time, and whether a click or a key reached the desktop without the account password | Unproven. Henry's Mac. |
| Password-after-screensaver setting and delay | Unproven. Henry's Mac. The kit reads the defaults keys and `sysadminctl -screenLock status`. It does not write them. |
| External display covered by the lock | Unproven. Henry's Mac. `external-display-covered` is `yes`, `no`, or `none`. |
| Missing symbol cannot be forced | If his OS has the symbol, say so. The CI row is `symbol_missing_requests_the_documented_path`. |
| `fdesetup status` and `fdesetup isactive` as the console user | Unproven. Henry's Mac. Paste the kit's lines, or the permission error. |
| FileVault pre-boot | Not observed. Unproven until he reboots a FileVault-on disk. The kit writes `filevault-pre-boot-observed=false`. |

## Reversibility

The kit does not write screen-saver preferences, does not enable or disable FileVault, does not install a launchd job, and does not link `login.framework`. The only changes an exercise run makes are the lock screen or the screen saver, and those end when the account password is entered. A read-only run makes neither change. An error during an exercise ends in a lock path when one is still safe, and otherwise in `never-unlocked`. It does not end Unlocked.
