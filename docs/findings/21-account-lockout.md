# Account-lockout count for a network logon

Issue 21. The Service checks the locked session's password with `LogonUserW` and `LOGON32_LOGON_NETWORK` (3), then closes the token. The hosted-runner measurement below is for a local account. Spike 9 still has to say whether a Microsoft account that is offline counts the same way.

## What the hosted runner measured

Seam 3 test `network_logon_lockout_count` runs on the GitHub-hosted `windows-2022` and `windows-2025` images. It is behind `runner_may_lock`, so it does not run on the school workstation.

The test creates a local account, sets the lockout threshold to 3 (duration 1, window 1), and calls the same network logon four times with a password that is not the account's. It prints one line and does not fail the build when the count stays at zero:

`lockout-measure threshold=3 before_bad=… after_bad=… before_locked=… after_locked=… wrong_logons_accepted=…`

Run `45135643553` printed the same line on both images:

`lockout-measure threshold=3 before_bad=0 after_bad=3 before_locked=false after_locked=true wrong_logons_accepted=0`

| Image | before_bad | after_bad | after_locked | wrong logons accepted |
| --- | --- | --- | --- | --- |
| windows-2022 | 0 | 3 | true | 0 |
| windows-2025 | 0 | 3 | true | 0 |

A rejected `LOGON32_LOGON_NETWORK` does increment the local account's bad-password count, and the account locks once that count reaches the threshold. The fourth rejection did not raise the count past 3; the account was already locked. None of the wrong passwords were accepted.

A rejected `LOGON32_LOGON_NETWORK` counts, so a flat wait after five failures is not enough: guessing again every 30 seconds still reaches the threshold. Offline Microsoft-account counting is still spike 9.

## What the password path does with that

On each password attempt the Service reads the policy with `NetUserModalsGet` level 3 (threshold, observation window, and duration). The cap uses the threshold and the observation window. Duration is read and not used for the cap. A failed read fails closed: threshold 3 and an observation window of 99999 minutes, which allows one password attempt.

The password budget is `threshold - 2` attempts per observation window, and at least one when the threshold is not 0. Threshold 0 means lockout is disabled, and the path then uses only the escalating wait. A failure stays inside the window until both the monotonic clock and the wall clock say it has aged out. The wait is the stricter of those two clocks, so a wait does not end while the failures still count. The waits escalate: 30 seconds, 1 minute, 2 minutes, 5 minutes, 10 minutes, then 15 minutes.

PIN failures are a separate counter. They use the existing PIN backoff, they never call `LogonUser`, and they do not spend the password budget. Password failures do not move the PIN counter. When the password budget is used up, the prompt says the password path is unavailable and, when a PIN is stored, offers the PIN.

Seam 3 `password_attempts_stay_under_the_lockout_threshold` sets the runner threshold to 5 (so the cap is 3), spreads attempts with the test clock, and checks that the bad-password count stays at 3 and the account does not lock. The test clock is `SOFTLOCK_CLOCK_FILE`, compiled only into the non-default simulated-physical build.
