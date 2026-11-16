# macOS backend (phase 2)

The macOS crate compiles and links today. It does not lock the machine. Every platform capability reports Unsupported, and the settings app shows "This platform isn't supported yet".

Phase 2 targets macOS 11 and later on Apple Silicon and Intel. The shape below is the spec's decision, recorded so the core does not have to change when the backend is written.

## Processes

- A root LaunchDaemon is the service.
- A per-user LaunchAgent is the lock host.

## Input guard

A `CGEventTap` at the HID level classifies events by source (`kCGEventSourceStateID` and the source PID). Physical events are swallowed. Synthetic events are passed through so agents keep working.

The user has to grant Accessibility and Input Monitoring permission.

## Overlay

A borderless window at shielding level, with `sharingType = .none`, plus presentation options that disable force quit, process switching, and session termination, and that hide the Dock and menu bar.

## Unlock

`LAContext` (Touch ID or the account password).

## Open risks

- Secure Input mode can starve event taps.
- ScreenCaptureKit on newer macOS versions may ignore `sharingType`. That has to be verified before capture exclusion can be promised.
- The Mac App Store is not a target. Event posting is blocked in the sandbox.

No lock behavior belongs in this crate until those risks have a tested answer.
