# Finding: Ctrl+Alt+Del policies without signing out

**Status:** not run. The tables below are blank on purpose. Fill them from [CHECKLIST.md](CHECKLIST.md) after the test on each computer. Do not guess.

This spike asks two questions:

1. Does each option disappear the next time Ctrl+Alt+Del is pressed, with no sign-out, and does clearing the setting bring it back immediately?
2. Which icons are still on that screen (power, network, accessibility)? Does `NoClose` hide the power icon? What can the accessibility icon launch, and can that reach the desktop? Does the network icon matter?

The program under test is `spike-ctrl-alt-del`. It is not part of the release build. CI runs `spike-ctrl-alt-del check` on a Windows runner. That check sets, clears, and restores every value and reads the registry back. It is not a substitute for the tables below.

## Machines

| Computer | Windows version | Edition | OS Build | Notes |
|---|---|---|---|---|
| Henry's laptop | | | | |
| School Lenovo ThinkStation P7 | | | | |
| Windows 10 22H2, only if neither machine above is Windows 10 | | | | |

## Question 1. Each option, with no sign-out

Write **yes** or **no**. If the item was not on the first screen, write **not on the first screen**. Leave a cell blank until that computer has been tested.

| Setting | What it should hide | Laptop: hidden immediately | Laptop: back immediately after clear | P7: hidden immediately | P7: back immediately after clear | Windows 10: hidden immediately | Windows 10: back immediately after clear | Notes |
|---|---|---|---|---|---|---|---|---|
| DisableTaskMgr | Task Manager | | | | | | | |
| NoLogoff | Sign out | | | | | | | |
| DisableChangePassword | Change a password | | | | | | | |
| HideFastUserSwitching | Switch user | | | | | | | |
| DisableLockWorkstation | Lock | | | | | | | |
| NoClose | Power icon (unknown until this test) | | | | | | | |

## Question 2. Icons that remain

| Icon | Laptop: still there with all six set? | Laptop: what it can do | Laptop: path to the desktop? | P7: still there? | P7: what it can do | P7: path to the desktop? | Photo file |
|---|---|---|---|---|---|---|---|
| Power | | | | | | | |
| Network | | | | | | | |
| Accessibility | | | | | | | |
| Anything else | | | | | | | |

### Accessibility tools

| Tool | Laptop: opened? | Laptop: could it type, click, or show the desktop? | P7: opened? | P7: could it type, click, or show the desktop? | Photo file |
|---|---|---|---|---|---|
| Narrator | | | | | |
| On-screen keyboard | | | | | |
| Magnifier | | | | | |
| Anything else in the menu | | | | | |

## Decision for each gap

A gap is anything that stayed on the screen, or anything that opened a way to the desktop. For each gap, the choice is one of: **neutralize by hardening**, **Keyboard Filter**, or **document as a limitation**.

Leave this table empty until the tables above are filled in. Add one row per gap. Do not fill a decision before the test.

| Gap | Computer and Windows edition | Decision | Why |
|---|---|---|---|
| | | | |

## Dependent tickets

This finding is an input to:

- [#17](https://github.com/whyarealltheuserstaken/softlock/issues/17) Ctrl+Alt+Del lockdown and UAC credentials while locked. It applies these policies while locked and needs to know which ones actually hide their button without a sign-out.
- [#19](https://github.com/whyarealltheuserstaken/softlock/issues/19) Disable touch, pen, gamepads and accessibility input while locked. It needs the list of tools the accessibility icon can launch.
- [#30](https://github.com/whyarealltheuserstaken/softlock/issues/30) User guide, terms, recovery guide, contributor guide and diagnostics export. Its honesty section needs the icons and paths this test could not close.
- [#32](https://github.com/whyarealltheuserstaken/softlock/issues/32) Deferred: Optional Keyboard Filter on Enterprise and Education editions. Revive it if a gap can only be closed by suppressing Ctrl+Alt+Del itself.

## Manual acceptance

Henry checks these only after both computers are filled in above, the photos are saved, and every gap has a decision. An agent must not check them.

- [ ] Manual checklist result recorded per value, per Windows version and edition: hidden immediately yes/no, restored immediately yes/no
- [ ] Remaining icons and what each can do are listed, with photos, and every gap has a decision: neutralize by hardening, Keyboard Filter, or document as a limitation
- [ ] Dependent tickets are named: 17, 19, 30, and the deferred Keyboard Filter ticket
