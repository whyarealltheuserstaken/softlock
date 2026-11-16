# 04: Fail-closed lock while the lockdown policies are set

The question is whether a real Windows lock still happens within one second while the lockdown policies are applied, and whether the user's programs keep running.

The probe is `spikes/fail-closed-proof`. It is not part of the product workspace. It journals every policy value, flushes that journal, then writes the value. Revert puts the previous value back, or deletes the value when it was absent. The same revert runs on a normal finish, on Ctrl+C, on panic, from a helper process if the probe is killed, and from the workflow step that runs even when the proof is cancelled.

## What the proof times

All eight lockdown values are applied first. `DisableLockWorkstation` stays set for the disconnect paths. It is cleared only for the last path.

1. `LockWorkStation` while `DisableLockWorkstation` is set. The call only reports that Windows accepted the request. The session must not send `WTS_SESSION_LOCK`.
2. `WTSDisconnectSession` on the console session from the session-0 service. Confirmation is `WTS_CONSOLE_DISCONNECT`.
3. `WTSDisconnectSession` on the probe's own session, from that session. This is the fall-back for when the service is gone. Confirmation is `WTS_CONSOLE_DISCONNECT`.
4. Clear `DisableLockWorkstation`, then `LockWorkStation` immediately. Confirmation is `WTS_SESSION_LOCK`.

Each confirmation is timed from just before the call to the session notification. The pass line is strictly under one second. A background process writes a heartbeat every 50 ms. The longest pause between those writes has to stay under two seconds, which is the check that programs keep running.

## Choice for tickets 15 and 17

Primary OS-lock primitive for ticket 15: clear `DisableLockWorkstation` for the session user, then call `LockWorkStation` from a process in that session. Confirm with `WTS_SESSION_LOCK`.

Fall-back when that process is gone: the service calls `WTSDisconnectSession` on the console session from session 0. Confirm with `WTS_CONSOLE_DISCONNECT`. Programs keep running.

Fall-back when the service is killed: the in-session process calls `WTSDisconnectSession` on its own session. That call does not need `DisableLockWorkstation` cleared. The proof runs it while the value is still set.

Ticket 17 must clear `DisableLockWorkstation` before `LockWorkStation`. The proof's first step is there to show `LockWorkStation` fails while the value is set, and the last step is there to show it succeeds right after the value is cleared. The other lockdown values stay applied the whole time. The report word for a lock that did not happen is `blocked`. That line also records the session lock flag from `WTSSessionInfoEx`. The own-session disconnect has to be seen by the in-session process as well as by the service.

This choice is what both runner images measured. Henry's laptop run is still required. Server runners can differ from that laptop, so issue 4 stays open.

## Measured timings

Copied from `report.txt` in [Fail-closed proof run 43899750076](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43899750076), head `7da37ac8b8dc83b8153460b2d1881443e2c0c85c`. Both images printed `result=pass`. The session was 2 and the service session was 0. While `DisableLockWorkstation` was set, the report line was `lock-while-policy=blocked`, `win32=none`, `notified=false`, and `session-lock-state=unlocked`. After that value was cleared, `WTS_SESSION_LOCK` arrived. The own-session path also recorded `local-notification=WTS_CONSOLE_DISCONNECT`. The three path times below are the `elapsed-us` fields: service disconnect, own-session disconnect, then user lock. All three are under one second, and the background heartbeat kept writing.

- [windows-2022 job](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43899750076/job/133412840880)
- [windows-2025 job](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43899750076/job/133412841384)

### windows-2022

