# m14: Lock Host: block physical input under the overlay, unlock with a PIN (dev launch)

**What to build:** The first half of the macOS tracer bullet, same shape as Windows ticket 12. A developer starts the lock host directly in a dev-only mode that release builds do not contain. It arms the session tap (or the HID tap, if m05's finding said the session tap misses physical keys) on its own thread, shows the overlay on every display the spikes could cover, and checks a dev-supplied PIN through Core's Argon2id. The PIN check moves to the service in m15. Guard arms before the overlay. The whole arm is under 500 ms.

Capture exclusion is claimed only for the paths m04 and m06 measured as omitting the marker (decision D6). Other paths are named in the finding and are not acceptance criteria.

If CI needs a swallow path, the test-only nonce in `kCGEventSourceUserData` is added in this ticket together with the release-binary scan, not as a follow-up. That is the rule already in SPEC.md Testing Decisions. The scan is the one m01 extended to both slices. Once PR #48 merges, that scan also names `softlock-dev-launch-marker` and `softlock_dev_launch_marker`.

Lessons from PR #48, which this ticket copies: a correct PIN during backoff stays locked, wrong PINs are counted and backed off in `FailingClosed` too, the clock and the watchdog actually advance, and a panic on any thread fail-closes without leaving the tap swallowing input after the overlay is gone. Until m15, this dev launch may issue its own nonce. m15 stops that. The dev launch must not keep issuing its own nonce once m15 lands.

**Blocked by:** m01, m03, m04, m05, m06, m10, m11.

**Blocks:** m15.

**Where:** CI only for posted events, the marker capture counts, the dev PIN, and the release scan. Henry's Intel Mac for the physical checklist. The ticket is not done on CI alone.

**Test machine:** The paragraph in D1 of [`decisions.md`](decisions.md). [Henry's sheet](henry-sheet.md) points at the same paragraph.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] A posted event the finding classifies as software reaches the probe window. A nonce-tagged test event, in the test-only build, does not (CI)
- [ ] Keyboard focus is unchanged after locking (CI, and the same check on his Mac)
- [ ] Each capture path m04 and m06 said we may claim has a zero marker count while locked (CI on the runner OS, and his Mac on his OS)
- [ ] Lock-to-armed time under 500 ms (CI)
- [ ] The dev PIN unlocks. A wrong PIN does not. Typed characters do not reach the probe (CI for the posted path, his Mac for a physical key)
- [ ] A correct PIN during backoff stays locked. The same PIN unlocks after the clock has advanced that backoff (CI). This is the lesson from PR #48
- [ ] Wrong PINs are counted and backed off in FailingClosed too: 5 wrong, then a correct PIN during the backoff stays locked (CI)
- [ ] The window loop advances time from the wall clock, so the backoff expires and the heartbeat watchdog runs (CI)
- [ ] A panic on the tap thread, the overlay thread, or the liveness thread fail-closes and does not leave the tap swallowing input with no overlay (CI)
- [ ] On his Mac, a physical key, click, and trackpad scroll do nothing except wake the PIN prompt, following the m10 grant steps (checklist)
- [ ] Release binaries, both slices, have no dev-launch mode and no test-only marker (m01 scan). Once PR #48 merges, that scan names `softlock-dev-launch-marker` and `softlock_dev_launch_marker` as well as the simulated-physical pair

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
