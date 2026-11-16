# 38: Remote session lock and unlock

**What to build:** On Windows 10 and on Windows 11, when a remote terminal is attached, locking covers that remote view. The requirement is the same on both. The Windows 10 versus Windows 11 list (RDP, Hello, Entra) is the one in SPEC.md Decision 20. This ticket does not copy it. A disconnect from the Entra lock-screen limit, which that list records, is not Unlocked. The runners are Server SKUs. The client proof is tickets 39 and 40. Connecting to a machine that is already SoftLocked shows the SoftLock screen on the remote view. Unlock is the existing Broker: a `Verified` verdict with the nonce from the Broker launch this service started. The methods are the SoftLock PIN, Windows Hello where that remote stack supports it, and otherwise the PIN or the account password. No weaker path. Where the stack in front of the person does not support Hello, this ticket uses the PIN or the account password. Remote-terminal input before unlock is the open remote-input entry in Decision 20. This ticket does not claim the hook can tell TeamViewer, AnyDesk, or Chrome Remote Desktop apart from an agent. A killed lock host, or both OS-lock attempts failing, keeps the guard up or reaches the real lock, and the state is not Unlocked. That wording is invariant 3 in Decision 5, plus the known gap in section 7 until ticket 13. The surface that keeps the lock visible on the remote view is the option Henry picks in the Windows capture entry. This ticket does not pick it. Option B is dropped there.

**Blocked by:** 15, 21, 22, 37, and the Windows capture entry and the Windows remote-input entry in Decision 20. It stays blocked while either entry is open.

**Blocks:** 39 and 40 (the inbound RDP rows)

**Needs Henry's hardware:** The client-SKU case, where the console terminal shows the login screen and the remote terminal has the session, is ticket 39. CI covers the fail-closed rule and the Broker verdict with the test-only nonce. CI does not claim a second physical machine.

**Status:** blocked on Decision 20. Not a GitHub issue. Not ready-for-agent.

- [ ] Locking while a remote terminal is connected covers that view (seam 3 if 37 found a remote path on the runner; otherwise the row is ticket 39 and this box says so)
- [ ] Connecting to an already locked session shows the SoftLock screen on the remote view before any desktop input
- [ ] Unlock accepts only a Broker `Verified` verdict. Hello is offered only where 37's stack supported it. Otherwise the PIN or the account password. A path with neither is a failure, not a success
- [ ] Remote-terminal input before unlock reaches the unlock UI and does not reach the probe window (seam 3 or ticket 39). The mechanism is the option Henry picked in the remote-input entry
- [ ] A killed lock host, or both OS-lock attempts failing, keeps the guard up or reaches the real lock, and the state is not Unlocked

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
