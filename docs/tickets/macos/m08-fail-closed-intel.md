# m08: Kit: the real lock primitive and FileVault status

**What to build:** Henry runs a probe that tries the fail-closed candidates and times how long until the real lock screen is what he sees. The order is the addendum's order. `SACLockScreenImmediate` is attempted only if decision D5 allows it, and only through `dlopen` / `dlsym`. A missing symbol is a recorded miss, not a crash. The login-window candidate and the screen-saver-plus-password candidate are the other two rows. He also runs `fdesetup status` and `fdesetup isactive` as himself, not as root, and pastes the output.

This kit does not install the product service. It answers which primitive ticket m17 is allowed to call.

**Blocked by:** m02, and D5 for the private-symbol row.

**Blocks:** m17.

**Where:** Henry's Intel Mac. Locking the session is a poor default CI job. A later CI job is allowed only if m02 says the runner comes back after a lock, and it runs in its own workflow. The proof this ticket requires is his Mac.

**Type:** Hardware kit. Deliverable is the sheet under `docs/findings/`.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] Each candidate has a row: reached the real lock, time, or the error, including a missing symbol (sheet)
- [ ] The screen-saver row says whether a click or a key reached the desktop without the account password (sheet)
- [ ] `fdesetup status` and `fdesetup isactive` output as the console user, or the permission error (sheet)
- [ ] Written finding names the primitive m17 will call, and says FileVault pre-boot was or was not observed. An Intel Mac without a T2 chip does not prove the Secure Enclave path

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
