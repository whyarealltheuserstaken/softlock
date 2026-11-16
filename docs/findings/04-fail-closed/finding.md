# 04: Fail-closed lock while the lockdown policies are set

The question is whether a real Windows lock still happens within one second while the lockdown policies are applied, and whether the user's programs keep running.

The probe is `spikes/fail-closed-proof`. It is not part of the product workspace. It journals every policy value, flushes that journal, then writes the value. Revert puts the previous value back, or deletes the value when it was absent. The same revert runs on a normal finish, on Ctrl+C, on panic, from a helper process if the probe is killed, and from the workflow step that runs even when the proof is cancelled.

## What the proof times

All eight lockdown values are applied first. `DisableLockWorkstation` stays set for the disconnect paths. It is cleared only for the last path.

1. `LockWorkStation` while `DisableLockWorkstation` is set. This must fail, and `WTS_SESSION_LOCK` must not arrive.
2. `WTSDisconnectSession` on the console session from the session-0 service. Confirmation is `WTS_CONSOLE_DISCONNECT`.
3. `WTSDisconnectSession` on the probe's own session, from that session. This is the fall-back for when the service is gone. Confirmation is `WTS_CONSOLE_DISCONNECT`.
4. Clear `DisableLockWorkstation`, then `LockWorkStation` immediately. Confirmation is `WTS_SESSION_LOCK`.

Each confirmation is timed from just before the call to the session notification. The pass line is strictly under one second. A background process writes a heartbeat every 50 ms. The longest pause between those writes has to stay under two seconds, which is the check that programs keep running.

## Choice for tickets 15 and 17

Primary OS-lock primitive for ticket 15: clear `DisableLockWorkstation` for the session user, then call `LockWorkStation` from a process in that session. Confirm with `WTS_SESSION_LOCK`.

Fall-back when that process is gone: the service calls `WTSDisconnectSession` on the console session from session 0. Confirm with `WTS_CONSOLE_DISCONNECT`. Programs keep running.

Fall-back when the service is killed: the in-session process calls `WTSDisconnectSession` on its own session. That call does not need `DisableLockWorkstation` cleared. The proof runs it while the value is still set.

Ticket 17 must clear `DisableLockWorkstation` before `LockWorkStation`. The proof's first step is there to show `LockWorkStation` fails while the value is set, and the last step is there to show it succeeds right after the value is cleared. The other lockdown values stay applied the whole time.

This choice stands when both runner images print `result=pass`. If a path fails, the lines below are the evidence and the choice is revised from those lines. Henry's laptop run is still required. Server runners can differ from that laptop, so issue 4 stays open.

## Measured timings

Filled from `report.txt` after the proof job finishes. No number is written here before that log exists.
