# Windows tickets for connected and remote displays

Planning files, 16 November 2026. They are not GitHub issues. The behavior is SPEC.md Implementation Decision 20. The capture-versus-remote mechanism in that decision is open.

Windows v1 tickets 1–31 and 35 stay the GitHub issues linked from [issue 33](https://github.com/whyarealltheuserstaken/softlock/issues/33). The next free ticket number after 35 is 36. These files are 36–40. Linux display tickets are a separate set in [`linux/`](linux/README.md), blocked on a Linux platform plan. macOS is [`macos/`](macos/README.md).

Style matches those issues: what to build, blocked by, where it can be proven, acceptance checkboxes, and the same footer.

## Amendment to ticket 18

GitHub [#18](https://github.com/whyarealltheuserstaken/softlock/issues/18) keeps topmost re-assert, the shell-surface mitigations from spike 07, and toast banners. Its resolution-change check and its second-monitor plug-in photo moved here, so the rows exist once.

- A display add, remove, or change is ticket 36 on the runners and ticket 39 on the ThinkStation P7.
- #18 no longer has a resolution-change checkbox, and its hardware photo line no longer includes a second monitor. Start and toasts stay on #18.

This file is the copy in the repository. The GitHub issue is unchanged.

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

The Windows entry in Decision 20 blocks 38 until Henry closes it. 36 and 37 do not wait on it. They run on `windows-2022` (Windows 10 family, Server) and `windows-2025` (Windows 11 family, Server) and compare the two. Ticket 40 is the Windows 10 client, and no such machine is identified. Ticket 31 stays blocked by 18. The display and inbound-RDP rows that would otherwise be added to 31 are 39 and 40.

GitHub issue #18 is not edited by these files. The amended checklist is the section above. The issue still has its original resolution-change and second-monitor lines until someone edits the issue. This change does not create or edit GitHub issues.
