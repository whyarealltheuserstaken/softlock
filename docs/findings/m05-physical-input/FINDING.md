# m05: physical vs posted input on the Intel Mac

Planning ticket: `docs/tickets/macos/m05-physical-input-intel.md`. Issue 54. Blocks m09, m10, and **m14**.

The kit is the workspace-excluded probe `spikes/macos-physical-input`. It uses the same listen-only `kCGSessionEventTap` m03 used: `kCGHeadInsertEventTap` and `kCGEventTapOptionListenOnly`. The callback returns every event unchanged. A failure, panic, hangup, or Control-C disables and releases the tap. The kit does not swallow keys, does not lock the session, and does not read a hardware serial number.

m03 (`docs/findings/m03-tap-classify-ci.md`) already posted under each of the three `CGEventSourceStateID` values on `macos-latest`. Those posts were software. This kit is the comparison on a real keyboard. Until Henry fills the sheet, the addendum draft rule stands:

- State id is HID system state and source pid is 0: treat as physical and swallow.
- Any other state id, or a non-zero source pid: treat as software and pass.

**m14** is the lock host that must follow that table, or replace it with the measured rows in this finding. A user-space driver that posts with HID system state is still expected to look like hardware. That gap is documented, not a surprise.

m02 created a live session tap on the runner, so this kit does not re-run the m03 timeout and marker rows when a tap can be created. If a machine cannot create a tap, those rows run here and the physical rows skip with `no-tap`.

## What the runner can prove

The GitHub job (`macos-latest`, `macos-15-intel`, `macos-26-intel`) is the slice that can still run without a person: create the listen-only tap, post from a **second process** under each source state, record state id, source pid, and whether the target window received the event, then release the tap. Physical key, click, scroll, secure input, and USB stay `not-run-on-runner`. Classification stays `pending-physical`.

## What only a person at a Mac can prove

Test machine header: [Henry's sheet](../../tickets/macos/henry-sheet.md). Target kit Mac: MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports), `MacBookPro16,2`, Intel Core i5, macOS Tahoe 26.7.3, laptop. The kit still runs on Apple silicon, other macOS 11+ versions, desktops with no lid, and one or many displays: rows that do not apply skip with a precise reason (`no-keyboard`, `no-trackpad`, `no-second-keyboard`, `no-tap`, `window-not-key`, `ci`, `operator`, `no-input`, `not-applicable`). They are never labelled as a lid miss. Run `--kit` for the person-at-the-Mac slice. Bare `run.sh` is the full suite.

Rows CI cannot fill:

1. **Physical key, click, and trackpad scroll**: state id, source pid, target received it or not.
2. **Secure input**: tap quiet, `kCGEventTapDisabledByUserInput`, or still seeing keys, and whether a key reached another app.
3. **USB keyboard hot-plug**, or the sheet says the machine had no second keyboard.
4. **Confirm or replace** the draft classification rule for m14.

Do not paste a hardware serial number.
