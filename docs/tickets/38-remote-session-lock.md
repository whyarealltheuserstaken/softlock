# 38: Remote session lock and unlock

**What to build:** On Windows 10 and on Windows 11, when a remote terminal is attached, locking covers that remote view. The requirement is the same. The documented differences are in SPEC.md Decision 20: biometrics toward an RDP session from Windows 10 version 1809, Microsoft Entra authentication from the 2022-10 updates on Windows 10 version 20H2 and on Windows 11, an Entra remote lock that disconnects instead of showing a lock screen that can take those tokens, Hello for Business on the remote device itself unsupported on the FAQ page for both, and an external Hello camera with the lid closed addressed in Windows 11 version 22H2. Where Hello is not supported, unlock is the PIN or the account password. A disconnect from the Entra lock-screen limit is recorded and is not Unlocked. The runners are Server SKUs. The client proof is tickets 39 and 40. Connecting to a machine that is already SoftLocked shows the SoftLock screen on the remote view. Unlock is the existing Broker: a `Verified` verdict with the nonce from the Broker launch this service started. The methods are the SoftLock PIN, Windows Hello where that remote stack supports it, and otherwise the PIN or the account password. No weaker path. The [Hello for Business FAQ](https://learn.microsoft.com/en-us/windows/security/identity-protection/hello-for-business/faq) says Hello for Business authentication on the remote device itself is not supported, and the [RDP sign-in article](https://learn.microsoft.com/en-us/windows/security/identity-protection/hello-for-business/rdp-sign-in) is about the certificate prompt on the client that is connecting, which is a different step. Where the stack in front of the person does not support Hello, this ticket uses the PIN or the account password. Keys, pointer, and touch from the remote terminal before unlock reach only the unlock UI. A crash, a missed heartbeat, or a failed arm ends in the operating-system lock for that session and does not enter Unlocked. The surface that keeps the lock visible on the remote view is the option Henry picks in Decision 20. This ticket does not pick A, B, or C.

**Blocked by:** 15, 21, 22, 37, and the Windows entry in Decision 20. It stays blocked while that entry is open.

**Blocks:** 39 and 40 (the inbound RDP rows)

**Needs Henry's hardware:** The client-SKU case, where the console terminal shows the login screen and the remote terminal has the session, is ticket 39. CI covers the fail-closed rule and the Broker verdict with the test-only nonce. CI does not claim a second physical machine.

**Status:** blocked on Decision 20. Not a GitHub issue. Not ready-for-agent.

- [ ] Locking while a remote terminal is connected covers that view (seam 3 if 37 found a remote path on the runner; otherwise the row is ticket 39 and this box says so)
- [ ] Connecting to an already locked session shows the SoftLock screen on the remote view before any desktop input
- [ ] Unlock accepts only a Broker `Verified` verdict. Hello is offered only where 37's stack supported it. Otherwise the PIN or the account password. A path with neither is a failure, not a success
- [ ] Remote-terminal input before unlock reaches the unlock UI and does not reach the probe window (seam 3 or ticket 39)
- [ ] Killing the lock host, or failing both OS-lock attempts and then panicking, ends in the OS lock for that session and the state is not Unlocked

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
