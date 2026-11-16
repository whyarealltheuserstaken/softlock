# m05: Kit: physical vs posted input on the Intel Mac

**What to build:** A hardware kit Henry runs on his Intel Mac. The probe is the same one m03 used, built as the universal binary from m01, run as the native x86_64 slice (`uname -m` is `x86_64`). He types a key, clicks, and scrolls the trackpad. He posts events with each of the three source states from a second process. The sheet records state id, source pid, and whether the target app received the event. He then focuses a secure password field and records whether the tap goes quiet, receives `kCGEventTapDisabledByUserInput`, or keeps seeing keys. If he has a spare USB keyboard, he plugs it in while the tap is up and types on it.

If m02 found the runner cannot create a tap, this kit also runs the m03 timeout and marker rows.

**Blocked by:** m03. The probe and the CI table of posted events exist first, so his sheet is a comparison, not the first look.

**Blocks:** m09, m10, m14.

**Where:** Henry's Intel Mac.

**Type:** Hardware kit. Deliverable is the filled sheet committed under `docs/findings/`, with the macOS version and the Mac model, plus the probe log.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] Physical key, click, and trackpad scroll each have a row: state id, source pid, target received it or not (sheet)
- [ ] A post under each of the three source states has the same columns, from a second process (sheet)
- [ ] Secure input has a row: tap disable event or silence, and whether a key reached another app (sheet)
- [ ] USB keyboard hot-plug has a row, or the sheet says the machine had no second keyboard (sheet)
- [ ] The classification rule in the addendum is confirmed or replaced in the finding, and m14 is named

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
