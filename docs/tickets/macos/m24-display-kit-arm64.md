# m24: Kit: displays and Screen Sharing on a physical Apple Silicon Mac

**What to build:** The display rows that Apple's pages treat as chip-specific, on a physical Apple Silicon Mac. No Apple Silicon Mac is recorded. This ticket does not invent one. `macos-latest` is the arm64 CI run of the reconfiguration callback (m20). It is not a physical hot-plug, a closed lid, AirPlay, Sidecar, or DisplayLink. The Intel rows are m23. This ticket is the physical Apple Silicon rows: external display hot-plug, the display count that machine actually allows, a closed lid with an external display, AirPlay if a receiver is available, Sidecar if an iPad is available, DisplayLink if a device is available, and inbound Screen Sharing to the locked Mac. The requirement is the same as on Intel. The sheet records the chip and the macOS version. It does not record a serial number, and it does not copy the M5 and M6 counts from Apple's current MacBook Pro guide onto this machine.

**Blocked by:** a named Apple Silicon Mac. The inbound Screen Sharing row is also blocked by m22.

**Blocks:** nothing on the first-demo path. The first demo stays on Henry's Intel Mac.

**Where:** An Apple Silicon Mac. None is recorded.

**Type:** Hardware kit.

**Status:** blocked. No machine is named. Not ready-for-agent. Not a GitHub issue. The display rows stay in scope once a machine is named. The inbound Screen Sharing row is post-v1 and also waits on m22.

- [ ] The machine is named, and the sheet records the chip and `sw_vers` (sheet)
- [ ] External display hot-plug while locked covers the new display, and the sheet records how many displays that chip kept awake (sheet)
- [ ] Closed lid with an external display shows the lock on that display (sheet)
- [ ] AirPlay, Sidecar, and DisplayLink each have a row: covered, or not available (sheet)
- [ ] Inbound Screen Sharing shows the SoftLock screen, and remote input before unlock reaches only the unlock UI (sheet)
- [ ] The sheet does not contain a serial number

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
