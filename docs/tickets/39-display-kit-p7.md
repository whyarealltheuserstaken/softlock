# 39: Kit: displays and inbound Remote Desktop on the P7

**What to build:** A sheet on the Lenovo ThinkStation P7. The roadmap describes it as a high-end desktop with multi-monitor and no lid. This ticket records the OS edition on the sheet, because a client SKU and a server SKU do not move the console the same way (Decision 20). Rows: an external display plugged in and then unplugged while locked; a resolution, scale, or DPI change; mirroring if the P7 can mirror; a closed lid with an external display; DisplayLink or another USB display if one is there; inbound Remote Desktop to the machine while it is SoftLocked. The closed-lid row on this desktop is "no lid, not runnable, unproven." AirPlay and Sidecar are macOS rows on m23. They are not run on the P7. The sheet records resolutions and whether a display was external. It does not record a serial number. Ticket 18 keeps the Start and toast photographs. The second-monitor plug-in that used to be on ticket 18 is this sheet.

**Blocked by:** 36. The inbound Remote Desktop row is also blocked by 38.

**Blocks:** nothing on the v1 critical path. Ticket 31 does not grow a second copy of these rows.

**Needs Henry's hardware:** Yes. The ThinkStation P7.

**Status:** waiting for Henry. Not a GitHub issue. Not ready-for-agent. The display rows stay in scope. The inbound Remote Desktop row has a v1 note that it is not run, and the sheet itself is v1.x or v2. Ticket 31's v1 gating is unchanged.

- [ ] External display hot-plug while locked: the new display shows the lock, and unplugging it leaves the remaining displays covered (sheet)
- [ ] A scale, DPI, or resolution change while locked leaves every display covered (sheet)
- [ ] Mirroring, if the P7 can do it, shows the lock on both outputs; if it cannot, the sheet says so
- [ ] Closed lid with an external display: the P7 has no lid, so the row says not runnable and unproven (sheet)
- [ ] DisplayLink or another USB display, if one is available: the lock is visible on it. If none is available, the sheet says so
- [ ] v1: the inbound Remote Desktop row names ticket 38's detection hook and remote-pass type, and the sheet is not run. v1 does not enable Remote Desktop as a feature
- [ ] v1.x/v2: Inbound Remote Desktop to the locked P7: the remote view shows the SoftLock screen, the real screen appears only after the remote pass, and remote-tool input before that pass does not reach the desktop (sheet)
- [ ] The sheet names the Windows edition and does not contain a serial number

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
