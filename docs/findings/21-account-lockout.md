# Account-lockout count for a network logon

Issue 21. The Service checks the locked session's password with `LogonUserW` and `LOGON32_LOGON_NETWORK` (3), then closes the token. Spike 9 still has to say whether each rejection increments Windows' bad-password count, including for a Microsoft account that is offline.

## What the hosted runner measured

Seam 3 test `network_logon_lockout_count` runs on the GitHub-hosted `windows-2022` and `windows-2025` images. It is behind `runner_may_lock`, so it does not run on the school workstation.

The test creates a local account, sets the lockout threshold to 3 (duration 1, window 1), and calls the same network logon four times with a password that is not the account's. It prints one line and does not fail the build when the count stays at zero:

`lockout-measure threshold=3 before_bad=… after_bad=… before_locked=… after_locked=… wrong_logons_accepted=…`

The numbers from the green run are filled in below after that log is read. Until then this file records the method, not a guessed count.

| Image | before_bad | after_bad | after_locked | wrong logons accepted |
| --- | --- | --- | --- | --- |
| windows-2022 | pending | pending | pending | pending |
| windows-2025 | pending | pending | pending | pending |

## What the unlock tests refuse to do

The unlock tests set the threshold to 10. Four wrong passwords are under that threshold. Those tests fail if `UF_LOCKOUT` is set on the test account. The product backoff starts after the fourth failure (30 seconds), which is before a threshold of 10.
