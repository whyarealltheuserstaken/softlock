# 36: Seam 3: display add, remove, and change

**What to build:** On `windows-2022` and `windows-2025`, the seam 3 probe covers every monitor after a display change while locked. [`WM_DISPLAYCHANGE`](https://learn.microsoft.com/en-us/windows/win32/gdi/wm-displaychange) is the message top-level windows receive when the resolution changes. [`WM_DPICHANGED`](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged) is the message when the window's DPI changes. Finding 02 measured these images as one 1024×768 console display, so a resolution change is the change the job can do without new hardware. An add and a remove run only when the image already has a virtual display driver. This ticket does not install a driver onto the hosted image. **Verified, read 16 November 2026.** The [indirect display driver model](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/indirect-display-driver-model-overview) is a user-mode driver for monitors that are not on a traditional GPU output. The page names three cases: streaming the desktop to a remote client, a virtual monitor, and a USB dongle with a monitor on the other end. If the runner has no such driver, the log says add and remove are unproven in CI. Ticket 39 proves those on the ThinkStation P7. The repository plan leaves topmost re-assert, shell surfaces, and toast banners on ticket 18. GitHub issue #18 is not edited by this ticket, and this ticket does not repeat those rows.

**Blocked by:** 11, 12

**Blocks:** 39

**Needs Henry's hardware:** No. Add, remove, a real DPI change, and mirroring, when the runner cannot do them, are ticket 39.

**Status:** in scope. Not a GitHub issue. This ticket is display change, not a remote view. Ticket 31's v1 gating is unchanged.

`windows-2022` is Windows Server 2022, the Windows 10 family (finding 02: build 20348). `windows-2025` is Windows Server 2025, 24H2, the Windows 11 family (finding 02: build 26100). Both images run this job. A difference in whether the resolution change stays covered fails the job. Hello, a client SKU, and a Windows 10 client machine are not this ticket. Those are 38, 39, and 40.

- [ ] Each image prints its product name and build, and a resolution change while locked leaves every monitor covered on both (seam 3)
- [ ] A DPI or scale change is either covered the same way or the log says the runner could not change DPI (seam 3)
- [ ] An added virtual monitor is covered, and a removed one drops out of the covered set, or the log says no virtual display driver was available and names add and remove unproven in CI (seam 3)
- [ ] The job does not install a display driver onto the hosted image (job log)
- [ ] The job log names this ticket as the resolution-change row. The job does not read or write GitHub issue #18. Editing that issue is a follow-up for the lead, listed in `docs/tickets/README.md`, and it is not a box on this ticket

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
