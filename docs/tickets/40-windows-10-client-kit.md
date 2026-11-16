# 40: Kit: Windows 10 client displays and inbound Remote Desktop

**What to build:** The same display and remote-desktop rows as ticket 39, on a Windows 10 client machine (version 2004 or later). The roadmap still has no Windows 10 machine identified (issue 33, still-open item 7). This ticket does not name one and does not borrow the P7. `windows-2022` is Windows Server 2022 and is the CI stand-in for the Windows 10 family in tickets 36 and 37. It is not this kit. Rows, once a machine exists: external display hot-plug, a scale or DPI change, a closed lid with an external display if the machine is a laptop, DisplayLink if a device is available, and inbound Remote Desktop while SoftLocked. Hello is offered only where that Windows 10 stack supports it. Otherwise the PIN or the account password. The Entra remote-session case that disconnects on lock, documented in Decision 20, is a row of its own if the machine is Entra-joined. The sheet records the edition and the build. It does not record a serial number.

**Blocked by:** a named Windows 10 client machine. The inbound Remote Desktop row is also blocked by 38.

**Blocks:** nothing on the v1 critical path.

**Needs Henry's hardware:** Yes, and no machine is identified. Henry's laptop is not assumed to be Windows 10. The sheet on that laptop, if it is Windows 11, stays with the Hello tickets already on the roadmap. This ticket starts when someone records a Windows 10 client.

**Status:** blocked. No Windows 10 client is identified. Not a GitHub issue. Not ready-for-agent.

- [ ] The machine is named, and the sheet records Windows 10 and the build (sheet)
- [ ] External display hot-plug while locked covers the new display (sheet)
- [ ] A scale or DPI change while locked leaves every display covered (sheet)
- [ ] Closed lid with an external display, if the machine has a lid. If it has no lid, the row says not runnable (sheet)
- [ ] DisplayLink, if a device is available. If none is available, the sheet says so
- [ ] Inbound Remote Desktop shows the SoftLock screen, unlock is the Broker methods 38 allows on Windows 10, and remote input before unlock does not reach the desktop (sheet)
- [ ] If the machine is Microsoft Entra joined, locking the remote session is recorded as a disconnect or as a lock screen, and the result is not Unlocked (sheet)
- [ ] The sheet does not contain a serial number

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
