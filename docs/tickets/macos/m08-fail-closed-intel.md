# m08: Kit: the real lock paths and FileVault status

**What to build:** Henry runs a probe that times the two paths decision D5 named. The first is `SACLockScreenImmediate`, resolved with `dlopen` and `dlsym`. The probe does not link `login.framework`. A missing symbol is a recorded miss, and it is not a crash. The second is the documented lock path: start the screen saver and require the account password to leave it. He also runs `fdesetup status` and `fdesetup isactive` as himself, not as root, and pastes the output.

The same change adds the chooser the product will call, and the CI test of its fallback. The chooser is a pure function of three results: whether `dlsym` returned the symbol, whether the call succeeded, and whether the documented path can require a password with a zero delay. The test feeds it four rows and does not call the live symbol, so the verify job does not lock the runner:

1. Symbol missing, and the documented path can require an immediate password. The documented path is the one requested.
2. Symbol present, call failed, and the documented path can require an immediate password. The documented path is the one requested.
3. Call succeeded. The documented path is not requested.
4. Documented path unavailable or password not required → never Unlocked; refuse to arm. This row is the private call unavailable while password-after-screensaver is off or the delay is not zero (decision D8, the lead). The message the product will show tells the owner how to turn that setting on and set the delay to zero.

That test runs under the existing `cargo test --workspace` job on `ubuntu-latest`, `windows-latest`, and `macos-latest`. A row that skips the documented path, that takes it after a successful private call, or that arms on row 4, fails CI. m17 keeps this test green when the service calls the chooser. At run time, if both paths fail, the guard and the overlay stay up, an error is shown, the lock is retried, and only a verified unlock releases them. That runtime row is m17's checklist. The chooser test here is the arm-time refusal.

This kit does not install the product service. It measures the two paths D5 already chose. It does not pick a third primitive.

**Blocked by:** m02 for the sheet on his Mac. The CI rows do not need a desktop and do not wait on m02. D5 is decided.

**Blocks:** m17.

**Where:** Henry's Intel Mac for the sheet. CI for the four chooser rows, in the existing verify job. Locking the session is a poor default CI job. A later CI job that locks a runner is allowed only if m02 says the runner comes back after a lock, and it runs in its own workflow. The proof that a path shows a lock screen is his Mac.

**Type:** Hardware kit, plus the chooser test. Deliverable is the sheet under `docs/findings/`, and a green verify job.

**Status:** waiting for Henry's Mac for the sheet. The Intel Mac details in D1 are still open. Not ready-for-agent. Not a GitHub issue.

- [ ] Private-call row: reached the real lock and the time, or the error, including a missing symbol when his OS can show one (sheet)
- [ ] Documented-path row: reached the lock screen and the time, and whether a click or a key reached the desktop without the account password (sheet)
- [ ] The sheet records the password-after-screensaver setting and the delay (sheet)
- [ ] If a missing symbol cannot be forced on his Mac, the sheet says so and points at the CI row that covers it
- [ ] CI: symbol missing requests the documented path, and does not crash
- [ ] CI: a failed call requests the documented path
- [ ] CI: a successful private call does not request the documented path
- [ ] CI: documented path unavailable or password not required → never Unlocked; refuse to arm
- [ ] `fdesetup status` and `fdesetup isactive` output as the console user, or the permission error (sheet)
- [ ] Written finding records both paths and says FileVault pre-boot was or was not observed. An Intel Mac without a T2 chip does not prove the Secure Enclave path

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
