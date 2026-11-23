# Checklist: offline account password

Spike 9. This is not a CI job. It needs the laptop that can sign in with a Microsoft account while the network is off. Do not put that machine's serial number in this repository.

The password path in issue 21 is already in the Service: `LogonUserW` with `LOGON32_LOGON_NETWORK`, the token closed immediately, and the backoff from the settings journal. This checklist is only the part that checklist cannot see from a GitHub-hosted runner.

## Before you start

- Use the laptop, not the school PC.
- Sign in with the Microsoft account you actually use.
- Confirm the lock prompt offers the account password (Tab switches when a PIN is also set).
- Turn the network off after you are signed in: disconnect Wi-Fi and unplug Ethernet. Do not sign out first.

## What to try

1. Lock from the tray.
2. Type the Microsoft account password and submit.
3. Write whether the lock released.
4. Type a wrong password once. Write whether Windows later says the account is locked, and the bad-password count if you can see it without leaving the password in a file or a photo of the prompt.
5. Turn the network back on and confirm you can still sign in.

## What to write

| Check | Result |
| --- | --- |
| Offline Microsoft-account password released the lock | |
| A wrong password increased Windows' lockout count | |
| The account was still usable after the network was back | |

A local account on the same laptop is a useful second row, but the Microsoft account offline is the open question.
