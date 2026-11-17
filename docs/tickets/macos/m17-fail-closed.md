# m17: Fail closed on SACLockScreenImmediate, then the documented path

**What to build:** The service and the lock host call the chooser m08 added. The private call is `SACLockScreenImmediate`, resolved with `dlopen` and `dlsym`, and the binary does not link `login.framework`. A missing symbol or a failed call takes the documented lock path when that path can require an immediate password. Decision D8, from the lead, matches the Windows rule that a failure never ends Unlocked. At arm time, if the private call is unavailable and password-after-screensaver is off or the delay is not zero, the chooser refuses to arm and the message tells the owner how to turn the setting on and set the delay to zero. At run time, if both lock paths fail, the guard and the overlay stay up, an error is shown, the lock is retried, and only a verified unlock releases them. A dead lock host, a dead tap (liveness or a disable event), and a stop of the daemon while locked each end at whichever path the chooser returned, or they stay in that held state when both paths fail. Policies and assertions stay held until that lock is confirmed, then they release. They stay held while the retry is still running. After he signs back in, the app says SoftLock switched to the macOS lock, when, and why, and which path ran. The wording matches the fail-closed sentence already required for Windows, with the OS name changed only in the macOS string resource when this ticket lands. This planning file does not change that string.

The CI test m08 added stays green. This ticket does not replace it with a job that locks the runner. The four rows are still: a missing symbol requests the documented path when that path can require an immediate password, a failed call does the same, a successful private call does not, and documented path unavailable or password not required → never Unlocked; refuse to arm.

**Blocked by:** m08 and m15.

**Blocks:** nothing further in this set. It is the macOS form of Windows ticket 15, off the first-demo path only in the sense that m16 can be shown with the dev PIN first. It is still required before any macOS build is described as fail-closed.

**Where:** CI for the chooser rows, in the existing verify job. Henry's Intel Mac for the real lock screen. A CI job that locks the runner is allowed only as its own workflow, and only if m02 said the runner comes back. It is not the proof.

**Test machine:** The paragraph in D1 of [`decisions.md`](decisions.md). [Henry's sheet](henry-sheet.md) points at the same paragraph.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] Killing the lock host while locked shows the real lock within about a second (checklist)
- [ ] A tap that misses its marker, or that receives a disable event, does the same (checklist)
- [ ] Stopping the daemon while locked does the same, and the lock host's own path works when the daemon is already dead (checklist)
- [ ] After sign-in, the notice names the time, the reason, and which of the two paths ran (checklist)
- [ ] The private call is `SACLockScreenImmediate` via `dlopen`. A different symbol is a new finding, not a silent swap
- [ ] CI still fails when a missing symbol or a failed call skips the documented path, and when a successful private call takes it
- [ ] CI: documented path unavailable or password not required → never Unlocked; refuse to arm
- [ ] When both lock paths fail, the guard and the overlay stay up, an error is shown, the lock is retried, and only a verified unlock releases them (CI for the state, his Mac if he can force both paths to fail)

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
