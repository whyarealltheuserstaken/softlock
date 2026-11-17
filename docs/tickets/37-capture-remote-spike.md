# 37: Spike: capture exclusion versus a remote view

**What to build:** A throwaway probe, not a product change. On `windows-2022` and `windows-2025`, a marker window with `WDA_NONE` and with `WDA_EXCLUDEFROMCAPTURE`. Count the marker in GDI, DXGI Desktop Duplication, and Windows.Graphics.Capture. Finding 02 already saw the marker disappear from those three when exclusion was on. Repeat that, then count the marker on a remote-render path: a remote terminal into the runner if the image allows one, or the remote WDDM indirect display path if one is present. Record `WTSQuerySessionInformation` for the console session and for any remote session, and the low-level hook's injected flag for a console key, for a key typed from the remote client, and for a key from an RDP shadow session when the image can start one. Also count the marker, exclusion off and on, through TeamViewer, AnyDesk, and Chrome Remote Desktop when that program is already on the image, and record the injected flag for a key from each. A program that is not installed is a written miss. This ticket does not install one. Those three inject the same way an agent does, which is why the flag row exists: the finding says whether the flag matched an agent's `SendInput`, and it does not invent a way to tell them apart. Microsoft's [terminal page](https://learn.microsoft.com/en-us/windows/win32/termserv/consoles-vs-terminals) treats those as different devices and does not document the flag. [`SetWindowDisplayAffinity`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity) says `WDA_EXCLUDEFROMCAPTURE` does not appear at all except on a monitor, and that the protection is a specific set of public capture APIs, not a guarantee. The page does not name Remote Desktop. The finding lists which paths showed the marker. It says whether the runner could start a remote terminal. The rule is already Decision 20. A shared injected flag is a risk, not a change to that rule. Windows Server can host more than one remote session. A Windows client moves the one interactive session onto the remote terminal and leaves the console on the login screen. That client-SKU check is ticket 39 if this runner cannot make it.

**Blocked by:** 11, for the probe (v1.x/v2). The desktop shape the probe starts from is finding 02, for the probe (v1.x/v2).

**Blocks:** 38's v1.x/v2 slice

**Needs Henry's hardware:** No for the runner measurement. The client-SKU session move, if this image cannot show it, is ticket 39.

**v1 framework:** The detection-hook result type exists, including a field for "remote input could not be told from agent input." A seam test feeds a fake connected session. v1 does not run the capture probe, does not open a remote tool, and does not enable the feature.

**v1.x or v2:** The checkboxes below. Both images run. A shared injected flag is a risk. The rule stays: remote-tool input is blocked until the remote pass, and local agent input is not blocked because of the shared flag.

**Status:** v1 framework plus a v1.x or v2 probe. Not a GitHub issue. Henry closed the rule in Decision 20. This spike does not reopen it. A Windows 10 client and a Windows 11 client are tickets 40 and 39, not this runner.

- [ ] v1: the detection-hook result type includes "could not tell remote input from agent input," and a seam test sets it from a fake. No remote tool is opened (seam 1)
- [ ] v1.x/v2: Marker counts with exclusion off and on, for GDI, DXGI, and Windows.Graphics.Capture (job log)
- [ ] v1.x/v2: A remote-render path is counted the same way, or the log says the runner has no remote terminal and no remote indirect display (job log)
- [ ] v1.x/v2: Console input, RDP-session input, and an RDP shadow each have a row: injected flag, or the path was not available (job log)
- [ ] v1.x/v2: TeamViewer, AnyDesk, and Chrome Remote Desktop each have a marker count with exclusion off and on, and an injected-flag row, or the log says that program was not on the image (job log)
- [ ] v1.x/v2: Written finding under `docs/findings/` names the paths that hid the marker and the paths that showed it, and names any remote-tool input that shares the agent's injected flag. The finding does not recommend letting that input through, and it does not recommend blocking local agent input

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI), the product name only via the name constant.
