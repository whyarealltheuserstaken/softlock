# m20: CI probe: display reconfiguration callback

**What to build:** On `macos-latest`, which is the arm64 slice of the Universal 2 build, register `CGDisplayRegisterReconfigurationCallback` and change the virtual display's mode from the probe. The `x86_64` slice of the same probe runs on Henry's Intel Mac, and that run is a row on m23, not a second probe. Apple Silicon physical hot-plug is m24, and no Apple Silicon Mac is named. Apple's [Notification of Configuration Changes](https://developer.apple.com/library/archive/documentation/GraphicsImaging/Conceptual/QuartzDisplayServicesConceptual/Articles/Notification.html) says the callback runs when this app reconfigures a local display. Log the display id and the flags. The SDK header [`CGDisplayConfiguration.h`](https://github.com/phracker/MacOSX-SDKs/blob/master/MacOSX10.15.sdk/System/Library/Frameworks/CoreGraphics.framework/Versions/A/Headers/CGDisplayConfiguration.h) names `kCGDisplayBeginConfigurationFlag`, `kCGDisplayAddFlag`, `kCGDisplayRemoveFlag`, `kCGDisplaySetModeFlag`, `kCGDisplayMirrorFlag`, `kCGDisplayUnMirrorFlag`, and `kCGDisplayDesktopShapeChangedFlag`. Print `sw_vers`, `uname -m`, and the `NSScreen` count. A mode change is the case the runner can drive. Restore the mode before the job exits.

**What a runner cannot prove:** a physical panel being plugged or unplugged, a closed lid, AirPlay, Sidecar, DisplayLink, a second physical monitor, or mirroring to a TV. The archive page says the callback also runs when a person disconnects a display. This runner has no such display. Those rows are m23. The finding says that in those words.

**Blocked by:** m01, m02.

**Blocks:** m23.

**Where:** CI only, `macos-latest`, arm64. One virtual display. The Intel slice is m23.

**Type:** Prototype spike. Deliverable is a written finding plus the proof job.

**Status:** waiting for Henry to approve the macOS set. Not ready-for-agent. Not a GitHub issue. This ticket is the display callback. It is not post-v1 and it is not a remote-view ticket.

- [ ] The job logs a begin-configuration callback and a set-mode callback for the mode change it performed (job log)
- [ ] Add, remove, and mirror flags are logged when they arrive. When they do not, the log says the runner did not add, remove, or mirror a display (job log)
- [ ] The finding states that the runner cannot prove physical hot-plug, a closed lid, AirPlay, Sidecar, DisplayLink, or a second physical panel
- [ ] The mode is restored, and the log does not contain a hardware serial number
- [ ] The job prints `uname -m` as `arm64`. The `x86_64` run is m23's checkbox, not this job

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
