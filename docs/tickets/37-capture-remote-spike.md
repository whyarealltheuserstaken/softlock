# 37: Spike: capture exclusion versus a remote view

**What to build:** A throwaway probe, not a product change. On `windows-2022` and `windows-2025`, a marker window with `WDA_NONE` and with `WDA_EXCLUDEFROMCAPTURE`. Count the marker in GDI, DXGI Desktop Duplication, and Windows.Graphics.Capture. Finding 02 already saw the marker disappear from those three when exclusion was on. Repeat that, then count the marker on a remote-render path: a remote terminal into the runner if the image allows one, or the remote WDDM indirect display path if one is present. Record `WTSQuerySessionInformation` for the console session and for any remote session, and the low-level hook's injected flag for a console key and for a key typed from the remote client. Microsoft's [terminal page](https://learn.microsoft.com/en-us/windows/win32/termserv/consoles-vs-terminals) treats those as different devices and does not document the flag. [`SetWindowDisplayAffinity`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity) says `WDA_EXCLUDEFROMCAPTURE` does not appear at all except on a monitor, and that the protection is a specific set of public capture APIs, not a guarantee. The page does not name Remote Desktop. The finding lists which paths showed the marker. It says whether the runner could start a remote terminal. It does not choose Decision 20. Windows Server can host more than one remote session. A Windows client moves the one interactive session onto the remote terminal and leaves the console on the login screen. That client-SKU check is ticket 39 if this runner cannot make it.

**Blocked by:** 11. The desktop shape it starts from is finding 02.

**Blocks:** 38

**Needs Henry's hardware:** No for the runner measurement. The client-SKU session move, if this image cannot show it, is ticket 39.

**Status:** planning. Not a GitHub issue. Does not wait on the Windows entry in Decision 20. Does not decide it. Both images run. A Windows 10 client and a Windows 11 client are tickets 40 and 39, not this runner.

- [ ] Marker counts with exclusion off and on, for GDI, DXGI, and Windows.Graphics.Capture (job log)
- [ ] A remote-render path is counted the same way, or the log says the runner has no remote terminal and no remote indirect display (job log)
- [ ] Console input and RDP-session input each have a row: injected flag, or the path was not available (job log)
- [ ] Written finding under `docs/findings/` names the paths that hid the marker and the paths that showed it, and tells ticket 38 which option in Decision 20 the measurement still leaves open

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