```
result=pass
runner-image=windows-2022
session=2
service-session=0
policies-applied=DisableTaskMgr,DisableChangePassword,DisableLockWorkstation,NoLogoff,NoClose,HideFastUserSwitching,ConsentPromptBehaviorAdmin,ConsentPromptBehaviorUser
lock-while-policy=blocked win32=none notified=false session-lock-state=unlocked
path-service-disconnect=ok notification=WTS_CONSOLE_DISCONNECT elapsed-us=55376 within-1s=true api-ok=true heartbeat-gap-ms=51
path-session-disconnect=ok notification=WTS_CONSOLE_DISCONNECT elapsed-us=61313 within-1s=true api-ok=true heartbeat-gap-ms=54 local-notification=WTS_CONSOLE_DISCONNECT
cleared-disable-lock-workstation=true
path-user-lock=ok notification=WTS_SESSION_LOCK elapsed-us=50768 within-1s=true api-ok=true heartbeat-gap-ms=50
background-kept-running=true
policies-restored=true
```

### windows-2025

```
result=pass
runner-image=windows-2025
session=2
service-session=0
policies-applied=DisableTaskMgr,DisableChangePassword,DisableLockWorkstation,NoLogoff,NoClose,HideFastUserSwitching,ConsentPromptBehaviorAdmin,ConsentPromptBehaviorUser
lock-while-policy=blocked win32=none notified=false session-lock-state=unlocked
path-service-disconnect=ok notification=WTS_CONSOLE_DISCONNECT elapsed-us=78854 within-1s=true api-ok=true heartbeat-gap-ms=68
path-session-disconnect=ok notification=WTS_CONSOLE_DISCONNECT elapsed-us=46203 within-1s=true api-ok=true heartbeat-gap-ms=59 local-notification=WTS_CONSOLE_DISCONNECT
cleared-disable-lock-workstation=true
path-user-lock=ok notification=WTS_SESSION_LOCK elapsed-us=114536 within-1s=true api-ok=true heartbeat-gap-ms=91
background-kept-running=true
policies-restored=true
```

### Earlier run

[Fail-closed proof run 43891281142](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43891281142), commit `0ab3d21b1b76b4b1c0befcd7a83a76b401ea9a72`, is a second data point. Both images printed `result=pass` there too. That report used `lock-while-policy=fail` and did not print `session-lock-state` or `local-notification`.

- [windows-2022 job](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43891281142/job/133382829116)
- [windows-2025 job](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43891281142/job/133382829781)

### windows-2022, earlier run

```
result=pass
runner-image=windows-2022
session=2
service-session=0
policies-applied=DisableTaskMgr,DisableChangePassword,DisableLockWorkstation,NoLogoff,NoClose,HideFastUserSwitching,ConsentPromptBehaviorAdmin,ConsentPromptBehaviorUser
lock-while-policy=fail win32=none notified=false
path-service-disconnect=ok notification=WTS_CONSOLE_DISCONNECT elapsed-us=63160 within-1s=true api-ok=true heartbeat-gap-ms=52
path-session-disconnect=ok notification=WTS_CONSOLE_DISCONNECT elapsed-us=45724 within-1s=true api-ok=true heartbeat-gap-ms=58
cleared-disable-lock-workstation=true
path-user-lock=ok notification=WTS_SESSION_LOCK elapsed-us=66426 within-1s=true api-ok=true heartbeat-gap-ms=54
background-kept-running=true
policies-restored=true
```

### windows-2025, earlier run

```
result=pass
runner-image=windows-2025
session=2
service-session=0
policies-applied=DisableTaskMgr,DisableChangePassword,DisableLockWorkstation,NoLogoff,NoClose,HideFastUserSwitching,ConsentPromptBehaviorAdmin,ConsentPromptBehaviorUser
lock-while-policy=fail win32=none notified=false
path-service-disconnect=ok notification=WTS_CONSOLE_DISCONNECT elapsed-us=84335 within-1s=true api-ok=true heartbeat-gap-ms=67
path-session-disconnect=ok notification=WTS_CONSOLE_DISCONNECT elapsed-us=84028 within-1s=true api-ok=true heartbeat-gap-ms=60
cleared-disable-lock-workstation=true
path-user-lock=ok notification=WTS_SESSION_LOCK elapsed-us=83093 within-1s=true api-ok=true heartbeat-gap-ms=51
background-kept-running=true
policies-restored=true
```
