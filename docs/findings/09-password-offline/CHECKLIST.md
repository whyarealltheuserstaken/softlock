# Checklist: offline account password

Spike 9. This is not a CI job. It needs the laptop that can sign in with a Microsoft account while the network is off. Do not put that machine's serial number in this repository.

The password path in issue 21 is already in the Service: `LogonUserW` with `LOGON32_LOGON_NETWORK`, the token closed immediately, and the backoff from the settings journal. This checklist is only the part that checklist cannot see from a GitHub-hosted runner.

## Before you start

- Use the laptop, not the school PC.
- Set a SoftLock PIN and confirm it unlocks before any password step. If the password path is exhausted or the account is close to lockout, the PIN is the way back in.
- Sign in with the Microsoft account you actually use.
- Write down `net accounts` (threshold, observation window, duration) and the current bad-password count before any wrong password. Stop if that count is within two of the threshold. Do not keep going until the account locks.
- If Windows Hello is required for this account ("Require Windows Hello sign-in for Microsoft accounts" removes the password), record that and do not type a password that no longer exists.
- Confirm the lock prompt offers the account password (Tab switches when a PIN is also set).
- Turn the network off after you are signed in: disconnect Wi-Fi and unplug Ethernet. Do not sign out first.

## What to try

1. Lock from the tray.
2. Type the Microsoft account password and submit. SoftLock passes `LookupAccountSidW`'s local SAM alias and the computer name to `LogonUserW`. `MicrosoftAccount\email` is the internet name from `LsaLookupSids2` with `LSA_LOOKUP_PREFER_INTERNET_NAMES`, not the form `LogonUserW` documents. The documented UPN form is the email with a null domain. Try the alias first. Only if that fails, try the email as a UPN, and write which one released the lock. Do not try both if the bad-password count is already near the threshold.
3. Write whether the lock released.
4. Type one wrong password, then stop. Write whether the local bad-password count moved. For a Microsoft account or a domain account that local count may not be the count Windows enforces.
5. Turn the network back on and confirm you can still sign in. Use the SoftLock PIN if the password path will not.

## What to write

| Check | Result |
| --- | --- |
| `net accounts` before any wrong password | |
| Bad-password count before any wrong password | |
| Windows Hello is set up | |
| "Require Windows Hello sign-in" has removed the password | |
| SoftLock PIN was set before the password steps | |
| Offline password released the lock with the local alias | |
| Offline password released the lock with the email UPN | |
| A wrong password increased the local bad-password count | |
| The local count matches what Windows enforces for this Microsoft or domain account | |
| The account was still usable after the network was back | |

A local account on the same laptop is a useful second row, but the Microsoft account offline is the open question. Stop before the bad-password count gets near the threshold.
