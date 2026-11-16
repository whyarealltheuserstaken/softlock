# Windows tickets for connected and remote displays

Planning files, 16 November 2026. They are not GitHub issues. The behavior is SPEC.md Implementation Decision 20. The capture-versus-remote mechanism in that decision is open.

Windows v1 tickets 1–31 and 35 stay the GitHub issues linked from [issue 33](https://github.com/whyarealltheuserstaken/softlock/issues/33). The next free ticket number after 35 is 36. These files are 36–40. Linux display tickets are a separate set in [`linux/`](linux/README.md), blocked on a Linux platform plan. macOS is [`macos/`](macos/README.md).

From ticket 35 up, ticket N is not GitHub issue #N. Ticket 35 is GitHub issue #38 (Decision 19). GitHub [#37](https://github.com/whyarealltheuserstaken/softlock/pull/37) is the open spike pull request "Spike P6: Windows Hello passthrough proof". GitHub [#40](https://github.com/whyarealltheuserstaken/softlock/pull/40) is the open spike pull request "Spike P1: Ctrl+Alt+Del policies without sign-out". Tickets 36–40 in this directory are planning files. They are not those numbers.

Style matches those issues: what to build, blocked by, where it can be proven, acceptance checkboxes, and the same footer.

## Follow-up for the lead: GitHub issue #18

These files do not edit GitHub [#18](https://github.com/whyarealltheuserstaken/softlock/issues/18). The issue still has its resolution-change check and its second-monitor plug-in photo. Ticket 36 proves the resolution change on the runners without that edit. Its checklist does not require the issue to change. Ticket 39 is the ThinkStation P7 hot-plug sheet.

When the lead edits #18, the proposed text is: keep topmost re-assert, the shell-surface mitigations from spike 07, and toast banners. Point the resolution-change check at ticket 36 and the second-monitor plug-in photo at ticket 39. Start and toasts stay on #18. That edit is the lead's. It is not a box on ticket 36.

## Numbered list

| Id | Title | Blocked by | Where |
|---|---|---|---|
| [36](36-display-change-ci.md) | Seam 3: display add, remove, and change | 11, 12 | CI, `windows-2022` and `windows-2025` |
| [37](37-capture-remote-spike.md) | Spike: capture exclusion versus a remote view | 11, finding 02 | CI, and a written miss if the runner has no remote terminal |
| [38](38-remote-session-lock.md) | Remote session lock and unlock | 15, 21, 22, 37, Decision 20 | CI for the rules that do not need a second machine; the client-SKU session move is 39 |
| [39](39-display-kit-p7.md) | Kit: hot-plug, DisplayLink, inbound RDP on the P7 | 36; the RDP row also 38 | ThinkStation P7 |
| [40](40-windows-10-client-kit.md) | Kit: the same rows on a Windows 10 client | a named Windows 10 client; the RDP row also 38 | No machine identified |

## Blocking edges

```text
11 --> 36
12 --> 36
11 --> 37
36 --> 39
37 --> 38
15 --> 38
21 --> 38
22 --> 38
38 --> 39
38 --> 40
```

The Windows capture entry and the Windows remote-input entry in Decision 20 block 38 until Henry closes them. 36 and 37 do not wait on them. They run on `windows-2022` (Windows 10 family, Server) and `windows-2025` (Windows 11 family, Server) and compare the two. The Windows 10 versus Windows 11 behavior list is in SPEC.md Decision 20. Ticket 40 is the Windows 10 client, and no such machine is identified. Ticket 31 stays blocked by 18. Henry is deciding whether Decision 20 is v1 or post-v1, so this file does not change ticket 31's gating. The display and inbound-RDP rows live on 39 and 40.

GitHub issue #18 is not edited by these files. The proposed issue text is the follow-up above. This change does not create or edit GitHub issues.
