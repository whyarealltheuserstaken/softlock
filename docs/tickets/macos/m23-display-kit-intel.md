# m23: Kit: external display, clamshell, AirPlay, Sidecar, DisplayLink, inbound Screen Sharing

**What to build:** Henry runs this on his Intel Mac. The machine is the paragraph in D1. The sheet rows are an external display plugged in while locked, a closed lid with that display still awake, a scale or resolution change, mirroring versus an extended desktop, AirPlay if a receiver is available, Sidecar if an iPad that meets Apple's Sidecar requirements is available, DisplayLink if a DisplayLink device is available, and inbound Screen Sharing to the locked Mac. This 2020 MacBook Pro is in the Mac list on Apple's [Continuity requirements](https://support.apple.com/en-us/108046) (models introduced in 2016 or later). The iPad is the unknown, not the Mac. D7 is unchanged: a lid close with no external display is not a takeover feature. This closed-lid row is the case Apple's [MacBook Pro display guide](https://support.apple.com/guide/macbook-pro/connect-an-external-display-apd8cdd74f57/mac) describes, an external display and accessories such as a keyboard and mouse, with the lid shut. m06 keeps Spaces, full-screen, click-through, and the displays already attached when its probe starts. This ticket does not repeat those. The sheet records resolutions and built-in versus external. It does not record a serial number.

**Blocked by:** m06, m20. The inbound Screen Sharing row is also blocked by m21 and m22.

**Blocks:** nothing on the first-demo path.

**Where:** Henry's Intel Mac. m20 already says what the runner cannot prove.

**Test machine:** The paragraph in D1 of [`decisions.md`](decisions.md). [Henry's sheet](henry-sheet.md) points at the same paragraph.

**Type:** Hardware kit. Deliverable is the filled sheet under `docs/findings/`.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue. The display rows stay in scope. The inbound Screen Sharing row has a v1 note that it is not run, and the sheet itself is v1.x or v2.

- [ ] External display hot-plug while locked: the new display shows the lock, or the sheet names the miss (sheet)
- [ ] Closed lid with an external display awake: the external display shows the lock. If no second keyboard is available, the sheet says the row could not be run (sheet)
- [ ] A scale or resolution change, and mirror versus extend, while locked: every person-visible display shows the lock (sheet)
- [ ] AirPlay, if a receiver is available: mirror or extend shows the lock on that display. If no receiver is available, the sheet says so
- [ ] Sidecar, if an iPad that meets Apple's requirements is available: the iPad shows the lock, and touch or a keyboard on the iPad before unlock reaches only the unlock UI. If no iPad is available, the sheet says so
- [ ] DisplayLink, if a device is available: the USB display shows the lock. If none is available, the sheet says so
- [ ] v1: the inbound Screen Sharing row names m22's detection hook and remote-pass type, and the sheet is not run. v1 does not enable Screen Sharing as a feature
- [ ] v1.x/v2: Inbound Screen Sharing to the locked Mac: the viewer sees the SoftLock screen, the real screen appears only after the remote pass, and remote-tool input before that pass does not reach the desktop (sheet)
- [ ] The `x86_64` slice of the m20 display probe runs on this Mac, and the sheet records `uname -m` (sheet). The arm64 run stays on `macos-latest`. A physical Apple Silicon Mac is m24
- [ ] The sheet does not contain a serial number

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
