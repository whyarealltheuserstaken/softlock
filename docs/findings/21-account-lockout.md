# Account-lockout count for a network logon

Issue 21. The Service checks the locked session's password with `LogonUserW` and `LOGON32_LOGON_NETWORK` (3), then closes the token. The hosted-runner measurement below is for a local account. Spike 9 still has to say whether a Microsoft account that is offline counts the same way.

## What the hosted runner measured

Seam 3 test `network_logon_lockout_count` runs on the GitHub-hosted `windows-2022` and `windows-2025` images. It is behind `runner_may_lock`, so it does not run on the school workstation.

The test creates a local account, sets the lockout threshold to 3 (duration 1, window 1), and calls the same network logon four times with a password that is not the account's. It asserts `before_bad=0`, `after_bad=3`, the account locks, and none of the wrong logons were accepted.

Run `45135643553` printed this line on both images before the assert was added:

`lockout-measure threshold=3 before_bad=0 after_bad=3 before_locked=false after_locked=true wrong_logons_accepted=0`

| Image | before_bad | after_bad | after_locked | wrong logons accepted |
| --- | --- | --- | --- | --- |
| windows-2022 | 0 | 3 | true | 0 |
| windows-2025 | 0 | 3 | true | 0 |

A rejected `LOGON32_LOGON_NETWORK` does increment the local account's bad-password count, and the account locks once that count reaches the threshold. The fourth rejection did not raise the count past 3; the account was already locked. None of the wrong passwords were accepted.

A rejected `LOGON32_LOGON_NETWORK` counts, so a flat wait after five failures is not enough: guessing again every 30 seconds still reaches the threshold. Offline Microsoft-account counting is still spike 9.

## What the password path does with that

On each password attempt the Service reads the policy with `NetUserModalsGet` level 3 (threshold, observation window, and duration). The cap uses the threshold and the observation window. Duration is read and not used for the cap. A failed read fails closed: threshold 2 and an observation window of 99999 minutes, which allows no password attempt.

The password budget is `threshold - 2` attempts per observation window. Threshold 1 or 2 allows none: one attempt can lock the account. Threshold 0 means lockout is disabled, and the path then uses only the escalating wait. Windows resets the bad-password count to zero only after the observation window has passed since the latest failure. SoftLock therefore counts every recorded failure while that latest stamp is still inside the window on either clock, and counts zero once it has aged out on both. Aging failures out one by one handed attempts back and could lock the account. The waits escalate: 30 seconds, 1 minute, 2 minutes, 5 minutes, 10 minutes, then 15 minutes.

Before `LogonUser`, the Service reads `usri3_bad_pw_count` and `UF_LOCKOUT` with `NetUserGetInfo` level 3 for the session user. It refuses when the account is locked, when one more failure would reach `threshold - 1`, or when the read fails. Failures from the sign-in screen, RDP, shares, or another program count in that number. For a Microsoft account or a domain account the local count may not be the count Windows enforces. That remains spike 9.

The failure stamp is saved to the journal before `LogonUser`. If that save fails, `LogonUser` is not called. Only `ERROR_LOGON_FAILURE` (1326) keeps the stamp. 1909, 1330, 1907, 1385, 1327, and 1311 turn the password path off for this lock and are logged as the result code, never the password. A successful check clears the stamps.

Stamps from a previous boot stay in force. Dropping them on startup would need a boot identity that does not move when the clock does. PR 105 is adding `LockIdentity`. Until that survives a clock change, a reboot does not clear the password budget.

PIN failures are a separate counter. They use the existing PIN backoff, they never call `LogonUser`, and they do not spend the password budget. Password failures do not move the PIN counter. When the password budget is used up, the prompt says the password path is unavailable and, when a PIN is stored, offers the PIN.

Seam 3 `password_attempts_stay_under_the_lockout_threshold` sets the runner threshold to 5 (so the cap is 3), spreads attempts with the test clock, and checks that the bad-password count stays at 3 and the account does not lock. The test clock is `SOFTLOCK_CLOCK_FILE`, compiled only into the non-default simulated-physical build.
