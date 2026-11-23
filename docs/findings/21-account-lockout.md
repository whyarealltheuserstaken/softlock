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

The product delay still starts after the fourth failure, which is the existing PIN curve. A Windows threshold of 3 therefore locks the account before that delay. The seam 3 unlock tests set the threshold to 10, which is above those four failures, and they fail if the test account is locked out. An administrator who sets the Windows threshold to 4 or less will have Windows lock the account first. Offline Microsoft-account counting is still spike 9.

## What the unlock tests refuse to do

The unlock tests set the threshold to 10. Four wrong passwords are under that threshold. Those tests fail if `UF_LOCKOUT` is set on the test account. The product backoff starts after the fourth failure (30 seconds), which is before a threshold of 10.
