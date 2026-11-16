# m15: Service: LaunchDaemon, LaunchAgent, socket, Broker nonce

**What to build:** The service half of the tracer bullet, same shape as Windows ticket 13. A LaunchDaemon is the authority and a LaunchAgent is the lock host. The user-visible name is the existing `SERVICE_DISPLAY_NAME` constant ("SoftLock Agent"). The launchd labels are `com.softlock.agent` and `com.softlock.agent.lockhost`, written next to the bundle id, not as a second product name. Both jobs are `KeepAlive`. A dev install script registers them. The MSI analogue is later and is not this ticket.

Any local process of the console user can lock, over a Unix-domain socket whose path uses the existing `SERVICE_ID`. The JSON verbs are the ones in SPEC.md section 6. There is no unlock verb on that socket. The Broker is a process the service starts as the user, with a fresh 32-byte `LaunchNonce` on a private inherited channel. `Verified` unlocks only when `eq_ct` matches that nonce. The lock host does not verify. Until m18, the dev PIN is checked by the service, not by the lock host.

If m02 found a daemon cannot start a GUI process in the Aqua session, launchd starts the agent at login and the daemon talks to it. That outcome is an acceptance of the finding, not a failure of this ticket.

**Blocked by:** m14 and m02.

**Blocks:** m16, m17, m18.

**Where:** CI only for the socket protocol (roles, no unlock verb, wrong nonce stays locked). Henry's Intel Mac for installing the daemon and the agent and locking from a second process.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] A console-user client can lock. A client that is not the console user cannot (CI)
- [ ] No public message sequence reaches Unlocked from Locked. A nonce with one flipped byte stays locked (CI, seam 1 already covers the state machine; this ticket covers the socket)
- [ ] The dev PIN is checked by the service. The lock host has no verdict that unlocks (CI)
- [ ] On his Mac, the dev install script loads both jobs, a second process can lock, and unloading the script's jobs removes them (checklist)
- [ ] Killing the daemon while locked ends in the real lock primitive m08 picked, or this checklist line waits if m17 has not landed and is recorded as blocked on m17 rather than faked
- [ ] Release binaries, both slices, still pass the m01 scan

The kill line is shared with m17. If m17 has not landed, this ticket's install checklist does not invent a lock primitive. It stops at "jobs load, a second process locks, the dev PIN unlocks."

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
