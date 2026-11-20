# 04: Fail-closed reaches the Windows sign-in screen while the lockdown values are set

The measurement is the proof job in `.github/workflows/os-lock-proof.yml` (`spikes/os-lock`). It applies `DisableLockWorkstation` and the other lockdown values from SPEC.md section 8, times the three calls in issue 4, and confirms each through a session notification (`WTS_SESSION_LOCK` or `WTS_CONSOLE_DISCONNECT`) in under one second. `verdict::pick` names the primary and the fall-back from those three results.

Dependent tickets:

- [15](https://github.com/whyarealltheuserstaken/softlock/issues/15) implements the primitive this spike picks: the real OS lock, Lock for real, and the watchdog. Its fail-closed job has to use this pair and end the runner session in its own last job.
- [17](https://github.com/whyarealltheuserstaken/softlock/issues/17) applies the lockdown values. When the primary is `LockWorkStation`, the fail-closed and hard-lock paths clear `DisableLockWorkstation` before that call. `WTSDisconnectSession` does not depend on that value.

## What each path is

1. **Blocked.** `LockWorkStation` from the signed-in session while `DisableLockWorkstation` is 1. The call can return success. The proof passes when no lock notification arrives and the session is not reported locked. The wait is the one-second budget, so this row's elapsed time is about one second.
2. **Primary candidate.** Clear `DisableLockWorkstation`, then `LockWorkStation` from that same user-session process. Confirmation is the lock notification, in under one second. The other lockdown values stay set.
3. **Service fall-back.** `WTSDisconnectSession` on the console session from `OsLockProbe`, a session-0 service. This is the call the service makes when the Lock Host is gone.
4. **Lock Host fall-back.** `WTSDisconnectSession` on the user-session process's own session. This is the call the Lock Host makes when the service is gone. It does not need `DisableLockWorkstation` cleared.

The hosted job reconnects the console session after each disconnect so both disconnects can be timed and the job can still report. It does not call an unlock API. After the report is written it locks the desktop with `LockWorkStation`, with the saved policies already restored. The laptop script does not reconnect after the last call: it disconnects and leaves the sign-in screen up.

## Laptop

`spikes/os-lock/run.ps1` is the manual run. It was not executed here. It refuses a school workstation, a prompt that is not elevated, a remote session, and a machine with no console session. It backs up every value before changing it, writes `%LOCALAPPDATA%\os-lock-probe\pending.json`, registers RunOnce `os-lock-restore`, and registers a one-shot SYSTEM logon task of the same name so a crash still restores HKLM. The run ends at the sign-in screen. Recovery is in `spikes/os-lock/README.md`.

## Measurements

Filled from the proof job log after that job is green. The lines to copy are `timing` and `pick`.
