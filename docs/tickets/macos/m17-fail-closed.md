# m17: Fail closed wired to the primitive m08 picked

**What to build:** The service and the lock host call the primitive m08's finding named. A dead lock host, a dead tap (liveness or a disable event), and a stop of the daemon while locked each end at the real macOS lock. Policies and assertions stay held until that lock is confirmed, then they release. After he signs back in, the app says SoftLock switched to the macOS lock, when, and why. The wording matches the fail-closed sentence already required for Windows, with the OS name changed only in the macOS string resource when this ticket lands. This planning file does not change that string.

**Blocked by:** m08 and m15.

**Blocks:** nothing further in this set. It is the macOS form of Windows ticket 15, off the first-demo path only in the sense that m16 can be shown with the dev PIN first. It is still required before any macOS build is described as fail-closed.

**Where:** Henry's Intel Mac. A CI job that locks the runner is allowed only as its own workflow, and only if m02 said the runner comes back. It is not the proof.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] Killing the lock host while locked shows the real lock within about a second (checklist)
- [ ] A tap that misses its marker, or that receives a disable event, does the same (checklist)
- [ ] Stopping the daemon while locked does the same, and the lock host's own path works when the daemon is already dead (checklist)
- [ ] After sign-in, the notice names the time and the reason (checklist)
- [ ] The primitive is the one m08 wrote down. A different symbol is a new finding, not a silent swap

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
