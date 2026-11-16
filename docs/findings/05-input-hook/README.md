# 05: SYSTEM in the interactive session sees elevated input

Measured on 16 November 2026 UTC by the workspace-excluded probe `spikes/input-hook`.

The `result=measured` lines below are from [Input hook proof run 43895906522](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43895906522), commit `3c42fba03facfff2c946a4ba7cd03720f655444e`. That is the run that printed them. The earlier run [43894384199](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43894384199) printed `result=fail` for the same operating-system outcome, because the harness still treated "the hook was not removed" as a job failure.

- [windows-2022 job](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43895906522/job/133399283309)
- [windows-2025 job](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43895906522/job/133399283803)

## Decision

Confirm the SYSTEM-in-session Lock Host for low-level keyboard input. On both runner images a `WH_KEYBOARD_LL` hook running as `SYSTEM` on the interactive desktop saw every `SendInput` key delivered to an elevated window, and a medium-integrity hook saw none of them.

`fallback-needed=false` is what these Server runner images showed. The console user's token is already high integrity (`elevation-type=1`, integrity 12288) and there is no linked token (`0x80070520`), so the token is unsplit. That does not say what a client Windows logon with UAC's split token does. Ticket [06](https://github.com/whyarealltheuserstaken/softlock/issues/6) has to re-check the elevated linked token there.

Silent removal was not shown. Windows Server 2022 never timed the callback out, so its removal result is inconclusive. Windows Server 2025 did time `SendInput` out, eleven times, and the same SYSTEM hook was still installed afterwards. No non-SYSTEM hook was put on the slow path, so that cannot be separated from "this setup never triggers removal".

This is a keyboard `SendInput` proof (F13 for visibility and load, F24 for the marker). The hook callback does no I/O. Mouse input is the same integrity rule and is left to the Lock Host work.

Dependents:

- [06](https://github.com/whyarealltheuserstaken/softlock/issues/6) still has to try physical input on real hardware. This spike only injected keys. It also has the two open questions below: a non-SYSTEM slow-hook control, and a client Windows logon whose token is split by UAC.
- [12](https://github.com/whyarealltheuserstaken/softlock/issues/12) can install the Lock Host hook as SYSTEM in the session. Liveness has to be a self-check: a periodic no-op marker with a deadline. It must not assume that Windows removes a hook whose callback is slow. The marker check is how the Lock Host notices that its own hook has gone; these images did not make it go.
- [13](https://github.com/whyarealltheuserstaken/softlock/issues/13) is how the service launches that SYSTEM-in-session process. The launch used here is the one ticket 02 left open: duplicate the service token, set its session to the console session, and `CreateProcessAsUser` onto `winsta0\default`.

## What both images showed

The job process is `runneradmin` in console session 2. `LowLevelHooksTimeout` is absent under `HKCU\Control Panel\Desktop`, so the documented default is 300 ms (`supervisor-timeout-raw=absent:2`). The service `InputHookProbe` runs as `SYSTEM` in session 0, enables `SeTcbPrivilege`, `SeAssignPrimaryTokenPrivilege`, and `SeIncreaseQuotaPrivilege`, and launches three processes onto `winsta0\default`:

| Role | Integrity | Token |
| --- | --- | --- |
| medium | 8192 | integrity label lowered from the high user token |
| elevated | 12288 | the console user token (`elevation-type=1`) |
| system | 16384 | the service token moved into session 2 |

The system process's own report is user `SYSTEM`, session 2, station `WinSta0`, desktop `Default`. There is no linked token:

```
linked-token=TokenLinkedToken: A specified logon session does not exist. It may already have been terminated. (0x80070520)
```

A medium-integrity process cannot take the foreground from the elevated probe window. On windows-2022 the medium process reported `foreground is pid 2324 hwnd b011e, wanted 40088` (pid 2324 was the elevated probe). The supervisor, which is already high integrity, then called `SetForegroundWindow`. After that, both images recorded the same visibility and load lines:

```
control system=20 medium=20 elevated-hook=20 window=20
elevated system=20 injected=20 medium=0 elevated-hook=20 window=20
load sent=40000 seen=40000 alive-after=true
injected-flag=true
passed-through=true
fallback-needed=false
design=confirm
```

The control line is the medium window in front: the system hook, the medium hook, the elevated hook, and the medium window each saw 20 F13 key-downs. The elevated line is the elevated window in front: the system hook saw 20, all of them with `LLKHF_INJECTED`, the medium hook saw 0, the high hook saw 20, and the elevated window received 20. The load is 40,000 injected F13 events (20,000 down/up pairs). The system hook counted all 40,000 and then saw a swallowed F24 marker, so it was still installed.

## Removal

The system hook then sleeps in the callback, longer than the 300 ms default, while the supervisor injects F24 markers from outside that process. A same-process `SendInput` does not trip the timeout; an earlier run blocked for the whole sleep (1,814 ms on windows-2022 with an 1,800 ms sleep). Only the SYSTEM hook was armed. The medium and elevated hooks were not, so there is no control that shows whether a non-SYSTEM hook on the same desktop would be removed.

### windows-2022 is inconclusive

Windows Server 2022 Datacenter, 21H2, build 20348. The callback slept 900 ms. `SendInput` waited for it to return (`timed-out=false`, `slow-strikes=0`). The timeout path never ran, so this image does not say whether a slow hook is removed.

```
slow-sleep-ms=900
slow-strike index=1 blocked-ms=903 timed-out=false
slow-strikes=0 entered=1 returned=1 probe-seen=true
hook-removed=false
liveness-detect-ms=906
result=measured
```

### windows-2025 timed out and the hook stayed

Windows Server 2025 Datacenter, 24H2, build 26100. Each of 11 markers returned in about 600 ms, before the 900 ms sleep ended. After the 11th callback had returned, the hook was disarmed and one more marker was injected. The hook saw it. `liveness-detect-ms=333` is how long that last check took, not a detection of removal. The hook was not removed on this SYSTEM-in-session setup. Without a non-SYSTEM slow hook, ticket 06 still has to tell "Windows does not remove hooks here" from "this setup never triggers removal".

```
slow-sleep-ms=900
slow-strike index=1 blocked-ms=600 timed-out=true
slow-strike index=2 blocked-ms=611 timed-out=true
slow-strike index=3 blocked-ms=600 timed-out=true
slow-strike index=4 blocked-ms=615 timed-out=true
slow-strike index=5 blocked-ms=602 timed-out=true
slow-strike index=6 blocked-ms=602 timed-out=true
slow-strike index=7 blocked-ms=601 timed-out=true
slow-strike index=8 blocked-ms=614 timed-out=true
slow-strike index=9 blocked-ms=611 timed-out=true
slow-strike index=10 blocked-ms=610 timed-out=true
slow-strike index=11 blocked-ms=601 timed-out=true
slow-strikes=11 entered=11 returned=11 probe-seen=true
hook-removed=false
liveness-detect-ms=333
result=measured
```

`result=measured` means every other check held and the only miss is that the hook was never removed. A hook that was removed, but whose marker check reported it after 500 ms, is `result=fail` and the process exits non-zero. `result=pass` would mean the hook was removed and the marker check reported that within 500 ms. Neither image did that.
