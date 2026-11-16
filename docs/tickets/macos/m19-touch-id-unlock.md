# m19: Unlock with Touch ID through the same Broker

**What to build:** The same Broker and the same nonce as m18, accepting a Touch ID success from `LAPolicy.deviceOwnerAuthentication` as `Verified`. No second unlock channel. The password fallback stays m18's path. This ticket is not started on an Intel Mac that m12 said has no sensor, and it is not started on a GitHub runner.

**Blocked by:** m12 and m18.

**Blocks:** nothing in this set.

**Where:** Apple Silicon only. Not CI. Not an Intel Mac without a Touch ID sensor. If m12 moved onto Henry's Intel Mac because that Mac has Touch ID, this ticket moves with it.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue. Off the Intel critical path.

- [ ] Touch ID unlocks, and the nonce check is the same one m18 tests in CI (checklist on a Mac with Touch ID, plus the existing nonce test)
- [ ] Cancel falls through to the password sheet and does not unlock (checklist)
- [ ] The lock host still has no verdict of its own (CI)
- [ ] The finding names the Mac model and confirms the label

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
