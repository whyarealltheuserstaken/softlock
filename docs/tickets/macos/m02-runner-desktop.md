# m02: Spike: can macos-latest host a window, posted events, capture, and a LaunchAgent?

**What to build:** The macOS analogue of Windows spike P7 ([issue 2](https://github.com/whyarealltheuserstaken/softlock/issues/2), finding in `docs/findings/02-windows-runner-desktop.md`). A throwaway probe, not a product crate, on `macos-latest`. It records `sw_vers`, `uname -m`, whether a borderless window can order front, whether `CGEventPost` delivers a key to that window, which capture APIs open, and whether a LaunchAgent and a LaunchDaemon can be loaded and unloaded in the job. It records the TCC result of creating a session event tap: created, `NULL`, or a prompt the job cannot click. The probe refuses to install a daemon unless `GITHUB_ACTIONS=true`, the same shape as the Windows desktop proof.

**Blocked by:** none. Can start with m01.

**Blocks:** m03, m04, m07, m08, m11, m15.

**Where:** CI only.

**Type:** Prototype spike. Deliverable is a written finding plus a minimal proof job. The finding names the dependents above and says which later tickets stay on the runner and which move entirely to Henry's Intel Mac.

**Status:** runner finding recorded in `docs/findings/m02-macos-runner-desktop.md` ([issue 51](https://github.com/whyarealltheuserstaken/softlock/issues/51)), from run 43963660084 on `07d4074adbb94a7cbbfea0dd6178460f6cbdceca`. The proof window became key and received the `CGEventPost` key. The LaunchAgent window did not stay visible. Hardware follow-ups are in that finding's section for Henry's Intel Mac.

- [x] The proof job prints `sw_vers` and `uname -m`, and `uname -m` is `arm64` (job log)
- [x] The proof job shows a window and a `CGEventPost` key that the window receives, or a written miss that names the API error (job log)
- [x] The proof job attempts `kCGSessionEventTap` and `kCGHIDEventTap` and records `NULL` versus a live tap, including any TCC denial (job log)
- [x] The proof job loads and unloads a throwaway LaunchAgent and a throwaway LaunchDaemon, and the job's last step unloads them even when the proof step is cancelled (workflow)
- [x] Written finding committed under `docs/findings/`, with the run URL and the head SHA, deciding which of m03, m04, m07, m08, and m15 can stay on the runner

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
