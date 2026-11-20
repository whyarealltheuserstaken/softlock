# 04: Fail-closed reaches the Windows sign-in screen while the lockdown values are set

Measured on 20 November 2026 UTC by the workspace-excluded probe `spikes/os-lock`.

The lines below are from [OS lock proof run 44686484658](https://github.com/whyarealltheuserstaken/softlock/actions/runs/44686484658), commit `1c33a918fe8bb67a752fb24bd17b2afc5f2f2244`. Both jobs passed 8 rows, restored every value, and finished `state=os-locked`.

- [windows-2022 job](https://github.com/whyarealltheuserstaken/softlock/actions/runs/44686484658/job/136122258806)
- [windows-2025 job](https://github.com/whyarealltheuserstaken/softlock/actions/runs/44686484658/job/136122258797)

## Decision

Primary is `LockWorkStation`, called from the signed-in session after `DisableLockWorkstation` is cleared. Fall-back is `WTSDisconnectSession`, which does not depend on that value. The session-0 service calls it on the console session. The Lock Host calls it on its own session.

`verdict::pick` printed `primary=LockWorkStation fallback=WTSDisconnectSession` on both images.

Dependent tickets:

- [15](https://github.com/whyarealltheuserstaken/softlock/issues/15) implements this pair: the real OS lock, Lock for real, and the watchdog. Its fail-closed job uses the same order and ends the runner session in its own last job.
- [17](https://github.com/whyarealltheuserstaken/softlock/issues/17) applies the lockdown values. Fail-closed and hard-lock paths clear `DisableLockWorkstation` before `LockWorkStation`. They do not clear it for `WTSDisconnectSession`.

## What both images showed

The job is `runneradmin` in console session 2. The probe sets the SPEC.md section 8 lockdown values, including `DisableLockWorkstation=1`, and journals each original value before the write.

`LockWorkStation` while that value is still 1 returns success (`win32=0`) and does not lock. No `WTS_SESSION_LOCK` arrives during the one-second wait, and `session_locked` is false. That row's elapsed time is the wait.

Clearing only `DisableLockWorkstation` (the other lockdown values stay set) makes the same call lock the session. Confirmation is `WTS_SESSION_LOCK`, and `session_locked` is true.

`WTSDisconnectSession` from `OsLockProbe` in session 0, aimed at the console session, and `WTSDisconnectSession` from the user-session process on its own session, both report `WTS_CONSOLE_DISCONNECT`. The service's `session_locked` flag was unreadable after the disconnect (`null`). The notification is the confirmation. A heartbeat process kept writing through the lock and both disconnects.

The hosted job reconnects the console between the two disconnects so both can be recorded. That reconnect is `WTSConnectSessionW` from `OsLockProbe` running as SYSTEM. The service is installed with `--ci-reconnect` only when `GITHUB_ACTIONS=true` and `RUNNER_ENVIRONMENT=github-hosted`. `--allow-local` does not turn it on, and the laptop script does not write `reconnect.on`. The service binary and its control files live in `C:\ProgramData\os-lock-probe`, which the elevated script creates for SYSTEM and Administrators only. After the saved values are restored, the run confirms a lock. A successful `LockWorkStation` return is not that confirmation.

## Rule

`LockWorkStation`'s return value cannot be trusted. A lock counts only when a `WTS_SESSION_LOCK` notification arrives, or the session state says locked, within one second. If that does not happen, fall back to `WTSDisconnectSession` and confirm it the same way (a console-disconnect notification, or the session state). If both fail, the product fails closed: the result is not a lock, the run exits non-zero, and it never ends Unlocked.

| Row | windows-2022 | windows-2025 |
| --- | --- | --- |
| `LockWorkStation` while the policy is set | api success, notified false, session unlocked, 1,003,947 µs | api success, notified false, session unlocked, 1,000,026 µs |
| `LockWorkStation` after clearing it | lock, session locked, 40,290 µs | lock, session locked, 140,014 µs |
| Service disconnect of the console session | console-disconnect, 67,102 µs | console-disconnect, 114,320 µs |
| Own-session disconnect | console-disconnect, 74,075 µs | console-disconnect, 104,032 µs |
| Background heartbeat | alive, 18 lines | alive, 20 lines |

The three confirming paths are each under one second. The blocked row is the full one-second wait.

## Laptop

`spikes/os-lock/run.ps1` is the client Windows run. It was not executed here. It refuses a school workstation (the model name or a `30F2`/`30F3` machine type in `SystemFamily`, `SystemVersion`, or `SystemProductName`, and it refuses when any of those cannot be read), a prompt that is not elevated, a remote session, a machine with no console session, and an elevated user who is not the console session user. It journals the eight DWORD lockdown values, absent or present, in `C:\ProgramData\os-lock-probe\pending.json` before changing them, and it refuses any other kind. RunOnce `os-lock-restore` restores the HKCU values. The SYSTEM task of the same name is what restores HKLM, and it deletes itself only after those three values match. The laptop does not time the two disconnects, because the SYSTEM reconnect that lets CI record both is CI only. It ends by confirming a lock, then disconnecting only if that confirmation fails. Recovery is in `spikes/os-lock/README.md`.

The timings in the table are from the run cited above, before the reconnect, journal, and confirmation changes. The next green proof run replaces them.
