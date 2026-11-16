# m03: Spike: posted-event fields, tap disable, and the liveness marker

**What to build:** On `macos-latest`, if m02 got a live tap. If m02 did not, this ticket's finding is "the runner cannot host a tap" and the same measurements move to m05 with no CI claim. For each `CGEventPost` using `kCGEventSourceStatePrivate`, `kCGEventSourceStateCombinedSessionState`, and `kCGEventSourceStateHIDSystemState`, record `kCGEventSourceStateID`, `kCGEventSourceUnixProcessID`, and whether the target window received the event. Sleep in the callback and record whether `kCGEventTapDisabledByTimeout` or `kCGEventTapDisabledByUserInput` arrives, how long it takes, and whether a key posted during the sleep reaches the target. Then run the liveness marker: post a no-op, require the callback to see it, and show that a callback that never runs is reported inside the existing heartbeat bound (three misses of 250 ms). Do not re-enable the tap and call that success.

This is the CI half of the Windows liveness lesson in `docs/findings/05-input-hook/README.md` (commit `e399397de95fe6699f7f094e82bb65fb289cfaf9`): a timeout report is not proof the tap is gone.

**Blocked by:** m02.

**Blocks:** m05, m14.

**Where:** CI only. There is no physical keyboard on the runner. Physical keys are m05.

**Type:** Prototype spike. Deliverable is a written finding plus the proof job.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] The proof job prints the state id and source pid for a post under each of the three source states (job log)
- [ ] The proof job prints whether the target window received each of those posts (job log)
- [ ] A slow callback records the disable event type, the wait, and whether a key reached the target during the wait, or records that no disable event arrived (job log)
- [ ] The marker check reports a dead callback within 1 second, and a healthy callback within one heartbeat (job log)
- [ ] Written finding: the draft classification rule stands or is replaced, and m05's hardware rows are listed

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
