# SoftLock v1: Product Spec

| | |
|---|---|
| **Status** | Approved. Ticket breakdown approved 15 Nov 2026 (31 v1 tickets, 1 deferred). Gaps resolved during ticketing are in Implementation Decision 17. Amended 15 Nov 2026 with two Henry-approved v1 features: the battery and heat floor (Decision 10) and pausing background apps (Decision 18), summarized in Decision 19; ticket 35 added. On 16 Nov 2026 Henry decided v1 ships both pause modes, with gentle pause as the default. The same day he decided connected, multi-display, and remote-display support on Windows 11, Windows 10, Linux, and the macOS Universal build, both slices (Decision 20). Each platform has its own open capture-versus-remote entry and its own open remote-input entry in that decision. The macOS phase 2 addendum of 16 Nov 2026 (`docs/platforms/macos.md`, tickets in `docs/tickets/macos/`) is planning only: it does not change Windows v1 behavior and it is not approved until Henry says so |
| **Owner** | Henry |
| **Date** | 15 November 2026 |
| **Working name** | SoftLock. Pick a new name before public launch (see Further Notes) |
| **License** | Apache-2.0 (free, open-source core) |
| **v1 platform** | Windows 11 and Windows 10 (version 2004 or later). macOS 11+ and mainstream Linux come in later phases, and the platform abstraction is designed now |

---

## Problem Statement

AI agents now run on people's own computers for long stretches. Examples are Cursor and Claude Code agents running on-device tests, ChatGPT Computer Use clicking through apps, Playwright suites, and RPA flows. These agents need an unlocked, drawn, interactive desktop. When the owner steps away, they have two bad choices:

1. **Lock the computer (Win+L, idle lock, lid close).** Windows switches to the secure Winlogon desktop. Agents can no longer send input or take useful screenshots, so every GUI agent and UI test stops. OpenAI's own docs tell Windows users to leave the device unlocked, and RPA and UI-test forums have described the same failure for years.
2. **Leave it unlocked.** Anyone walking by, plus kids and pets, can read the screen, type into the agent's window, close apps or open Task Manager. A single stray keystroke can wreck a 45-minute agent run.

Existing tools don't solve this. Toddler-style input blockers have no real password and fall apart at Ctrl+Alt+Del. Keep-awake utilities don't lock anything. Remote-lock tools call the real OS lock, which kills agents. The closest products are Softros Transparent Lock Screen (paid, closed, Windows only, not built for agents) and Warden (paid, macOS 15.2+ only, and it tells you to use the real lock when you leave). Nothing is free, open source, cross-platform by design, agent-first, and honest about loopholes.

The owner wants something that behaves **exactly like a lock screen** to anyone physically at the computer. The password is required, Ctrl+Alt+Del can't break out, and there are no loopholes. At the same time, software keeps running in the foreground and agents keep seeing and driving the real desktop.

## Solution

SoftLock is a second lock layer that sits beside the operating system's lock.

- **Locking.** You press a hotkey (or click the tray icon or the app button) and SoftLock locks within half a second. The physical screen shows a clearly branded SoftLock lock screen: a clock, an optional message and an unlock prompt. You choose whether it's opaque, dimmed or see-through.
- **Physical input is blocked, software input gets through.** SoftLock blocks every keystroke, click and touch that comes from real hardware. Input that software creates (agents, automation, test frameworks) passes through untouched. Focused windows stay focused, and nothing steals the agent's cursor or keyboard focus.
- **Agents see the real desktop.** The lock screen is excluded from screen capture. An agent's screenshot shows the actual desktop, and its clicks land on the actual apps, while the person at the monitor sees only the lock.
- **Ctrl+Alt+Del can't break out.** Windows doesn't let any app swallow Ctrl+Alt+Del. While SoftLock is locked, a small admin background service strips that screen down to (in effect) **Cancel**, which drops you straight back onto SoftLock. No Task Manager, sign out, switch user, change password or lock. The service puts every setting back on unlock, and repairs them automatically after a crash or power loss.
- **Unlock with your normal Windows sign-in.** That means Windows Hello (face, fingerprint, PIN) or your account password, with an optional separate SoftLock PIN. The unlock design is pluggable, so more methods can be added later.
- **It fails closed.** If any part of SoftLock crashes or hangs while locked, the service immediately hands over to the real Windows lock. The computer is never left open. This is stated plainly in the setup wizard, the user guide and the terms.
- **It keeps the computer working, within safe limits.** While locked, SoftLock keeps the computer awake and online, including on Modern Standby laptops, so agents and downloads keep running. If the battery runs low or the computer reports overheating, SoftLock switches to the real Windows lock and lets the computer sleep. As an option, it also pauses background apps you picked (chat or music apps, for example) while locked, and wakes them when you unlock.
- **It can replace the real lock (your choice).** A setting in the first-run wizard, easy to change later, lets SoftLock take over Win+L, the idle lock and lid-close locking. It's off by default in v1.
- **Settings are scriptable but tamper-proof.** You change settings in a clean settings app or in a readable TOML file that you or an AI agent can edit. Changes are accepted only while the computer is not locked, by SoftLock or by Windows.
- **Locking is safe to automate.** The service exposes a lock-only local interface. Agents and scripts can lock the computer, and nothing outside the lock screen can ever unlock it. The command-line tool and the remote chat-bot triggers that build on this come later.
- **It's easy to install and keep current.** A normal installer explains in plain words what the background service does and why it needs admin rights. An in-app updater uses signed update manifests and offers stable and dev channels. Builds start without code signing (a warning is expected), and signing is added at launch.
- **Disk encryption is part of the story.** Pulling the power or holding the power button can't be stopped by any software. It lands at the real Windows sign-in, so setup checks for BitLocker or Device Encryption and recommends turning it on.

---

## User Stories

Actors: **owner** (the person who installs SoftLock and owns the computer), **bystander** (anyone physically at the computer while it's SoftLocked, including the owner before they unlock), **agent** (any software running in the owner's session: AI agents, test runners, automation, remote-control tools), **administrator** (an IT person or power user managing the computer), **contributor** (an open-source developer), **maintainer** (Henry).

Stories 80–89 were added on 15 November 2026. They're numbered after the original 79, so existing references stay valid.

### Locking

1. As an owner, I want to lock with a global hotkey (default Ctrl+Alt+Shift+L, configurable), so that locking is as fast as Win+L.
2. As an owner, I want to lock from the tray icon and from a button in the SoftLock app, so that I can lock without remembering a shortcut.
3. As an owner, I want the lock to take effect in under 500 ms, with physical input blocked first, so that nobody can slip a keystroke in while it's still arming.
4. As an owner, I want my currently focused window and its keyboard focus to stay exactly as they were when I lock, so that the agent I was watching keeps typing into the right place.
5. As an owner, I want a visible "SoftLock is on" state in the tray and the app, so that I always know whether the computer is protected.
6. As an owner, I want a "Lock for real" (hard lock) action in the tray and app that calls the actual Windows lock, so that I can use full OS protection when I don't need agents running.
7. As an owner, I want a lock request that fails partway through to fall back to the real Windows lock rather than leave me unlocked, so that asking for a lock always gets me a lock of some kind.

### Blocking physical input while software keeps running

8. As a bystander, I want every physical keystroke, mouse movement, click and scroll to do nothing except wake the unlock prompt, so that I can't interact with the desktop without the password.
9. As an agent, I want my injected keyboard and mouse input (SendInput and similar APIs) to reach the target windows unchanged while the computer is SoftLocked, so that my task continues.
10. As an agent, I want the mouse cursor to stay where I put it, without physical mouse movement nudging it, so that my click coordinates stay valid.
11. As an owner, I want touchscreens, pens and game controllers to be disabled while locked (configurable), so that inputs that don't go through the keyboard and mouse hooks can't become a loophole.
12. As an owner, I want the Windows key, Alt+Tab, Alt+Esc, Ctrl+Esc, Win+D, Win+R, media keys and every other shortcut to be blocked from the physical keyboard while locked, so that no shortcut reaches the shell.
13. As an owner, I want accessibility tools that can generate input or act on the UI (on-screen keyboard, touch keyboard, Voice Access, speech recognition) to be stopped while locked, so that a bystander can't use them as an input path around the lock.
14. As an owner, I want newly plugged-in keyboards, mice and USB "keystroke injector" gadgets to be blocked like any other physical device, so that plugging in hardware doesn't get around the lock.
15. As an owner, I want SoftLock to keep checking that its input blocking is still active, and to fail closed if Windows silently removes it, so that a dropped hook can never leave the computer open.
16. As an owner, I want the "last physical activity" time tracked separately from software input, so that idle-lock takeover works even while agents are busy typing.

### What the screen shows, to people and to agents

17. As a bystander, I want to see a clearly branded SoftLock lock screen on every monitor, with a clock and "Press any key to unlock", so that I know the computer is locked and by what.
18. As an owner, I want to choose opaque (default), dimmed or see-through display modes, so that I can hide my screen from onlookers or watch my agent's progress from across the room.
19. As an owner, I want an optional custom message (for example "Agent run in progress, don't touch. Back at 3pm"), so that people nearby know what's going on.
20. As an agent, I want screenshots, screen recordings and screen-sharing captures to show the real desktop without the SoftLock overlay, so that my vision and clicks match what's actually there.
21. As an owner, I want monitors that are connected, disconnected or rearranged while locked to be covered right away, so that a new monitor never shows an unlocked desktop.
22. As an owner, I want Windows notification banners suppressed while locked (configurable), so that message previews don't pop up above the lock screen.
23. As a bystander, I want the SoftLock screen to look clearly unlike the Windows sign-in screen, so that SoftLock is never mistaken for, or abused as, a fake Windows login.

### Ctrl+Alt+Del and other system escapes

24. As an owner, I want pressing Ctrl+Alt+Del while SoftLocked to show a screen with no Task Manager, Sign out, Switch user, Change password or Lock options, so that the only useful choice, Cancel, returns to SoftLock.
25. As an owner, I want SoftLock to be honest that Windows still shows a few system icons on the Ctrl+Alt+Del screen (power, network, accessibility), and to neutralize what it can, so that I understand exactly what remains possible.
26. As an owner, I want physical Win+L to be ignored while SoftLocked unless I've chosen otherwise, so that a bystander can't freeze my agents by switching to the real lock.
27. As an owner, I want UAC elevation prompts that appear while locked to require an administrator password rather than a simple "Yes" click, so that a bystander can't approve an elevation on the secure desktop.
28. As an owner, I want every system setting SoftLock changed during the lock to be restored exactly as it was (including values my administrator set), so that SoftLock never permanently changes my computer.
29. As an owner, I want those system settings recovered automatically at the next service start after a crash, forced restart or power loss, so that I'm never stuck with a disabled Task Manager.
30. As an administrator on an Enterprise or Education edition, I want SoftLock to optionally use Windows Keyboard Filter to suppress Ctrl+Alt+Del entirely, so that managed devices get the strongest available lock.
31. As an owner, I want the power button's action to follow my SoftLock setting while locked (default "do nothing"), so that a tap of the power button doesn't put the computer to sleep and stop my agents.

### Unlocking

32. As an owner, I want to unlock with Windows Hello face or fingerprint without touching the keyboard, so that unlocking is as quick as the real lock screen.
33. As an owner, I want to unlock with my Windows Hello PIN or my Windows account password (local or Microsoft account), so that I don't need another secret.
34. As an owner, I want an optional SoftLock-only PIN, stored only as a salted Argon2id hash, so that I can unlock quickly without exposing my account password.
35. As an owner, I want to choose which unlock methods are enabled and in what order they're offered, so that the unlock fits how I work.
36. As an owner, I want increasing delays after repeated failed unlock attempts, kicking in before Windows' own account-lockout threshold, so that guessing is impractical and I don't lock myself out of my Windows account.
37. As an owner, I want an unlock attempt to never pass physical input to anything except the SoftLock prompt or the Windows Hello dialog it opened, so that starting an unlock can't become a way to type into the desktop.
38. As an owner, I want unlocking to restore every system setting and re-enable every device before control returns to me, so that the computer is fully normal afterwards.
39. As a contributor, I want a documented unlock-method interface, so that new methods (for example a security key or a phone approval) can be added without touching the lock state machine.
40. As an owner, I want unlocking the real Windows lock (after a fall-back or a sleep) to also clear the SoftLock state, so that I don't have to authenticate twice.

### Fail closed

41. As an owner, I want SoftLock to switch to the real Windows lock within about one second if the lock screen process crashes, hangs or stops reporting that input blocking is active, so that a SoftLock failure never exposes my desktop.
42. As an owner, I want the lock screen process to switch to the real Windows lock itself if the background service disappears, so that killing the service is not a way around the lock.
43. As an owner, I want a notification after I sign back in explaining that SoftLock fell back to the Windows lock, when, and why, so that I understand why my agents stopped.
44. As an owner, I want the fail-closed behavior explained in plain words in the setup wizard, the user guide and the terms before I first lock, so that I know agents will pause if SoftLock ever fails.

### Keeping the computer awake and online

45. As an owner, I want the computer to stay awake (no idle sleep) while SoftLocked, so that agents and long tasks keep running.
46. As an owner on a Modern Standby laptop, I want SoftLock to keep the display powered (shown black or dimmed by the overlay) while locked if that's what it takes to avoid standby throttling, so that desktop apps aren't paused.
47. As an owner, I want network connectivity kept and monitored while locked, with status shown in the app afterwards, so that cloud agents and downloads don't silently drop.
48. As an owner, I want every power setting SoftLock changes to be restored on unlock, so that my battery life goes back to normal.
80. As an owner, I want SoftLock to switch to the real Windows lock and let the computer sleep when the battery falls below a level I choose (default 15%) while it isn't charging, so that a long agent run can't drain my laptop flat while it's held awake.
81. As an owner, I want SoftLock to do the same when the computer reports that it's overheating, and to tell me plainly if my computer doesn't report heat at all, so that keeping it awake never cooks it, for example in a bag.
82. As an owner, I want a notice after I sign back in that says the battery or heat floor ended the SoftLock, when it happened, and the reading, so that I know why my agents stopped.
83. As an owner, I want a lock I ask for while the battery is already below the floor to use the real Windows lock straight away, with the same notice, so that I always get a lock and never a drained battery.

### Pausing background apps (optional, off by default)

84. As an owner, I want to pick apps, or confirm a suggested list of busy background apps, that SoftLock pauses while locked, so that my battery and fans go to my agent instead of chat, music or launcher apps.
85. As an owner, I want v1 to ship both a gentle pause (Windows Efficiency mode, the default) and a strong pause (fully suspend, opt-in per app), and I want the wizard and settings to say that strong mode freezes the app so its network connections and timers may drop, so that I can choose.
86. As an owner, I want every paused app resumed when I unlock, when SoftLock falls back to the Windows lock, and after a crash or restart, so that an app is never left frozen.
87. As an agent, I want SoftLock never to pause my own process tree, the app that was in the foreground when the lock started, or an Efficiency-mode app I bring to the foreground, so that pausing never stalls my task.
88. As an owner, I want SoftLock never to pause Windows system processes, security software or SoftLock itself, whatever the config file says, so that the pause list can't be used to weaken the lock or my computer's protection.
89. As an owner, I want pausing to be a simple list of apps with no rules, schedules or conditions, so that it's easy to understand.

### Replacing the real lock (takeover, optional, off by default in v1)

49. As an owner, I want the first-run wizard to ask whether SoftLock should take over Win+L, and to explain the trade-off, so that I make an informed choice.
50. As an owner, I want an optional idle-lock takeover, so that SoftLock (not the Windows lock) engages after N minutes without physical activity.
51. As an owner, I want an optional lid-close takeover, so that closing my laptop lid SoftLocks it instead of sleeping, if my hardware supports staying awake with the lid closed.
52. As an owner, I want each takeover setting to be changeable later in one obvious place, and to be fully reverted to the Windows defaults it replaced, so that trying it is risk-free.

### Settings and configuration

53. As an owner, I want a clean settings app that follows Apple's Human Interface Guidelines for clarity and Google's Modern Web Guidance for the web code, so that SoftLock is pleasant and trustworthy to use.
54. As a power user or AI agent, I want every setting to live in one readable TOML file with comments and a published schema, so that I can version-control and script my setup.
55. As an owner, I want both the app and the file to be refused while the computer is SoftLocked or Windows-locked, so that nothing can weaken the lock while I'm away.
56. As an owner, I want invalid config edits rejected with clear field-level errors while the last good config stays active, so that a typo can't break or disable SoftLock.
57. As an owner, I want changes that weaken security (unlock methods, PIN, hardening, takeover) to need my confirmation with a Windows Hello or password check before they apply, so that an agent editing the file while I'm at the computer can't quietly weaken the lock.
58. As an owner, I want to preview exactly what the lock screen will look like, so that I can set it up without locking.

### Agent integration (framework only in v1)

59. As an agent running in my session, I want to request a SoftLock through an authenticated local interface, so that I can secure the computer when my owner asks me to.
60. As an owner, I want the local interface to have no unlock command at all, so that no agent or script can ever unlock the computer.
61. As an owner, I want every lock request to record its source (hotkey, tray, app, local API caller), so that I can see who locked the computer.

### Installation, the service, and updates

62. As an owner, I want an ordinary installer that lists in plain language what the background service does, why it needs admin rights, and what it changes while locked, so that I can trust what I'm installing.
63. As an owner, I want setup to check whether BitLocker or Device Encryption is on and to recommend turning it on if not, so that a forced restart can't expose my files.
64. As an owner, I want a first-run setup wizard covering hotkey, unlock methods, display mode, takeover options, keep-awake and the fail-closed disclosure, so that I'm protected in about two minutes.
65. As an owner, I want the app to show the service status (running, version, last self-check) at any time, so that I can confirm the enforcement is working.
66. As an owner, I want the in-app updater to check a signed manifest, verify the download's signature, and install only while unlocked, so that updates are convenient and can't be swapped for something malicious.
67. As a maintainer, I want stable and dev update channels, so that I can ship test builds to myself without affecting other users.
68. As an owner, I want the updater to refuse downgrades unless I explicitly choose one, so that an attacker can't roll me back to a vulnerable version.
69. As an owner, I want a clean uninstall that restores every setting and refuses to run while SoftLocked, so that removing SoftLock leaves nothing behind.
70. As an owner, I want SoftLock to make no network connections except update checks (which I can turn off), and to send no telemetry, so that my privacy is respected.

### Trust, transparency, and the project

71. As an owner, I want a user guide that says plainly what SoftLock protects against (people at the keyboard) and what it doesn't (malicious software already running as me, hardware attacks, power-off without disk encryption), so that I'm never misled about the protection.
72. As an owner, I want local logs of lock, unlock and fall-back events, with no secrets in them, and a one-click diagnostics export, so that I can report problems.
73. As a contributor, I want the platform-specific code behind clear interfaces, so that I can add macOS or Linux support without rewriting the core.
74. As a contributor, I want most behavior covered by automated tests that run on GitHub Actions, so that I can change code confidently without special hardware.
75. As a maintainer, I want a short manual release checklist for the few things that need real hardware (physical keyboard, Ctrl+Alt+Del, lid, touch, Windows Hello), so that releases are honestly verified.
76. As a maintainer, I want the public repository to contain only the free core, with no planned paid features, so that the open-core split stays clean.
77. As a contributor, I want a clear contribution policy (DCO sign-off once outside pull requests start), so that the project's code origin stays clean.
78. As a future macOS user, I want the same lock behavior on macOS 11+ on Apple Silicon and Intel, so that SoftLock works on all my computers.
79. As a future Linux user, I want the same lock behavior on mainstream distributions under Wayland and X11, so that SoftLock works on my Linux workstation.

### Connected displays and remote views (16 November 2026)

Stories 90–95 are Henry's Decision 20. They sit after 80–89 so the earlier numbers stay valid. They apply on Windows 11, Windows 10 (version 2004 or later), Linux under X11 and under Wayland, and the macOS Universal 2 build, both the `x86_64` slice and the `arm64` slice. AirPlay and Sidecar are the macOS names for a wireless or iPad display. On Windows and Linux the same requirement is every person-visible display, including a USB display and a remote view.

90. As an owner, I want a lock that starts while a remote session is already up (Remote Desktop, Screen Sharing, VNC, Chrome Remote Desktop, and a remote view of the same kind) to cover that remote view with the same SoftLock screen the console shows.
91. As a person connecting remotely to a machine that is already SoftLocked, I want to see the SoftLock lock screen, so that the remote view is not an unlocked desktop.
92. As an owner, I want unlock from a remote session to be the same Broker flow as at the console: the SoftLock PIN, Windows Hello or Touch ID where that remote stack supports it, and otherwise the PIN or the account password, with no weaker path.
93. As a bystander on a remote session, I want keys, clicks, and touch from that session, before unlock, to reach only the unlock UI, so that the remote view is not a way into the desktop.
94. As an owner, I want a crash or any other failure while a remote session is covered to follow invariant 3 in Decision 5: `OsLocked`, or `FailingClosed` and still guarded when the OS lock call itself fails.

Story 93 is the behavior Henry asked for. TeamViewer, AnyDesk, and Chrome Remote Desktop inject input the same way an agent does, and Mac Screen Sharing input arrives as posted events, so the mechanism is the open remote-input entry in Decision 20. Story 94 is invariant 3 plus the known gap in section 7 until ticket 13. Neither story adds a classification the hook or the tap does not have.
95. As an owner, I want every person-visible display covered while locked, including one that is added, removed, mirrored, rescaled, or whose DPI changes, an AirPlay display, a Sidecar iPad, a DisplayLink or other USB display, and a closed laptop lid that leaves an external display awake.

---
## Implementation Decisions

### 1. Threat model (this defines "no loopholes")

| Adversary | v1 position |
|---|---|
| **T1: A person physically at the computer** (keyboard, mouse, touch, pen, gamepad, USB devices, voice, power and lid buttons) | **In scope, and must be loophole-free.** They can't get to the desktop or any running app without an enabled unlock method. Their best options are a forced power-off or reboot (accepted, because it lands at the real Windows sign-in, which is why disk encryption is recommended) or disrupting the session (for example unplugging the network). Both are denial of service, not access. |
| **T2: Software running as the owner** (agents, scripts, remote-control tools) | **Trusted to run, not trusted to unlock.** Software input from an agent in the session passes through by design. SoftLock's own interfaces offer no unlock path. SoftLock doesn't claim to stop *malicious* code running as the owner, because such code already has the owner's files and session. A remote view is a bystander for the picture (Decision 20). TeamViewer, AnyDesk, and Chrome Remote Desktop inject input the same way an agent does, and Mac Screen Sharing input arrives as posted events. The hook and the tap cannot tell that input apart from an agent's. Whether it is blocked is the open remote-input entry in Decision 20. This row does not claim a mechanism that does that today. |
| **T3: Hardware and pre-boot attacks** (DMA, cold boot, keyloggers, booting another OS) | Out of scope. Covered by BitLocker or Device Encryption and firmware settings. |

**Physical vs. software input, defined:** on Windows, an input event is *software* if the low-level hook reports it as injected (`LLKHF_INJECTED` for keyboard, `LLMHF_INJECTED` for mouse) and *physical* otherwise. Some consequences:
- Input from RDP sessions, Hyper-V basic sessions, KVM-over-IP and other virtual-HID tools looks physical inside the guest, so it's blocked. Ticket 37 records the hook's injected flag for a console key, for an RDP key, and for an RDP shadow key. The terminal page cited there treats the console and a remote terminal as different devices and does not document the flag. TeamViewer, AnyDesk, and Chrome Remote Desktop are not this bullet. Their input is injected the same way an agent's is, and the open remote-input entry in Decision 20 is where that is decided.
- Some accessibility tools inject input or act through UI Automation without any input event at all. That's why they're stopped while locked (Hardening section).

### 2. Windows limits we design around (stated honestly)

- **Ctrl+Alt+Del (the Secure Attention Sequence) can't be intercepted or swallowed** by any user-mode code. Winlogon handles it and switches to the secure Winlogon desktop. `BlockInput` is undone by it. SoftLock therefore doesn't try to block it. Instead, it removes the options on that screen through policy values, so **Cancel** is the only meaningful action, and Cancel returns to the Default desktop, where SoftLock is still showing. The power, network and accessibility icons on that screen aren't all removable by policy. Power is accepted (a restart lands at the real sign-in), and the accessibility tools are stopped by hardening. Keyboard Filter (Enterprise, Education and IoT only) can suppress the sequence itself and is used when available and enabled.
- **The secure desktop is unreachable.** No SoftLock process can draw on or read input from the Winlogon desktop, and neither can agents. That's why SoftLock never calls the real lock except for hard lock and fail-closed.
- **UAC prompts appear on the secure desktop** (by default), outside SoftLock's control. While locked, SoftLock changes the elevation prompt for administrators to require credentials, and sets standard-user prompts to auto-deny, so a bystander can't approve them with one click. Agent elevations will wait or time out while locked, which is expected and documented.
- **Win+L is handled by the system, not delivered to low-level hooks** in the normal case. The `DisableLockWorkstation` policy is the only reliable way to neutralize it. That same policy also makes `LockWorkStation()` fail, so every code path that needs the real lock (hard lock, fail-closed) **clears that policy first** or uses the session-disconnect primitive (section 7).
- **No uiAccess in v1.** uiAccess windows sit in a higher z-band, but they need an Authenticode-signed binary in a secure location. v1 is unsigned, so the overlay is a normal topmost window. Shell surfaces (Start menu, Task View, some toasts, the secure-desktop dim background) can draw above it if an agent opens them. Physical input to them is still blocked. The remaining risk is that their contents are visible on screen (see Risks).
- **Low-level hooks are removed silently** if a callback exceeds `LowLevelHooksTimeout`. The hook therefore runs on a dedicated high-priority thread, does no I/O and no allocation in the callback, and is checked continuously for liveness.
- **Capture exclusion** (`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`) needs Windows 10 version 2004 (build 19041) or later. That sets the minimum supported version. DWM applies it, so BitBlt/GDI capture, DXGI Desktop Duplication and Windows.Graphics.Capture all skip the overlay. The same affinity is what can hide the overlay from a remote view. That conflict is Decision 20, and it is open.

### 3. Architecture and modules

SoftLock is a Rust workspace with thin native layers. Each module is deep: a small interface hiding a lot of behavior.

| Module | Runs as | Responsibility | Platform-specific? |
|---|---|---|---|
| **Core** (`softlock-core` library) | linked into the others | Lock state machine, config model, schema and validation, IPC protocol types, unlock-method interface, backoff policy, policy-journal model, heartbeat rules. **No OS calls.** | No |
| **Platform interfaces** (in Core) | n/a | Traits that every OS backend implements (section 4) | No (definitions only) |
| **Windows backend** | linked into Service, Lock Host, Broker | Implements the platform traits with `windows-rs` and Win32 | Windows |
| **Service** (`softlock-service`) | Windows service, LocalSystem, automatic start | The single authority. Owns the state machine, IPC server, policy apply/revert with journal, device hardening, power requests, battery and heat floor, background-app pausing (with the owner's token), watchdog, config authority, update installation, BitLocker detection | Thin Windows shell around Core |
| **Lock Host** (`softlock-lockhost`) | Started by the Service into the owner's interactive session on `winsta0\default`, with a SYSTEM token (see note) | Native overlay windows (one per monitor), the input guard (low-level hooks), the unlock prompt UI, the heartbeat. Pre-spawned and idle at sign-in, so locking is instant | Native Win32 + Direct2D/DirectComposition |
| **Unlock Broker** (`softlock-broker`) | Started on demand by the Service as the owner (user token) | Calls Windows Hello (`UserConsentVerifier`), which has to run in the user's context. Reports to the Service over a private inherited pipe, then exits | Windows |
| **App** (`softlock-app`) | Tauri 2, the owner, medium integrity, starts at sign-in | Tray icon, global hotkey registration, settings UI, setup wizard, lock preview, status, updater UI, user-editable config file watcher | Mostly portable. Tray and hotkey via Tauri plugins |
| **Installer** | n/a | WiX-built MSI. Installs the Service (MSI `ServiceInstall`), the binaries and the App autostart, and shows the "what the service does" page. MSIX isn't used, because unsigned MSIX packages can't be installed outside developer mode | Windows |

**Why the Lock Host runs as SYSTEM in the user's session:** a low-level hook only sees input headed to windows at or below its own integrity level. A hook at medium integrity misses input to elevated windows, which would be a loophole. Running the guard as SYSTEM in the session covers every window on the Default desktop, and UIPI stops lower-integrity processes (agents) from sending messages to or tampering with it. The Unlock Broker exists because Windows Hello must be called in the user's context. *This choice is confirmed by prototype P2. The fallback is to use the owner's elevated linked token for administrators.*

**The overlay never takes focus or mouse input.** Its windows use `WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` plus `WDA_EXCLUDEFROMCAPTURE`. Agents' injected clicks fall through to the real windows underneath, and keyboard focus stays where it was. The overlay receives physical keystrokes only from its own input guard, which swallows them in the hook and forwards them in-process to the unlock prompt (translated with `ToUnicodeEx` and the owner's active keyboard layout). The overlay re-asserts topmost on a timer and on `WM_WINDOWPOSCHANGED`, and re-covers monitors on `WM_DISPLAYCHANGE` and on `WM_DPICHANGED` (a DPI or scale change). One window per person-visible display, including a monitor on a remote terminal. Whether that remote window keeps `WDA_EXCLUDEFROMCAPTURE` is Decision 20 and is open. IME composition isn't supported in the unlock field in v1.

**Input guard rules (on the Lock Host's dedicated hook thread):**
- Injected keyboard or mouse event: pass to the next hook.
- Physical keyboard event: swallow. If the prompt is idle, wake it. If the prompt is open, forward the key to it.
- Physical mouse event: swallow. A click or wheel wakes the prompt. Movement does nothing (the cursor isn't moved).
- During **credential passthrough** (a Windows Hello dialog opened by SoftLock is in the foreground): pass physical keyboard events only while the foreground window belongs to the system credential UI process (image path and signature checked once per dialog). System chords (Win, Alt+Tab, Alt+Esc, Ctrl+Esc, Ctrl+Shift+Esc) are always swallowed. Passthrough ends when the dialog closes or after 60 s.
- Records the time of the last physical input, used for idle-lock takeover.
- Hook liveness: every 250 ms the guard confirms its hook callback has run recently. If it hasn't seen any event, it injects a no-op marker event and checks that the callback saw it. Failure is reported in the heartbeat, which leads to fail-closed.

### 4. Platform abstraction (designed now, implemented for Windows in v1)

The Core depends only on these traits. Windows implements all of them in v1. macOS and Linux implement them in later phases. A `Fake` implementation in Core drives the core test seam.

```rust
pub trait InputGuard {            // in the Lock Host
    fn arm(&mut self, sink: PhysicalInputSink) -> Result<(), GuardError>;
    fn disarm(&mut self);
    fn set_passthrough(&mut self, target: Option<PassthroughTarget>);
    fn health(&self) -> GuardHealth;          // { hook_alive, last_physical_input }
}
pub trait Overlay {               // in the Lock Host
    fn show(&mut self, look: &LockScreenLook) -> Result<(), OverlayError>;
    fn update(&mut self, view: &PromptView);
    fn hide(&mut self);
    fn health(&self) -> OverlayHealth;        // { covered_monitors, capture_excluded, topmost }
}
pub trait SessionControl {        // in the Service
    fn os_lock(&self, session: SessionId) -> Result<(), OsLockError>; // fail-closed and hard lock
    fn session_events(&self) -> Receiver<SessionEvent>; // OsLocked, OsUnlocked, Logon, Logoff, ...
}
pub trait Hardening {             // in the Service; every apply is journaled first
    fn plan(&self, cfg: &HardeningConfig, session: &SessionInfo) -> HardeningPlan;
    fn apply(&self, plan: &HardeningPlan, journal: &mut Journal) -> Result<(), HardenError>;
    fn revert(&self, journal: &Journal) -> Result<(), HardenError>;
}
pub trait Power {
    fn hold(&self, req: AwakeRequest) -> AwakeGuard;
    fn capabilities(&self) -> PowerCaps;
    fn battery(&self) -> BatteryReading;      // { has_battery, charging, on_ac, percent: Option<u8> }
    fn thermal(&self) -> ThermalReading;      // { sources: Vec<ThermalSource>, throttling: bool, near_critical: bool }
    fn sleep_now(&self) -> Result<(), PowerError>; // only after a battery or heat floor fall-back (section 10)
}
pub trait AppPause {              // in the Service; acts with the owner's token; every pause is journaled first
    fn candidates(&self, session: &SessionInfo) -> Vec<RunningApp>;   // picker list, denylist already applied
    fn pause(&self, app: &PausedApp, protect: &ProtectedSet, journal: &mut Journal) -> Vec<PauseOutcome>;
    fn resume(&self, journal: &Journal) -> Vec<ResumeOutcome>;          // idempotent; skips a PID whose start time changed
}
pub trait Credentials { fn verify_password(&self, user: &UserId, pw: &Secret) -> Verdict; }
pub trait EncryptionStatus { fn system_volume(&self) -> DiskEncryption; }
```

### 5. Lock state machine (owned by the Service)

```rust
enum LockState {
    Unlocked,
    Arming   { source: LockSource, step: ArmStep },           // journal -> guard -> overlay -> hardening -> power
    Locked   { source: LockSource, prompt: PromptState },
    Unlocking{ step: DisarmStep },                            // hardening revert -> power release -> resume paused apps -> overlay hide -> guard disarm
    FailingClosed { reason: FailReason },                     // calling the real OS lock
    OsLocked { after_fail: Option<FailReason> },              // the Windows lock screen is up
    Recovering,                                               // service start with a non-empty journal
}
enum PromptState { Idle, Open { method: MethodId }, Passthrough { method: MethodId },
                   Verifying { method: MethodId }, Backoff { until: Instant, failures: u32 } }

// Transitions (anything not listed is rejected and logged):
// Unlocked   --LockRequested(src)-->            Arming
// Arming     --AllStepsOk-->                    Locked
// Arming     --AnyStepFailed-->                 FailingClosed      // never back to Unlocked
// Arming     --BatteryBelowFloor-->             FailingClosed      // a lock asked for on low battery is a real lock (section 10)
// Locked     (on entry) starts the background pause pass (section 18); its results never cause a transition
// Locked     --Verified{via: LockHostChannel}--> Unlocking          // the ONLY way out of Locked
// Locked     --HeartbeatMissed|HostExited|GuardUnhealthy|OverlayUnhealthy--> FailingClosed
// Locked     --OsLockObserved-->                OsLocked
// Locked     --BatteryFloor|HeatFloor-->        FailingClosed      // section 10; the reading is kept for the notice
// Unlocking  --Done-->                          Unlocked
// Unlocking  --RevertFailed-->                  OsLocked (after os_lock) + journal kept for retry
// FailingClosed --OsLockConfirmed-->            OsLocked
// FailingClosed --Verified{nonce}, and only while the OS lock call failed--> Unlocking
//                 // same check as Locked: Broker nonce, eq_ct. A bad or missing nonce stays FailingClosed.
//                 // While the OS lock is still available this transition is rejected.
// OsLocked   --OsUnlocked, revert succeeds-->   Unlocked (journal reverted, user notified if after_fail)
// OsLocked   --OsUnlocked, revert still failing--> OsLocked (guard and overlay disarmed, journal kept, Repair notice)
// Recovering --SessionActiveAndJournalSaysLocked--> FailingClosed
// Recovering --NoActiveLock-->                  Unlocked (journal reverted)
// Any        --ServiceStopRequested while Locked--> FailingClosed, then stop
```

Invariants (each one is a test):
1. `Locked → Unlocking` happens only on a `Verified` verdict from the Broker this service launched, checked against that launch's CSPRNG nonce, or on the credential verifier returning `Verified` for a `SubmitSecret`. `FailingClosed → Unlocking` happens only on that same nonce check, and only after the OS lock call has failed. A bad or missing nonce stays `FailingClosed`. The Lock Host does not verify and has no verdict that unlocks. No public IPC verb produces it.
2. Every hardening change is in the persisted journal **before** it's applied.
3. Any failure while `Arming` or `Locked` ends in `OsLocked`, never `Unlocked`, except when the OS lock call itself fails. Then `FailingClosed` stays guarded until a matching `Verified` nonce moves it to `Unlocking`. It does not unlock while the OS lock is still available.
4. Config writes are rejected in every state except `Unlocked` with the session not OS-locked.
5. After `Unlocked` is reached, the journal is empty and the system settings equal the values recorded before the lock.
6. Every app SoftLock paused is resumed before `Unlocked` is reached and after the OS lock is confirmed in `FailingClosed`. A pause or resume failure never changes the lock state.
7. Keep-awake requests are released only after the OS lock is confirmed, never before, so a floor trip can't put an unlocked desktop to sleep.

### 6. IPC contract

- **Transport:** one named pipe owned by the Service. Its DACL allows SYSTEM and the interactive user. Messages are length-prefixed (u32, little-endian) UTF-8 JSON, which is easy to debug, and the volume is tiny. A frame whose length prefix is above 64 KiB is rejected before the body is allocated. Every message has `{ "v": 1, "id": <u64>, "kind": ... }`. An unknown major version is rejected with `Unsupported`. Unknown fields are rejected as malformed. The PIN requests use the fixed kinds `SetPin` and `ClearPin`.
- **Named-pipe hardening, required by the service and lock-host tickets:** the server is created with `FILE_FLAG_FIRST_PIPE_INSTANCE` and an explicit DACL limited to SYSTEM and the interactive user. Clients verify the server's identity before sending a secret or trusting a reply. Without the first-instance flag, another process can create the pipe name first.
- **Caller identity:** on connect, the Service reads the client's PID and token (`GetNamedPipeClientProcessId`, `ImpersonateNamedPipeClient`) and requires the client's session to be the active console session. The role comes from how the client connected:
  - **Lock Host and Broker:** private, launch-time pipes (inherited handles plus a one-time 256-bit nonce). They never use the public pipe.
  - **App:** the public pipe. Role `App`, identified by its image path inside the install directory.
  - **Any other process of the console user:** the public pipe. Role `LockClient`.

| Request | Allowed roles | States | Result |
|---|---|---|---|
| `Hello { client_version }` | all | all | `{ service_version, state, capabilities }` |
| `GetStatus` | all | all | state, lock source, guard and overlay health, last fall-back, battery reading and floor, heat-check sources (or "unavailable"), paused and skipped apps |
| `Lock { source, note? }` | App, LockClient | Unlocked | `Accepted` or `AlreadyLocked` |
| `HardLock` | App | Unlocked, Locked\* | calls the real OS lock |
| `GetConfig` | App, LockClient | all | effective config plus revision |
| `ValidateConfig { toml }` | App, LockClient | all | `Ok` or `Invalid { errors[] }` |
| `PutConfig { toml, base_revision }` | App | Unlocked only | `Applied`, `NeedsConfirmation { fields[] }`, `Invalid`, `Conflict`, `Locked` |
| `ConfirmSensitiveChange { token }` | App | Unlocked only | runs a Hello or password check through the Broker, then applies |
| `SetSoftLockPin { pin }` / `ClearSoftLockPin` | App | Unlocked only, after a confirmation check | `Ok` |
| `ListRunningApps` | App | Unlocked only | apps with a visible window in the owner's session for the pause picker: name, path, icon, recent CPU, `suggested`, or `not_pausable { reason }` |
| `Subscribe` | App | all | event stream: `StateChanged`, `ConfigChanged`, `FellBackToOsLock { reason, at }` (reasons include `BatteryFloor { percent }` and `HeatFloor { source }`), `PauseSummary { paused, skipped[], resume_failures[] }`, `UpdateAvailable` |
| `CheckUpdate` / `InstallUpdate { version }` | App | Unlocked only | `UpToDate`, `Available { version, notes }`, or `Installing` |

\* `HardLock` while Locked is accepted only from the App. Every other role is rejected and logged. It fails closed with its own reason. While Unlocked, the App's `HardLock` calls the OS lock and the state stays Unlocked.

**There is no `Unlock` request on the public pipe.** This is the lock-only agent framework: in a later release, the CLI and remote triggers will be thin `LockClient`s on top of `Lock`.

**Lock Host private channel:** `HostReady { monitors }`, `Heartbeat { seq, guard: GuardHealth, overlay: OverlayHealth }` every 250 ms, `PromptOpened { method }`, `SubmitSecret { method, secret }` (zeroized after use, never logged), `RequestHello` (the Service launches the Broker), and from the Service `Arm`, `Disarm`, `Show(look)`, `PromptResult { verdict, backoff_until? }`.

### 7. Fail closed and the watchdog

- **Detection (Service side):**
  - The Service holds the Lock Host's process handle and waits on it, so a crash is detected immediately.
  - It requires heartbeats, so a hang is detected. Three missed 250 ms heartbeats (≤ 1 s) count as a failure.
  - It treats an unhealthy guard or overlay in a heartbeat as a failure.
  - It treats an arming step that fails or takes longer than 2 s as a failure.
- **Detection (Lock Host side):** if the private pipe to the Service breaks or the Service stops answering for 1 s, the Lock Host triggers the OS lock itself.
- **OS-lock primitive, in order:**
  1. Clear `DisableLockWorkstation` for the session user.
  2. If the Lock Host is alive, it calls `LockWorkStation()`.
  3. Otherwise the Service calls `WTSDisconnectSession` on the console session from session 0. Like the lock, this shows the Windows sign-in screen, and the owner's apps keep running.
  4. The Service confirms through session notifications (`WTS_SESSION_LOCK` / `WTS_CONSOLE_DISCONNECT`).
  The Lock Host's own fall-back is `WTSDisconnectSession` on its session, which doesn't depend on the policy. (*Prototype P4 checks that both work while the hardening policies are applied.*)
- **After fall-back:** policies stay applied until the OS lock is confirmed, so there's no window of reverted policies on an open desktop. They're then reverted from the journal, which also releases the power requests and resumes paused apps (section 18). A battery or heat floor trip uses this same path, then lets the computer sleep (section 10). When the owner signs back in, the App shows "SoftLock switched to the Windows lock at <time> because <reason>."
- **Service lifecycle:**
  - SCM recovery restarts the Service on crash.
  - While Locked, the Service doesn't accept `SERVICE_CONTROL_STOP`. An uninstall or stop request leads to fail-closed first.
  - If the Service process is killed outright (an administrator-level action), the Lock Host's fall-back covers it.
- **Known gap, until ticket 13:** a dead Lock Host, and the case where both OS locks fail and a panic then runs `emergency()` (which releases the session and drops the hooks), still rely on the ticket-13 Service watchdog. That watchdog re-locks if the Lock Host dies. See issue #13 and PR #72. The callback-panic fix does not change `emergency()`.

### 8. Hardening applied while locked (Windows)

Every item is journaled first, applied on `Arming`, reverted on `Unlocking`, and re-asserted every 5 s plus on registry change notifications (`RegNotifyChangeKeyValue`), so a domain Group Policy refresh in the middle of a lock can't reopen a hole. Per-user values are written under the locked user's hive (`HKEY_USERS\<SID>`). If the hive isn't loaded during recovery, it's loaded with `RegLoadKey`.

| Item | Mechanism | Purpose |
|---|---|---|
| Task Manager | `DisableTaskMgr = 1` (user System policies) | Removes Task Manager from Ctrl+Alt+Del and blocks Ctrl+Shift+Esc |
| Lock | `DisableLockWorkstation = 1` (user) | Removes Lock and neutralizes physical Win+L |
| Change password | `DisableChangePassword = 1` (user) | Removes the item |
| Sign out | `NoLogoff = 1` (user Explorer policies) | Removes Sign out from Ctrl+Alt+Del |
| Switch user | `HideFastUserSwitching = 1` (machine System policies) | Removes Switch user |
| Shut down / restart | `NoClose = 1` (user Explorer policies), effect on the Ctrl+Alt+Del power icon to be verified (P1) | Reduces casual restarts. Restarting is accepted under T1 anyway |
| UAC | `ConsentPromptBehaviorAdmin = 1` (credentials on secure desktop), `ConsentPromptBehaviorUser = 0` (auto-deny) | No one-click elevation by a bystander |
| Touch, pen, game controllers | Disable the matching device instances through SetupAPI (`DICS_DISABLE`), journaled by instance ID | These inputs bypass the low-level keyboard and mouse hooks |
| Accessibility input tools | Terminate and block-relaunch (with a WMI process-start watch) the on-screen keyboard, touch keyboard, Voice Access and speech recognition while locked. This is a **fixed security list, not a user allowlist** | They generate injected input or act through UI Automation |
| Notifications | Turn off toast banners for the session user while locked | Stops previews drawing above the overlay |
| Power and lid buttons | Set the active scheme's power-button action (and, with lid takeover, the lid action) to "do nothing" with `PowerWriteACValueIndex` / `PowerWriteDCValueIndex` | Taps don't sleep the computer |
| Keyboard Filter (optional, **deferred out of v1**, see section 17) | On Enterprise, Education or IoT with the feature enabled, block Ctrl+Alt+Del through its WMI provider while locked | The strongest available Ctrl+Alt+Del suppression |

The journal is a JSON document in a directory that only SYSTEM and Administrators can write. It holds, for each change, the target, the previous value (or "absent"), and the new value. It's written and flushed before each change. Revert restores the previous values exactly, preserving administrator-set policy, and is idempotent. App pauses (section 18) use the same journal: process ID, process start time, image path, mode, and the previous throttling state and priority class.

### 9. Unlock methods

```rust
pub trait UnlockMethod: Send + Sync {
    fn id(&self) -> MethodId;                                 // "windows_hello", "os_password", "softlock_pin", ...
    fn availability(&self, ctx: &UnlockCtx) -> Availability;  // Ready | NotEnrolled | Unsupported
    fn input_kind(&self) -> InputKind;                        // SecretEntry | SystemDialog | NoInput
    fn verify(&self, ctx: &UnlockCtx, attempt: Attempt) -> Verdict; // runs in the Service or the Broker, never in the Lock Host
}
pub enum Verdict { Verified { user: UserId }, Rejected { reason: RejectReason }, Cancelled }
```

| Method | Input | Verified by | Notes |
|---|---|---|---|
| `windows_hello` | `SystemDialog` (face or fingerprint need no keys. PIN uses credential passthrough) | The Broker calls `UserConsentVerifier.RequestVerificationAsync` in the user context | The result comes back over the Broker's inherited pipe. Available only if Hello is set up |
| `os_password` | `SecretEntry` in the SoftLock prompt | The Service calls `LogonUserW` (network logon, then the token is closed right away) for the session user, including `MicrosoftAccount\<email>` | Failures count toward Windows' account-lockout counter, so SoftLock's backoff starts first |
| `softlock_pin` | `SecretEntry` | The Service checks Argon2id (m = 64 MiB, t = 3, p = 1, 16-byte random salt), using a constant-time comparison | The hash is stored in the Service's SYSTEM-only secrets store, **never in the user-editable config** |

- Backoff: no delay for attempts 1–4. From attempt 5 the delay is 30 s, doubling each time up to 15 min. It's counted per lock session, persisted in the journal, and resets on success.
- Methods are registered at compile time in v1. There's **no dynamic plug-in loading into SYSTEM processes**. A later Windows credential provider (C++) or out-of-process plug-ins will implement the same trait over a signed, versioned boundary.
- If the OS lock appears and is unlocked while SoftLocked, Windows has authenticated the owner, so SoftLock moves to `Unlocked`.

### 10. Keep awake and online, with a battery and heat floor

- While Locked, the Service holds `PowerCreateRequest`/`PowerSetRequest(PowerRequestSystemRequired)`. The Lock Host also calls `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` as a belt-and-braces measure.
- **Modern Standby** is detected with `GetPwrCapabilities` (`AoAc`). On those devices, turning the display off can push the system into standby, where the Desktop Activity Moderator throttles desktop apps. With `keep_display_on = "auto"`, SoftLock also holds `PowerRequestDisplayRequired`, so the panel stays on and shows the opaque or dim overlay. *Prototype P5 measures whether this is enough with the lid closed.*
- Network: no aggressive changes in v1. The Service watches connectivity with `INetworkListManager` events, logs drops and reconnects, and reports them in status. "Keep online" amounts to preventing sleep plus reporting.
- All requests are released on unlock, and all power-scheme edits are journaled and reverted.

**Battery floor.** It's checked only while Locked with keep-awake on.
- **Readings:** the Service registers for `GUID_BATTERY_PERCENTAGE_REMAINING` and `GUID_ACDC_POWER_SOURCE` with `RegisterPowerSettingNotification(..., DEVICE_NOTIFY_SERVICE_HANDLE)`, which arrive as `SERVICE_CONTROL_POWEREVENT` / `PBT_POWERSETTINGCHANGE`. It also polls `GetSystemPowerStatus` every 30 s as a backstop.
- **Rule:** the floor trips when the computer has a battery (`BatteryFlag` isn't 128), isn't charging (`ACLineStatus` is 0, or the charging flag 8 is clear), and `BatteryLifePercent` is below `battery_floor_percent` on two readings at least 30 s apart. The default is 15 and the range is 5–50. A percent of 255 (unknown) never trips it; it's logged and shown in status. Desktops without a battery never trip it.
- **Locking on low battery:** if a lock is requested while the floor would already trip, Arming goes straight to `FailingClosed` (the real Windows lock) with the battery notice.
- Windows' own low- and critical-battery actions still apply underneath, and SoftLock doesn't change them.

**Heat floor (best effort).** Windows has no vendor-neutral, reliable temperature API on consumer laptops. Thermal zones depend on the firmware: many laptops expose none, some report a fixed or motherboard value, and the CPU package temperature needs a kernel driver, which SoftLock won't ship. With `heat_check = "auto"`, the Service (as SYSTEM) reads whatever the computer exposes, every 10 s while Locked.

**Power policy, logged only.** These are not heat, and they never trip the floor. `CoolingMode` from `CallNtPowerInformation(SystemPowerInformation)` is the cooling policy: `PO_TZ_PASSIVE` means slow the CPU before spinning the fans, `PO_TZ_ACTIVE` means fans first, and `PO_TZ_INVALID_MODE` means Windows has no cooling-policy mode. Many laptops default to Passive on battery. Likewise `MhzLimit` below `MaxMhz`, from `CallNtPowerInformation(ProcessorInformation)`, is usually the power-plan cap, not thermal throttling. SoftLock logs both and shows them in status. Neither is a heat signal.

**Heat signals** (the only readings that can trip the floor):
1. The "Thermal Zone Information" performance counters, through PDH: `Throttle Reasons` bit 0 (value 0x1, thermal), or `% Passive Limit` below 100.
2. WMI `root\WMI` `MSAcpi_ThermalZoneTemperature`: `CurrentTemperature` within 5 K of `CriticalTripPoint`, both in tenths of a kelvin. The class is admin-only, which the Service is.
3. The battery's own temperature from `IOCTL_BATTERY_QUERY_INFORMATION` (`BatteryTemperature`), when the battery reports it: 60 °C or more.

- **Plausibility:** a temperature whose value stays identical for 10 minutes, or falls outside 250–400 K, is treated as fabricated and ignored.
- **Trip:** the ACPI zone and the battery temperature (signals 2 and 3) trip at once. The thermal-zone counters (signal 1) trip after the condition has lasted 2 minutes without a break. Cooling mode and the MHz cap never trip it.
- **No usable source:** if none of the heat signals is available, status, settings and the wizard say "This computer doesn't report its temperature to Windows, so SoftLock can't watch it. Don't leave it running in a bag." A cooling mode or an MHz cap is not a heat reading. The firmware's own thermal shutdown stays the real backstop.
- The heat check can be turned off. It isn't a security setting.

**What a floor trip does:**
1. `FailingClosed { reason: BatteryFloor { percent } | HeatFloor { source } }` runs the normal OS-lock primitive (section 7). The power requests stay held until the OS lock is confirmed, so the computer never sleeps into an unlocked desktop.
2. After confirmation, the journal revert releases the power requests and the Lock Host's execution state, restores the power-button action, and resumes paused apps (section 18).
3. The computer is allowed to sleep. On Modern Standby devices, the Windows lock screen turns the display off after its own timeout and standby follows. On S3 devices, if the lid is closed or the reason is heat, the Service calls `SetSuspendState(FALSE, FALSE, FALSE)`. Otherwise the normal idle-sleep timer applies.
4. It's logged. After sign-in, the App shows "SoftLock switched to the Windows lock at <time> because the battery was at 14%" or "...because the computer reported overheating (<source>)."

References: [GetSystemPowerStatus](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getsystempowerstatus), [SYSTEM_POWER_STATUS](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-system_power_status), [RegisterPowerSettingNotification](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerpowersettingnotification), [power setting GUIDs](https://learn.microsoft.com/en-us/windows/win32/power/power-setting-guids), [SYSTEM_POWER_INFORMATION (CoolingMode)](https://learn.microsoft.com/en-us/windows/win32/power/system-power-information-str), [PROCESSOR_POWER_INFORMATION (MhzLimit)](https://learn.microsoft.com/en-us/windows/win32/power/processor-power-information-str), [thermal zone performance counters](https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/examples--requirements-and-diagnostics), [BATTERY_QUERY_INFORMATION_LEVEL (BatteryTemperature)](https://learn.microsoft.com/en-us/windows/win32/power/battery-query-information-str), [Win32_TemperatureProbe (CurrentReading not populated)](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-temperatureprobe), [SetSuspendState](https://learn.microsoft.com/en-us/windows/win32/api/powrprof/nf-powrprof-setsuspendstate).

### 11. Takeover settings (off by default in v1)

| Setting | When on | When turned off |
|---|---|---|
| `win_l` | `DisableLockWorkstation` stays set while unlocked too. The Lock Host keeps a low-level keyboard hook (pass-through only) that detects the physical Win+L chord and sends `Lock` | Policy restored to its journaled value |
| `idle_lock` | Windows' idle-lock sources are recorded and turned off: the secure screensaver, the `InactivityTimeoutSecs` machine policy, sign-in required on wake, and Dynamic Lock. SoftLock locks after `idle_minutes` with no **physical** input | All restored |
| `lid_close` | The lid action is set to "do nothing" on the chosen power sources. `GUID_LIDSWITCH_STATE_CHANGE` triggers `Lock` | Lid action restored |

After a fail-closed fall-back (or a hard lock) with `win_l` on, the takeover policy is re-applied when the owner signs back in, since the fall-back had to clear `DisableLockWorkstation` to reach the real lock (section 17). The wizard explains each setting in one sentence, plus the trade-off: "Agents keep running, but the real Windows lock isn't used." Takeover changes are recorded in a separate persistent takeover journal, so uninstall restores them.

### 12. Configuration

- **Effective config** is owned by the Service. It's stored where SYSTEM writes and users only read.
- **User-editable config:** a TOML file in the owner's profile, created with comments on first run. The App watches it. On save, the App sends it with `PutConfig`. The Service validates it against the schema, rejects invalid fields with errors written to the App and to a sibling errors file, and keeps the last good config.
- **Locked means read-only:**
  - On `Arming`, the Service removes the owner's write permission on the editable file (journaled) and restores it on unlock.
  - The App disables editing while locked.
  - `PutConfig` returns `Locked` in any state except `Unlocked`, and also while the Windows lock is up (tracked through session notifications).
  - The file isn't re-read automatically after unlock. If it changed, the App offers "Apply changes made to the config file?"
- **Sensitive fields** return `NeedsConfirmation` and are not applied. A sensitive field is any top-level key `unlock`, `hardening`, or `takeover`, including dotted keys (`unlock.methods = []`), inline tables (`unlock = { methods = [] }`), and table headers with whitespace or a dotted suffix (`[ unlock ]`, `[unlock.pin]`). A document that does not parse also returns `NeedsConfirmation`, so an unreadable file cannot skip the check. These apply only after a Hello or password check in the App. **Turning any takeover setting on counts as weakening**, because it replaces the real Windows lock (section 17).
- **Not sensitive:** `battery_floor_percent`, `heat_check` and everything in `[pause]`. They don't change who can get in (section 18 explains why pausing can't weaken the lock), so they need no confirmation. Like every field, they're refused while locked.
- **Schema:** generated from the Rust types (`schemars`), published as JSON Schema, and used by the App UI and by agents. `schema_version` allows migrations.

```rust
#[derive(Serialize, Deserialize, JsonSchema)] #[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,                  // 1
    pub lock: LockSection,                    // hotkey: String = "Ctrl+Alt+Shift+L", display: Display, message: Option<String>, show_clock: bool
    pub unlock: UnlockSection,                // methods: Vec<MethodId> (ordered, ≥1), softlock_pin_enabled: bool
    pub keep_awake: KeepAwakeSection,         // enabled: bool = true, keep_display_on: Auto|Always|Never,
                                              // battery_floor_percent: u8 = 15 (5..=50), heat_check: Auto|Off = Auto
    pub pause: PauseSection,                  // enabled: bool = false, apps: Vec<PausedApp> (at most 32)
    pub network: NetworkSection,              // keep_online: bool = true, notify_on_drop: bool
    pub takeover: TakeoverSection,            // win_l: bool = false, idle_lock: bool = false, idle_minutes: u16 = 10, lid_close: bool = false
    pub hardening: HardeningSection,          // disable_touch_pen: true, disable_game_controllers: true, uac_require_credentials: true,
                                              // suppress_notifications: true, stop_accessibility_input: true, power_button: DoNothing|Default,
                                              // keyboard_filter: Auto|Off
    pub updates: UpdatesSection,              // channel: Stable|Dev, auto_check: bool = true
}
pub enum Display { Opaque, Dimmed { percent: u8 }, Transparent }
pub struct PausedApp { pub path: String, pub mode: PauseMode }   // full image path, or a packaged app's family name
pub enum PauseMode { Efficiency, Suspend }                       // Efficiency is the default
```

### 13. App, settings UI and setup wizard

- Tauri 2 with a TypeScript front end using a lightweight framework (Svelte 5) and plain CSS. It follows **Google's Modern Web Guidance**: Baseline web features only, no unneeded dependencies, semantic HTML, WCAG 2.2 AA, keyboard-complete, `prefers-reduced-motion`, `prefers-color-scheme`. The Modern Web Guidance plugin is installed for coding agents when implementation starts.
- Visual and interaction design follows **Apple's Human Interface Guidelines** principles: clarity, deference and consistency. That means plain-language labels, one primary action per screen, honest destructive-action wording, and progressive disclosure for advanced settings. It should still feel native on Windows (Mica background, system accent color, Segoe UI Variable).
- **Wizard steps:**
  1. Welcome and what SoftLock is and isn't.
  2. The background service: what it does and why it needs admin rights.
  3. Fail-closed disclosure, with an "I understand" checkbox.
  4. Unlock methods (Hello detected, optional PIN).
  5. Hotkey.
  6. Display mode, with a live preview.
  7. Keep awake and online: the battery floor (default 15%), whether this computer reports its temperature, and an optional "Pause background apps while locked" picker (off by default). Gentle pause (Efficiency mode, EcoQoS) is preselected. Choosing strong pause (fully suspend) explains that the app is frozen, so its network connections and timers may drop.
  8. Takeover options (all off, each explained).
  9. Disk-encryption check.
  10. Try it: a guided 10-second test lock.
- **Settings, Keep awake page:** keep awake on or off, keep display on, a battery-floor slider (5–50%), the heat check with the sources this computer reports (or the "doesn't report its temperature" note), and network status.
- **Settings, Pause background apps page:** an on/off switch, the picker built from `ListRunningApps` with the suggested apps marked for the owner to confirm, and a per-app choice. Gentle pause (Efficiency mode, EcoQoS) is preselected. Strong pause (fully suspend) is opt-in, and choosing it explains that the app is frozen, so its network connections and timers may drop. Apps that can't be paused are shown with the reason. No rules, schedules or conditions.
- **Tray menu:** Lock now, Lock for real, Open settings, status line, Quit (Quit doesn't stop the Service).
- The native lock-screen overlay is **not** a webview. It's drawn with Direct2D. The App only *previews* it, by asking the Lock Host to render a preview into a normal window.

### 14. Installer, service transparency and updates

- **Installer:** a WiX MSI (per-machine) that installs the Service, Lock Host, Broker and App, the autostart entry, and an uninstall custom action that refuses while Locked and otherwise reverts all journals and takeover changes. The installer UI and the App's "About the service" page show the same plain-language text: what the Service is, what it changes while locked (the hardening table above, the battery and heat floor, and any apps you chose to pause), what it never does (no network except update checks, no telemetry, no reading your files), and how to remove it.
- **Updater:**
  - The App checks a static manifest per channel (`stable`, `dev`) hosted on GitHub Releases. The manifest and every artifact are signed with an Ed25519 (minisign) key held by the maintainer. This is independent of Authenticode, so it's free and required from day one.
  - The App downloads the update and asks the Service to install it.
  - The **Service verifies the signature itself** with the embedded public key before running `msiexec` silently. It never installs while Locked and refuses downgrades unless the request carries an explicit user-confirmed downgrade flag.
  - The update key's private half never sits in CI secrets for the stable channel. Stable releases are signed locally by the maintainer. Dev-channel builds use a separate key.
- **Unsigned start:** releases ship without Authenticode signing, and the README and install guide show the SmartScreen "More info → Run anyway" step. Code signing (and notarization on Mac) is a launch task.
- **Disk encryption:** the Service reads `Win32_EncryptableVolume` (BitLocker and Device Encryption) for the system volume and reports On, Off or Unknown.

### 15. Logging, privacy and diagnostics

- Structured local logs (`tracing`), rotated, readable by Administrators and the owner. They hold state transitions, lock sources, fall-back reasons (including battery and heat readings), hardening apply/revert results, and each app pause and resume (the app's file name and the result). **Secrets, typed characters and window titles are never logged.**
- No telemetry. The only outbound network traffic is update checks, which can be turned off.
- "Export diagnostics" puts logs, version and redacted config into a zip.

### 16. Later platforms (abstraction now, implementation later)

- **macOS 11+ (Apple Silicon and Intel), phase 2.** The bullets that used to sit here (HID-level `CGEventTap`, `sharingType = .none`, shielding window level, `LAContext`) are the 15 Nov sketch. They are superseded by the addendum in [`docs/platforms/macos.md`](platforms/macos.md), written 16 Nov 2026 against commit `23b5a39bc92c2ee32f470256d78be27d030dc59f`. The addendum keeps these traits and the Broker nonce rule. It reopens input classification and capture exclusion, because Apple's current pages do not support "HID state means a physical key" or "sharing type none hides the window from capture." Tickets are in [`docs/tickets/macos/`](tickets/macos/README.md). This addendum adds no lock behavior and no GitHub issues. Henry has not approved it yet.
- **Linux, phase 3:**
  - Root daemon plus a session client.
  - The leading design is an **evdev exclusive grab (`EVIOCGRAB`) of physical input devices**. Virtual `uinput` devices used by agents stay ungrabbed, and the daemon reads the grabbed devices for the unlock prompt. This works under both Wayland and X11.
  - `ext-session-lock-v1` is evaluated for the lock surface, but compositors route *all* input to the lock client (including `uinput` devices) and may stop presenting other surfaces. That likely conflicts with agent passthrough, so it may serve only "lock for real".
  - Overlay: a `wlr-layer-shell` overlay where available, with an X11 override-redirect fallback.
  - Capture exclusion has no portable Wayland mechanism and is an open risk.
  - Unlock: PAM.
  - Connected displays and remote views are Decision 20, on both X11 and Wayland. [`docs/platforms/linux.md`](platforms/linux.md) is this sketch. It is not a ticketed Linux platform plan, and there are no other Linux product tickets. The display tickets in [`docs/tickets/linux/`](tickets/linux/README.md) are blocked on that plan. They do not add an architecture.

### 17. Decisions resolved during ticketing (15 November 2026)

Agreed with Henry when the ticket breakdown was approved:
- **Takeover on = weakening change.** Enabling `win_l`, `idle_lock` or `lid_close` (in the App or the file) returns `NeedsConfirmation` and applies only after a Hello or password check.
- **Win+L takeover survives a fall-back.** Fail-closed and hard lock clear `DisableLockWorkstation` to reach the real lock; when the owner signs back in, the Service re-applies it if `win_l` is on.
- **SoftLock PIN before confirmation exists.** Development builds may set the SoftLock PIN without the Hello or password check until the sensitive-change confirmation is built. No release (stable or dev channel) ships before that confirmation lands, enforced by a release gate.
- **Dev install script.** Until the MSI exists, a development install script registers the Service and lays out the binaries so the App role check and the Lock Host spawn work. The MSI replaces it.
- **Admin-only repair command.** The "Repair SoftLock settings" Start menu entry (recovery guide) reverts leftover journals, refuses while Locked, and has no lock or unlock function, so it isn't the out-of-scope CLI.
- **Keyboard Filter deferred.** The optional Keyboard Filter item (section 8) is out of v1 until an Enterprise or Education machine or VM is available to test it.

### 18. Pausing background apps while locked (opt-in)

It's off by default. v1 ships both modes. Gentle pause is the default, and strong pause is opt-in per app. The owner picks apps in settings, or confirms suggestions, and SoftLock lowers their power use while locked. It's a flat per-user list of up to 32 apps, each with one of two modes. It decides nothing about which apps may run, show through or receive input, so it isn't the planned per-app allowlist or rules feature (Out of Scope). Both modes journal every pause before it happens, resume it on unlock, on fall-back to the Windows lock, and after a crash or restart, act only with the owner's own rights, and never touch system or security processes, SoftLock itself, or the agent's process tree.

| Mode | Mechanism | Undo | Caveats |
|---|---|---|---|
| **Efficiency** (default, gentle) | `SetProcessInformation(ProcessPowerThrottling)` with `PROCESS_POWER_THROTTLING_EXECUTION_SPEED` in both the control and state masks (EcoQoS on Windows 11; Low QoS on Windows 10), plus `SetPriorityClass(IDLE_PRIORITY_CLASS)`. This is what Task Manager's Efficiency mode does | Restore the journaled throttling state and priority class | The app keeps running, just slower and on efficient cores. The app can change its own setting back |
| **Suspend** (strong, opt-in per app) | `NtSuspendProcess` / `NtResumeProcess` from `ntdll`. They're undocumented but have been stable since Windows XP, and Resource Monitor's "Suspend process" uses them | `NtResumeProcess` exactly once for each recorded suspend | The app is fully frozen, so network connections and timers may drop. It also stops answering window messages, drops audio, and keeps any locks it held, so anything waiting on it can hang. The wizard and settings say this before the owner can choose it. The suspend count outlives SoftLock, hence the journal. Use it only for apps the agent never touches |

Rejected alternatives:
- `DebugActiveProcess` freezes the target, but it makes SoftLock its debugger. Only one debugger is allowed, the app can see it (anti-cheat, DRM and apps already being debugged break), and SoftLock would have to pump debug events.
- A job-object freeze isn't a documented API for apps, and putting an existing process into a job can't be undone.
- Calling `SuspendThread` on each thread is documented as a debugger tool, and it races with threads created during the walk.

**Who pauses, and what is never touched:**
- **The owner's rights only.** The Service pauses while impersonating the owner's token (`WTSQueryUserToken`, then `ImpersonateLoggedOnUser`), so it can only open processes the owner could pause from an unelevated Task Manager. SYSTEM's rights are never used for pausing. That alone puts elevated processes, services, other users' processes and SoftLock's SYSTEM processes out of reach.
- **A fixed denylist in code**, not configurable, on top of that:
  - SoftLock's own processes (Service, Lock Host, Broker, App).
  - Anything under `%SystemRoot%`. That covers `csrss`, `winlogon`, `lsass`, `dwm`, `explorer`, `ctfmon`, `LogonUI`, `consent`, `CredentialUIBroker`, the shell hosts, `svchost` and `audiodg`.
  - Critical processes (`IsProcessCritical`).
  - Protected processes: `GetProcessInformation(ProcessProtectionLevelInfo)` other than `PROTECTION_LEVEL_NONE`, which covers antimalware services.
  - Security products registered with Windows Security Center (antivirus and firewall), including their install folders.
  - Processes outside the owner's session, or not owned by the owner.
- **The protected set**, computed when the pause pass runs: the process that owned the foreground window when the lock started, the process that sent `Lock` over the pipe (for a `LockClient`), and every ancestor and descendant of either one (parent PID, checked against start times). An app in the protected set is skipped and reported as "in use by your agent".
- An Efficiency-mode app whose window becomes the foreground while locked (an agent switched to it) is restored for the rest of the lock.
- The config file can name anything. These rules are enforced by the Service at pause time, not by the picker. Picks that break them are skipped and listed in the summary after unlock.

**Lifecycle:**
- The pause pass runs in the background once `Locked` is reached, so it never slows the 500 ms lock. A failure is logged and never changes the lock state.
- Processes match by full image path, or by package family name for packaged apps. For each one, the Service writes the journal entry (PID, process start time, image path, mode, previous throttling state and priority class), flushes it, and only then pauses the process.
- New processes from a picked app that start while locked are paused too, using the process-start watch from section 8.
- Resume happens in `Unlocking` (before `Unlocked`), after the OS lock is confirmed in `FailingClosed`, in `Recovering`, from the repair command, and on uninstall. Resume checks the PID and the start time, so a reused PID is never touched. Entries for processes that have exited, including after a reboot, are dropped.
- After unlock, the App shows a short summary: how many apps were paused, which were skipped and why, and any resume failures.

**Picker and suggestions:** `ListRunningApps` (Unlocked only) lists apps with a visible window in the owner's session, minus the denylist and the App's own tree. The suggestions are the busiest apps (by CPU over the last minute) that aren't in the foreground. Call, recording and remote-control apps (Zoom, Teams, OBS, TeamViewer, AnyDesk and the like) are never suggested. Nothing is added until the owner ticks it.

**Security interactions:**
- **Escalation through the config.** An agent editing the TOML while unlocked could name `dwm.exe`, Defender or `LogonUI`. Because of impersonation and the denylist, the most it can pause is one of the owner's own unelevated apps, which it could already kill. That's why pause settings aren't sensitive. Like all config, they're refused while locked.
- **Weakening the lock.** The input guard, overlay, unlock prompt and watchdog live in SoftLock's SYSTEM processes, which the owner's token can't open and the denylist excludes. If one of them ever stops, the watchdog fails closed as usual. Pausing an app never lets physical input through.
- **Blocking unlock.** The credential UI, `LogonUI` and the Windows Hello components are under `%SystemRoot%`, and the Broker is SoftLock's own. None of them can be paused.
- **Other apps' input hooks.** If a suspended app owns a low-level hook, everyone's input waits for it until Windows drops that hook after `LowLevelHooksTimeout`. SoftLock's guard isn't affected, but agent input can lag, and the Suspend warning says so.
- **Stuck apps.** The journal and the resume on every exit path cover this. The worst case after a crash is "suspended until the Service restarts", which the SCM does right away. A reboot ends the processes anyway.

References: [SetProcessInformation](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation), [PROCESS_POWER_THROTTLING_STATE](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/ns-processthreadsapi-process_power_throttling_state), [Quality of Service levels (EcoQoS)](https://learn.microsoft.com/en-us/windows/win32/procthread/quality-of-service), [Task Manager Efficiency mode](https://devblogs.microsoft.com/performance-diagnostics/reduce-process-interference-with-task-manager-efficiency-mode/), [SuspendThread](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-suspendthread), [DebugActiveProcess](https://learn.microsoft.com/en-us/windows/win32/api/debugapi/nf-debugapi-debugactiveprocess), [Job Objects ("the association cannot be broken")](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects), [PROCESS_PROTECTION_LEVEL_INFORMATION](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/ns-processthreadsapi-process_protection_level_information), [LowLevelKeyboardProc (hook timeout)](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc).

### 19. Decisions added after ticketing (15 November 2026, Insomnia follow-up)

Henry approved both features for v1 after the Insomnia review (market research addendum):
- **Battery floor:** default 15%, range 5–50%. It's checked only while Locked with keep-awake on and the computer not charging. A lock asked for below the floor uses the real Windows lock straight away.
- **Heat floor:** best effort and documented that way. No kernel drivers and no vendor SDKs. If the computer reports nothing, SoftLock says so.
- **Both floors fail closed** to the real Windows lock, then release keep-awake so the computer can sleep, and show a notice after sign-in.
- **Pausing:** off by default. On 16 Nov 2026 Henry decided v1 ships both modes. Gentle pause (Efficiency mode, EcoQoS throttling) is the default. Strong pause (`NtSuspendProcess`) is an opt-in per-app setting. The setup wizard and the settings page explain that strong mode fully freezes the app, so network connections and timers may drop. Both modes journal every pause and resume it on unlock, on fall-back, and after a crash or restart. Both run with the owner's token only, and neither touches system or security processes, SoftLock itself, or the agent's process tree.
- **Open-core boundary:** pausing is a flat per-user list in the free core. It isn't the planned allowlist or rules feature and has no rules, schedules, conditions, profiles, remote control or fleet management.
- **Tickets:** the battery and heat floor goes into ticket 20. Pausing is the new ticket 35 (GitHub #38), blocked by 16, 20 and 23. Its settings page is in 25 and its wizard step in 29.

### 20. Connected displays and remote views (16 November 2026)

**Decision, Henry, 16 November 2026,** in his words:

> Our app should have support for connected display/multi-display/remote display/etc. support.

> If RD is running and softlock is applied, it should be applied to the RD screen too, same procedures to unlock, if anything crashes or fails should lock. If RDing in to a softlocked device it should show the softlock screen, same unlock process.

**Scope, Henry, 16 November 2026,** in his words:

> this should be for all versions (Windows 11, 10, Linux, and MacOS Universal), not just MacOS.

That decision is the behavior on every one of those platforms. The mechanism that keeps a lock visible on a remote view is **open on each platform whose capture path differs**. Henry has not picked any of them. This section does not pick them.

#### What the decision requires, on each platform

These are the invariants. A test that needs the open mechanism waits until Henry closes that platform's entry. Until then the tickets measure, and they do not promise a mechanism.

- **Remote view while locking.** Locking while a remote view is already connected covers that view with the same SoftLock screen, and the same unlock procedure.
- **Connecting to a locked machine.** A remote connection to a machine that is already SoftLocked shows the SoftLock lock screen.
- **Same unlock.** Unlock is the Broker already specified for that platform: a `Verified` verdict carrying the nonce from the Broker launch this service started. Windows offers the SoftLock PIN, Windows Hello where that stack supports it, and otherwise the PIN or the account password. macOS offers Touch ID where that stack supports it, and otherwise the account password or the dev PIN. Linux offers whatever the Linux plan's unlock is (the phase-3 sketch names PAM) and no weaker path. A remote session adds no method.
- **Remote input.** Keys, pointer, and touch from the remote view, before unlock, are what story 93 asks for. TeamViewer, AnyDesk, and Chrome Remote Desktop inject that input the same way an agent does. Mac Screen Sharing delivers it as posted events. Nothing in the hook or the tap tells those events apart from an agent's. The mechanism is the open remote-input entry below. This bullet does not claim one exists today. Input that looks physical (RDP, Hyper-V, KVM-over-IP) stays the physical-input rule above.
- **Fail closed.** The rule is invariant 3 in Decision 5, not a second rule for a remote session. Any failure while `Arming` or `Locked` ends in `OsLocked`, never `Unlocked`, except when the OS lock call itself fails. Then `FailingClosed` stays guarded until a matching `Verified` nonce moves it to `Unlocking`. It does not unlock while the OS lock is still available. Section 7's known gap until ticket 13 still applies: a dead Lock Host, and the case where both OS locks fail and a panic then runs `emergency()` (which releases the session and drops the hooks), still rely on the ticket-13 Service watchdog. See issue #13 and PR #72. "That session" is the interactive session the person is in. A second logged-on user stays out of scope.
- **Displays.** While locked, every person-visible display shows the lock: a panel that is added, removed, mirrored, rescaled, or whose scale or DPI changes; a USB display; and a laptop lid that is closed while an external display stays awake. A gap that shows the desktop is a failure, and the failure takes the fail-closed path.

**Windows 11 and Windows 10 (version 2004 or later).** The five invariants and the display rule are the same on both. Remote Desktop is the remote view, plus a remote tool of the same kind (Chrome Remote Desktop, TeamViewer, AnyDesk) when that is the view. `WDA_EXCLUDEFROMCAPTURE` is the capture-exclusion API on both, from Windows 10 version 2004. The differences that are documented, read 16 November 2026:

- [Connect to a remote Microsoft Entra joined device](https://learn.microsoft.com/en-us/windows/client-management/client-tools/connect-to-remote-aadj-pc) says that starting in Windows 10, version 1809, biometrics can authenticate to a remote desktop session. The same page says Microsoft Entra authentication for that connection needs Windows 11 with the 2022-10 cumulative update, or Windows 10 version 20H2 or later with the 2022-10 cumulative update. It also says the Windows lock screen in that remote session does not support Microsoft Entra tokens or passwordless methods such as FIDO keys, so trying to lock the remote session disconnects it instead. Fail-closed on that configuration records the disconnect. The disconnect is not Unlocked.
- The [Hello for Business FAQ](https://learn.microsoft.com/en-us/windows/security/identity-protection/hello-for-business/faq) says using Hello for Business for authentication on the remote device itself is not supported. That sentence is not split by Windows 10 and Windows 11. The same page says an external Hello camera with the lid closed was unreliable on some laptops, and that this was addressed in Windows 11, version 22H2. Where the stack in front of the person does not support Hello, unlock is the PIN or the account password on both versions.
- `windows-2022` is Windows Server 2022 (finding 02: build 20348), the Windows 10 family. `windows-2025` is Windows Server 2025 (finding 02: build 26100, 24H2), the Windows 11 family. Tickets 36 and 37 run on both images and compare them. They do not prove a client SKU, Home versus Pro as an RDP host, or Hello. The client kit is ticket 39 for the P7, whose edition the sheet records, and ticket 40 for a Windows 10 client. No Windows 10 client machine is identified (roadmap issue 33, still-open item 7). Ticket 40 stays blocked on that.

**macOS Universal 2, both slices.** The same invariants, with Screen Sharing, VNC, and Chrome Remote Desktop as the remote views, and AirPlay, Sidecar, and DisplayLink as displays. The `arm64` slice is what `macos-latest` runs (m20, m21). The `x86_64` slice runs on Henry's Intel Mac (m23 runs that slice of the display probe). Intel and Apple Silicon differ where Apple's pages differ, not in the requirement:

- Sidecar's Mac list on [Continuity features](https://support.apple.com/en-us/108046), read 16 November 2026, is macOS Catalina 10.15 or later and Mac models introduced in 2016 or later, plus two named older models. This 2020 Intel Mac is in that list. An Apple Silicon Mac of 2016 or later is in it too. Whether an iPad is available is unknown on both.
- [Connect an external display to MacBook Pro](https://support.apple.com/guide/macbook-pro/connect-an-external-display-apd8cdd74f57/mac) gives display counts for the Apple-silicon chips named on that page and says most steps apply to all models, with older models on a different article. m23 records the Intel machine. It does not copy those counts onto it. A physical Apple Silicon Mac is not in the kit. Ticket m24 is those rows, and it is blocked until a machine is named. `macos-latest` is the arm64 CI proof of the callback, not a physical hot-plug.

**Linux, X11 and Wayland.** The same invariants. There is no ticketed Linux platform plan. [`docs/platforms/linux.md`](platforms/linux.md) is a phase-3 sketch (a root daemon, an evdev grab, a layer-shell or X11 overlay, PAM) and it says the crate does not lock the machine. There are no other Linux product tickets. The display and remote-display tickets under [`docs/tickets/linux/`](tickets/linux/README.md) are blocked on a Linux platform plan. They do not add an architecture.

The paths differ, and the tickets name them without designing SoftLock around them:

- X11 and Wayland are both in scope. A hot-plug, a mode change, and mirroring are proved on `ubuntu-latest` only to the extent that image already has a display server. Ticket l01 prints the session type and stops when there is none. It does not install a desktop in order to invent one.
- Remote views the requirement covers, read 16 November 2026: [xrdp](https://github.com/neutrinolabs/xrdp/blob/devel/README.md) is an RDP server for Linux and can use an Xorg module or Xvnc. [GNOME's desktop sharing help](https://help.gnome.org/gnome-help/sharing-desktop.html) lets another device view and control the desktop, and [GNOME Connections](https://help.gnome.org/connections/connect.html) connects with `rdp://`, `vnc://`, or `spice://`. [RustDesk](https://github.com/rustdesk/rustdesk/blob/master/README.md) describes itself as a remote desktop solution and its tree has a screen-capture library (`libs/scrap`). VNC is the same class of view. These are the Linux remote paths. Which compositor or server a future plan uses is not decided here.

Ticket 18 keeps topmost re-assert, shell surfaces, and toast banners. Display add, remove, and change on Windows are ticket 36 (both runner images) and tickets 39 and 40. macOS is m20, m21, m22, m23, and m24. m06 keeps Spaces, full-screen, click-through, and the displays already attached when the probe starts. Linux is l01 through l04. Those files are the rows. This decision does not copy them.

#### Windows facts, read 16 November 2026

**Verified.** [`SetWindowDisplayAffinity`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity) says `WDA_EXCLUDEFROMCAPTURE` (`0x00000011`) means the window is displayed only on a monitor and everywhere else does not appear at all. One use the page names is video-recording controls, so the controls are not included in the capture. The remarks say the function protects window content from being captured or copied through a specific set of public operating system features and APIs, only while DWM is composing the desktop, and that it is not a guarantee the way DRM would be. The page does not name Remote Desktop. Setting the value on a Windows older than Windows 10 version 2004 behaves as `WDA_MONITOR`. `WDA_MONITOR` shows the window with no content everywhere except a monitor.

**Verified.** [`WM_DISPLAYCHANGE`](https://learn.microsoft.com/en-us/windows/win32/gdi/wm-displaychange) is sent to top-level windows when the display resolution has changed. [`WM_DPICHANGED`](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged) is sent when the window's DPI changes, including a move onto a monitor with a different DPI, or a DPI change on the monitor that hosts the window.

**Verified.** [Remote and console terminal differences](https://learn.microsoft.com/en-us/windows/win32/termserv/consoles-vs-terminals) says there is one console terminal, and the local input and output devices are attached to it. A remote terminal's keyboard, mouse, and monitor are on another system. The page's RDP scenario says that when someone remotes in, the console terminal is no longer used for the active session and is attached to an empty session with the login screen, and a remote terminal is attached to the session. When the person comes back and signs in at the console, the console terminal is reattached and the remote terminal is terminated. On Windows 10 Multisession and on Windows Server with the Remote Desktop Session Host role, the page says there can be multiple remote terminals and still only one console terminal. A WDDM remote indirect display driver, one instance per remote terminal, sends the desktop image to the client and can expose up to 16 monitors. Local WDDM adapters display the console terminal. The page says programs can call `WTSQuerySessionInformation` to tell that the user is connected remotely. It does not say whether input from the remote terminal is marked injected in a low-level hook. Console input and RDP-session input are both classified, by ticket 37, because they are different devices on that page.

**Verified.** The [Windows Hello for Business FAQ](https://learn.microsoft.com/en-us/windows/security/identity-protection/hello-for-business/faq), section "Why can't I use Windows Hello for Business when connecting remotely to a Windows device?", says Hello for Business is for local interactive sign-in, the credentials are accessible on the device where the user is physically present, and it is not supported to enroll Hello for Business or to use it for authentication on the remote device itself. The same page says Hello on the local device can sign in to a remote virtual machine and can handle redirected prompts. The key-trust section says RDP does not support key-based authentication as a supplied credential, and that a certificate deployed for that purpose can enable RDP. The cloud Kerberos section says cloud Kerberos trust cannot be used as a supplied credential with RDP or VDI. [Remote Desktop sign-in with Windows Hello for Business](https://learn.microsoft.com/en-us/windows/security/identity-protection/hello-for-business/rdp-sign-in) is a different step: a certificate in the Hello container, unlocked by a biometric gesture or a PIN on the client that is connecting, and on a Microsoft Entra hybrid joined device the page says the user must use the PIN. That is signing in to the remote session. It is not SoftLock's Broker inside a session that is already connected. Where the stack in front of the person does not support Hello, Decision 20 uses the PIN or the account password.

Finding 02 already measured the GitHub runners: one 1024×768 console display, session 2, and `WDA_EXCLUDEFROMCAPTURE` removed the marker from GDI, DXGI Desktop Duplication, and Windows.Graphics.Capture. That measurement is local capture. It is not a remote terminal.

#### The conflict, open, one entry per platform

Story 20 says an agent's screenshot, recording, and screen-sharing capture show the desktop without the overlay. A remote viewer is a person and has to see the lock. Those two sentences collide wherever the remote view is itself a capture. Henry decides each entry. This spec does not.

**Windows. Open.** The mechanism is `WDA_EXCLUDEFROMCAPTURE`. The affinity page says the window does not appear at all except on a monitor, through a specific set of public capture APIs, and it does not name Remote Desktop. The same API is on Windows 10 version 2004 and on Windows 11. The terminal page gives a remote terminal its own monitors. A Windows client moves the one interactive session onto that terminal, so there may be no second session. Server SKUs can have more than one remote session. Ticket 37 measures both runner images. It does not choose.

- **A.** Tell agent capture (GDI, DXGI, Windows.Graphics.Capture) apart from the remote terminal, and exclude only the agent path.
- **C.** A documented limit. The limit that matches Henry's words is that the remote viewer sees the lock, and an agent on that same path may see it too.
- **D.** Turn `WDA_EXCLUDEFROMCAPTURE` off while a capture-based remote tool (TeamViewer, AnyDesk, Chrome Remote Desktop) or an RDP shadow session is connected. While it is off, that viewer sees the lock, and an agent capture of the same image sees the lock. When that connection ends, exclusion is turned back on.

Option B is dropped. It was a per-session overlay for a second Remote Desktop Services session. A second interactive user, including a second RDS session, is out of scope. Offering B would need that scope change, and this decision does not make it. The letter is left unused so it is not reused for D.

**Lead recommendation for Windows, not a decision:** A where ticket 37 can tell agent capture from the remote terminal. D where the remote view is a capture-based tool or an RDP shadow and A cannot split them, so the viewer sees the lock. C when neither A nor D can be done, with the remote viewer seeing the lock.

**macOS. Open.** This is decision D9. The mechanism under test is `NSWindow.SharingType.none`. Apple's page says not to use it to hide content from capture. Screen Sharing, VNC, AirPlay, and DisplayLink are capture or pixel-forwarding paths. DisplayLink's page, read 16 November 2026, says the driver needs Screen Recording permission to read pixels and send them over USB ([DisplayLink Manager on macOS](https://support.displaylink.com/knowledgebase/articles/1932214-displaylink-manager-app-for-macos-introduction-in)). The flag is the same on Intel and on Apple Silicon. m21 measures the arm64 runner. m23 measures the Intel Mac. m24 is a physical Apple Silicon Mac, and no such machine is named. The dropped Windows option B is not a macOS option.

- **A. Likely infeasible.** Screen Sharing, VNC, and a local capture read the same window-server image. A second surface drawn into that image is in the image the capturer reads, so it cannot be visible to the remote viewer and absent from a capture of the image. [`NSWindow.SharingType.none`](https://developer.apple.com/documentation/appkit/nswindow/sharingtype-swift.enum/none) is a legacy constant the page says macOS no longer uses, and the discussion says not to use it to hide content from capture. Asking `.none` to split that one image asks a no-op to do the split. m21 still measures both slices' behavior. It does not make A feasible.
- **C.** The same documented limit as Windows: the remote viewer sees the lock, and an agent on that same path may see it too.
- **D.** Agent-side exclusion. The agent filters SoftLock windows out of its own ScreenCaptureKit filter ([`SCContentFilter` `init(display:excludingWindows:)`](https://developer.apple.com/documentation/screencapturekit/sccontentfilter/init(display:excludingwindows:))). The capturer chooses the filter. SoftLock cannot make an agent pass it. The window stays in the window-server image, so a remote viewer of that image still sees the lock.

**Lead recommendation for macOS, not a decision:** A is likely infeasible for the reason above. D for an agent that opts into the filter. C for a remote path that reads the same window-server image, with the remote viewer seeing the lock.

**Linux. Open.** The phase-3 sketch already says capture exclusion has no portable Wayland mechanism. That is a different mechanism from Windows affinity and from macOS `sharingType`, so it is its own entry. X11 capture (a VNC server or xrdp's Xvnc path reading the X framebuffer) and Wayland capture (a portal or PipeWire source, which is what a GNOME remote-desktop session and RustDesk's screen-capture library have to read) are different again. This entry does not choose an X11 extension or a Wayland protocol, and it does not extend the sketch.

- **A.** Tell agent capture apart from the remote viewer's path, once a Linux plan names those paths.
- **C.** The same documented limit: the remote viewer sees the lock, and an agent on that same path may see it too.

**Lead recommendation for Linux, not a decision:** do not pick A or C until a Linux platform plan exists and l02 has measured the runner. The recommendation, when that measurement exists, is the same rule: the remote viewer sees the lock. Windows D and macOS D are not Linux options. The sketch has no capture-exclusion switch to turn off, and it already says Wayland has no portable mechanism an agent would be required to filter.

#### Remote input, open, one entry per platform

Story 93 asks that remote input before unlock reach only the unlock UI. The classification rule cannot do that for the tools below, and this section does not pretend it can. Henry decides each entry. Physical-looking input (RDP, Hyper-V, KVM-over-IP, when the hook does not mark it injected) stays blocked under the physical-input rule. That is not these entries.

The two options are the same shape on each platform:

- **Session.** Detect a connected remote session and, while it is connected, block the input class the platform cannot tell from an agent's, except the input the unlock UI is allowed to receive. On Windows that class is injected input. On macOS it is posted events. On Linux it is whatever class a future plan finds it cannot separate, and this entry does not name an API for it. The cost is that an agent's input of that same class is blocked for the same stretch.
- **Limit.** A documented limit. A capture-based remote tool's input passes like an agent's. The person on that tool can type into the desktop the way an agent can.

**Windows. Open.** TeamViewer, AnyDesk, and Chrome Remote Desktop inject with the same flag an agent's `SendInput` sets (`LLKHF_INJECTED`, `LLMHF_INJECTED`). Ticket 37 records that flag for a console key, an RDP key, an RDP shadow key, and a key from each of those tools when the image has the tool. The terminal page does not document the flag.

**macOS. Open.** Screen Sharing input arrives as posted events, the same class an agent's `CGEventPost` uses. The tap cannot tell them apart. m03 records the fields. m21 records the remote path when the runner can open one. Intel and Apple Silicon use the same posted-event path.

**Linux. Open.** The phase-3 sketch does not name a flag that separates xrdp, VNC, GNOME Remote Desktop, or RustDesk input from input an agent posts. This entry does not add that flag. l02 records what the runner already has. It does not install a remote server in order to invent a classification.

**Lead recommendation, not a decision:** Limit, until a measurement shows a connected session can be detected without also stopping the agent's input for that whole stretch. Session is the option that matches story 93, and it stops agent input while the remote session is up. Henry picks. These entries stay open with the capture entries.

#### Tickets

Windows files are in [`docs/tickets/`](tickets/README.md). macOS files are in [`docs/tickets/macos/`](tickets/macos/README.md). Linux files are in [`docs/tickets/linux/`](tickets/linux/README.md). None of them are GitHub issues.

| Ticket | What it is | Blocked by |
|---|---|---|
| 36 | Seam 3 on `windows-2022` and `windows-2025`: display add, remove, and change | 11, 12 |
| 37 | Spike, both images: capture exclusion versus a remote view | 11, and finding 02 |
| 38 | Remote-session lock and unlock, Windows 10 and Windows 11 | 15, 21, 22, 37, and the Windows entry above once Henry closes it |
| 39 | P7 kit | 36; the RDP row also 38 |
| 40 | Windows 10 client kit | a named Windows 10 client machine; the RDP row also 38 |
| m20 | CI probe on `macos-latest` (arm64 slice) | m01, m02 |
| m21 | Spike: `sharingType` none versus a remote view | m04 |
| m22 | Remote-session lock and unlock | m14, m17, m18, m19, m21, and the macOS entries once Henry closes them |
| m23 | Intel Mac kit, including the `x86_64` slice of the display probe | m06, m20; Screen Sharing also m22 |
| m24 | Apple Silicon hardware kit | a named Apple Silicon Mac; no such machine is recorded; Screen Sharing also m22 |
| l01 | `ubuntu-latest` display-session probe | a Linux platform plan |
| l02 | Spike: capture versus xrdp, VNC, GNOME Remote Desktop, RustDesk | the plan, and l01 |
| l03 | Remote-session lock and unlock | the plan, l02, and the Linux entry once Henry closes it |
| l04 | Hardware kit | the plan, and a named Linux machine; remote rows also l03 |

---
## Testing Decisions

### What makes a good test here

- Tests check **external behavior at public boundaries**: "a lock request leads to physical input being blocked, injected input arriving, the overlay missing from captures, and policies set", not which function called which.
- Internal modules are refactored freely as long as the seam tests pass.
- Every state-machine invariant in Implementation Decision 5 and every hardening row in Decision 8 has at least one test.
- **Red-green TDD** at the seams below, one fresh agent per ticket, following the `implement` workflow.
- Security properties are tested negatively too, for example "no public IPC message sequence reaches `Unlocked` from `Locked`", "config writes while locked are rejected", and "a killed Lock Host ends in the OS lock". Property-based tests (`proptest`) generate random event and request sequences against the state machine.
- **Simulating physical input in CI:** GitHub-hosted runners have no real keyboard, and anything a test injects is flagged as injected. The Lock Host therefore has a **test-only build feature**. With it, injected events whose extra-info field carries a per-run random nonce are classified as physical. Release builds are compiled without the feature, and CI checks the release binaries to make sure the feature's code and marker strings are absent. The ticket that adds this test-only feature must add that release-binary check in the same change. The check is not a later follow-up. This simulates the classification input, not the hook. The real hook and real `SendInput` are still exercised.
- **Simulating battery and heat in CI:** the runners have no battery and no thermal zones. The same test-only build feature lets a test feed battery and thermal readings into the Windows backend's `Power` implementation, so the real floor path (OS lock, release, sleep request) runs on a runner. The same release-binary check proves it's absent from release builds.
- **Prior art:** none yet (new repo). Conventions: Rust unit and integration tests with `cargo test` / `cargo nextest`, `proptest`, `insta` snapshots for IPC JSON and config errors, and Playwright against the Tauri front end.

### Proposed test seams

1. **Lock core (`LockController` public API with the fake platform).** This is the state machine, config validation, unlock verdict handling, backoff, journal, heartbeat and watchdog rules, the battery and heat floor rules (hysteresis, unknown readings, charging, plausibility, lock below the floor), and the pause lifecycle and denylist and protected-set filtering over a fake process table, driven by events against an in-memory `Fake` implementation of every platform trait. It runs on any OS and covers most logic and all invariants.
2. **Service IPC contract (the named pipe, black box).** The real Service runs on a Windows runner, and tests act as `App`, `LockClient` and an impostor client speaking the documented JSON protocol. This covers roles, rejections (`Locked`, `Unauthorized`, `Invalid`, `Conflict`), the absence of any unlock path, `PutConfig` and confirmation flows (including the new keep-awake and pause fields, which never need confirmation), `ListRunningApps` (App only, Unlocked only), update-manifest verification against a local test server, and event streams.
3. **Windows desktop end to end (probe harness).** A probe app on a Windows runner, with the test-only physical feature, checks what the OS actually does:
   - Injected input reaches the probe window, and simulated-physical input doesn't.
   - Focus is unchanged.
   - Captures (GDI, DXGI duplication, Windows.Graphics.Capture) don't contain the overlay's marker color. A remote view is not this bullet. Ticket 37 measures it, and Decision 20 is open.
   - A display change leaves every monitor covered (ticket 36) on both `windows-2022` (Windows 10 family) and `windows-2025` (Windows 11 family). The runner's known display is one 1024×768 console screen (finding 02). A resolution change is the change the job can do. An add and a remove run only if a virtual display driver is already on the image. Otherwise the log says add and remove are unproven in CI, and tickets 39 and 40 have the hardware rows.
   - The hardening registry values, device states and power requests are applied and then restored exactly.
   - Killing or suspending the Lock Host, or stopping the Service, ends in an OS-locked or disconnected session.
   - Recovery after a simulated crash (journal present at Service start) restores everything.
   - Pausing: probe child processes in the pause list are throttled (read back with `GetProcessInformation`) or suspended (their heartbeat stops), and they're resumed exactly on unlock, on fail-closed, and after the Service is killed and restarted. Denylisted, elevated, foreground-tree and lock-requester-tree probes are never touched.
   - Battery and heat: a simulated low battery or thermal trip while locked ends in the OS lock, and the power requests are released only after that.
   The fail-closed cases run last, in their own job, because they end the runner's interactive session.
4. **Settings app UI (front end against a mocked IPC).** The wizard flow, settings validation errors, read-only state while locked, sensitive-change confirmation, updater prompts, the battery-floor and heat-check settings, and the pause picker (suggestions, gentle mode preselected, the strong-mode explanation that the app is fully frozen so network connections and timers may drop, unpausable apps with reasons), tested with Playwright against the Tauri front end and a fake IPC bridge.

Everything else (the Windows backend internals, overlay drawing, hook details) is covered by these seams and doesn't get tests of its own.

### CI

- **GitHub Actions:**
  - `cargo fmt` and `clippy` (deny warnings) plus seam 1 on Linux, Windows and macOS (keeps Core portable).
  - Seams 2 and 3 on `windows-2022` and `windows-2025` runners (Server SKUs with an interactive desktop. This must be confirmed in prototype P7).
  - Seam 4 on Linux.
  - MSI build, plus an install, uninstall and "nothing left behind" check on Windows.
  - The macOS Universal 2 job (both slices, `lipo`, the release-binary scan on each slice, and a Tauri `universal-apple-darwin` build) is specified in the macOS addendum and in ticket m01. It is not in `.github/workflows/ci.yml` yet. Seam 1 on `macos-latest` stays as it is.
  - `cargo deny` for licenses (Apache-2.0-compatible only) and advisories.
- GitHub-hosted runners are Windows Server, so client-only behavior (the Ctrl+Alt+Del screen layout, Windows Hello, Modern Standby, the lid) **can't be covered in CI**. It goes on the manual checklist.

### Manual release checklist (only where real hardware is required)

Run on one Windows 11 laptop (Modern Standby, touchscreen if available) and one Windows 10 22H2 machine:
1. A physical keyboard, mouse, touchpad and touch are all blocked. A USB keyboard plugged in while locked is blocked.
2. Ctrl+Alt+Del shows no Task Manager, Sign out, Switch user, Change password or Lock. Cancel returns to SoftLock. Note which icons remain.
3. Physical Win+L does nothing while locked. With `win_l` takeover on, Win+L SoftLocks.
4. Unlock with Hello face or fingerprint, Hello PIN (credential passthrough), the account password and the SoftLock PIN.
5. Close the lid with `lid_close` on. Run a 30-minute agent task, then a 60-minute one with the display on and the lid closed. Verify the agent made progress.
6. The power button tap does nothing while locked.
7. Run an agent (for example Computer Use, or a Playwright headed run) end to end while locked. Its screenshots show the desktop.
8. A remote view under test (Remote Desktop, and TeamViewer or AnyDesk when that product is the view) shows the SoftLock screen. What that view's input does before unlock is the open remote-input entry in Decision 20. The sheet is ticket 39. This line does not add a second copy of those rows, and it does not claim the hook can tell that input from an agent's.
9. Battery floor: on battery, set the floor 2% below the current level, lock and run an agent. When the battery passes the floor, the real Windows lock appears, the laptop sleeps, and after sign-in the notice shows the reading. Note which heat sources the laptop and the P7 report in status.
10. Pausing: pick one app in Efficiency mode and one in Suspend (for example a chat app and a music app), lock for 10 minutes, then unlock, and both work normally. Repeat, holding the power button while locked, and check that nothing is stuck after the restart.
Each item is a one-line pass or fail in the release notes.

---

## Out of Scope

- **Per-app allowlists or rules** (which apps may run, show through or get input). This is a planned future feature and is deliberately not designed here, because parts of it may become paid. The v1 pause list (Implementation Decision 18) isn't this. It only lowers the power use of apps the owner picked and never decides what may run, show or get input.
- **Anything beyond a flat pause list:** pause rules, schedules, conditions, per-agent profiles, wildcards, or centrally managed pause lists.
- **Vendor temperature sensors:** kernel drivers, vendor SDKs or EC/MSR reads for temperature. The heat floor uses only what Windows exposes (Implementation Decision 10).
- **The `softlock` command-line tool.** v1 ships only the Service's lock-only IPC that it will use.
- **Remote and AI triggers** (messaging a bot "softlock device", MCP servers, chat integrations).
- **Future enterprise edition** (for example fleet or MDM management, central policy, remote triggers). Nothing from it is designed, stubbed or committed to the public repository.
- **The final product name and branding,** domains and trademark filing.
- **Code signing** (Authenticode, an EV certificate) and **Apple notarization.** Releases start unsigned.
- **macOS and Linux implementations** beyond the platform traits and the notes in Implementation Decision 16.
- A Windows credential provider (C++), dynamic unlock-method plug-ins, and security-key or phone-approval methods.
- The optional Keyboard Filter integration (user story 30), deferred until an Enterprise or Education test machine is available (Implementation Decision 17).
- Replacing or restyling the real Windows sign-in or lock screen (LogonUI). SoftLock never imitates it.
- Anti-theft, device tracking, or remote wipe.
- Protection against malicious software already running as the owner, hardware attacks, or a forced power-off without disk encryption.
- Multiple simultaneous interactive users. A second person logged on at the same time, including a second Remote Desktop Services session on a server SKU, stays out. One interactive session that moves between the console terminal and a remote terminal is in scope (Decision 20).
- IME composition in the unlock field, and localization beyond English (strings are externalized for later).

---

## Further Notes

### Top technical risks: prototype these first, before building the full v1

| # | Risk | Prototype (spike) | What happens if it fails |
|---|---|---|---|
| **P1** | **Do the Ctrl+Alt+Del policies take effect instantly, without a sign-out, and what does that screen still show?** `DisableTaskMgr`, `NoLogoff`, `DisableChangePassword`, `HideFastUserSwitching` and `DisableLockWorkstation` must hide their options the very next time Ctrl+Alt+Del is pressed, on Windows 10 and 11, Home and Pro. Also check whether `NoClose` hides the power icon, what the accessibility icon can launch, and whether the network icon matters. | A service that sets and clears each value, then manual Ctrl+Alt+Del presses on two machines | If an option needs a sign-out to disappear, that piece can't be fully loophole-free on that edition. We document it, use Keyboard Filter where available, or keep that option and make sure its action can't reach the desktop |
| **P2** | **Physical vs. software input classification gaps.** Will a SYSTEM-in-session hook see input to elevated windows? Does blocking at the low-level hook also stop Raw Input (`WM_INPUT`) and apps that read HID devices directly? Do touch, pen and gamepads really need device disabling? Do Voice Access and the on-screen keyboard bypass the guard? Is the hook ever dropped under load? How do remote tools classify? The 15 Nov guess was that RDP, VMs and KVMs look physical and are blocked, and that TeamViewer and AnyDesk are injected and pass. The physical-looking half is the classification rule in Decision 1. TeamViewer, AnyDesk, and Chrome Remote Desktop are injected the same way an agent is, so the hook passes them today, and it cannot single them out. Decision 20 leaves that as the open remote-input entry: Session blocks injected input except the unlock UI while a remote session is connected, which also stops agent input for that time, and Limit documents that a capture-based remote tool's input passes like an agent's. Ticket 37 records the flag for console input, RDP, an RDP shadow, and those tools when the image has them. | A hook prototype plus a test matrix of input sources and target apps (elevated window, raw-input app, game, browser) | Each gap gets a hardening item (device disable, process stop) or a documented limitation. Raw Input leakage is the most serious possible result |
| **P3** | **Can the overlay stay on top and private?** Without uiAccess (unsigned builds), a topmost window can be drawn over by the Start menu, Task View, toasts, UAC's secure-desktop dimmed background (which may be a snapshot of the real desktop that ignores capture exclusion), and full-screen exclusive apps. | Open each surface by injected input while locked and photograph the physical screen | Physical input still can't reach them, so this is information exposure, not access. Mitigations: suppress toasts, close shell surfaces when they open, document it. uiAccess becomes possible once the builds are signed |
| **P4** | **Does fail-closed work under the policies?** `LockWorkStation` fails while `DisableLockWorkstation` is set. Check that clearing the policy and then locking, or `WTSDisconnectSession` from session 0 and from the user session, reliably reaches the sign-in screen within 1 s. Also check the Lock Host's own fall-back when the Service is killed. | Kill and suspend tests on a real machine and on a CI runner | This is the core promise. If any path is unreliable, it gets fixed before anything else ships |
| **P5** | **Keep-awake on Modern Standby and with the lid closed.** Do the System- plus Display-required power requests stop standby and Desktop Activity Moderator throttling when the screen is dim or black, and with the lid closed (internal panel off)? | A 60-minute agent run on a Modern Standby laptop: lid open with the screen dimmed, then lid closed, plugged in and on battery | Document hardware limits. `lid_close` stays off by default, and some devices may only be able to keep the lid open |
| P6 | **Unlocking with Windows Hello under the input guard.** The Hello dialog must come up above the overlay and accept the PIN through credential passthrough without opening a hole. `LogonUserW` must work offline for Microsoft accounts (cached credentials). | Prototype the Broker plus passthrough | Fall back to the password plus SoftLock PIN in our own field, and Hello face or fingerprint only |
| P7 | **CI feasibility.** Do GitHub-hosted Windows runners give a usable interactive desktop for low-level hooks, `SendInput`, overlays and capture, and can the Service be installed there? | A one-day CI spike | Use a self-hosted Windows runner for seam 3 |
| P8 | **Antivirus and SmartScreen.** An unsigned input-blocking "screen locker" that runs as SYSTEM is likely to trigger false positives. | Scan release candidates with VirusTotal and Microsoft Defender | Submit false-positive reports to Microsoft. Bring code signing forward if blocking is widespread |
| P9 | **Heat readings are often missing or fake on consumer laptops.** Thermal zones depend on the firmware, and CPU temperature needs a kernel driver. | No separate spike. Ticket 20 records which sources the laptop and the P7 report | The heat floor is documented as best effort, and status says when the computer reports nothing. The battery floor and the firmware's own thermal shutdown still apply |
| P10 | **Suspending apps can hang other apps** (window messages, held locks, low-level hooks), and `NtSuspendProcess` is undocumented. | No separate spike. Ticket 35 tests it on the runner and on Henry's laptop | v1 ships both modes. Gentle pause (Efficiency) stays the default. Strong pause (Suspend) is opt-in per app: the wizard and settings say the app is fully frozen, so network connections and timers may drop. Every pause is journaled and resumed on every exit path |

### Contribution, licensing and the open-core split

- The public repository is Apache-2.0, with a `LICENSE` and a `NOTICE` file. Dependencies must be Apache-2.0-compatible (enforced with `cargo deny`).
- **Contributions:** while Henry is the only author, no agreement is needed. **Once outside pull requests start, require a DCO** (`Signed-off-by`, checked by a bot). **Before any third-party code is accepted into anything that could become a paid module, add a CLA** (a license CLA, not copyright assignment), following GitLab's DCO-for-core, CLA-for-`ee` pattern. Planned paid features are never written into the public tree, so they never need relicensing.
- Henry is a minor. Contributor agreements, commercial licenses and trademark filings are contracts, so a parent, guardian or legal entity may need to hold the copyright and sign them. That's a question for a lawyer before the paid edition or the CLA.

### Name

"SoftLock" is a **working name only**. The research found an iPhone security app called SoftLock (softlock.app), a long-running security vendor at softlock.net, HMD's "Softlock" device lock, a near-identical macOS GitHub project, a live "SOFTLOCK STUDIO" Class 9 filing, and the gaming meaning of "softlock". All the obvious softlock domains are taken. To keep a rename cheap:
- Keep the product name in one constant and in the string resources. The settings app uses that name. Its bundle id is `com.softlock.app` and its npm package name is `softlock`. Both live next to the product-name constant. The package stays private.
- The background service's display name is "SoftLock Agent". Only the service uses that name. Its pipe id (`softlock-agent`) and its Windows service name and registry subkey (`SoftLockAgent`) are computed from that one display-name constant.
- Don't publish the crate or the package. The settings-app package is named `softlock`, but it is private.
Candidates to check properly before launch: SoftLatch, SoftVeil, ForegroundLock. AgentLock and OverlayLock should be avoided (already used by similar projects).

### Docs that ship with v1

- **User guide:** what SoftLock protects against and what it doesn't, unlock methods, takeover options, display modes, the battery and heat floor (and that the heat check is best effort), pausing background apps (gentle Efficiency mode by default, and the strong-mode explanation that the app is fully frozen so network connections and timers may drop), and what to do after a fall-back.
- **"What the background service does":** the same text as the installer page and the hardening table.
- **Terms and disclaimer:** Apache-2.0 "as is". It must include: "If SoftLock fails while locked, it switches to the real Windows lock, and your agents will pause until you sign in." It also says SoftLock is not anti-theft and not a substitute for disk encryption.
- **Recovery guide:** how the Service repairs settings after a crash or power loss, and a manual "repair settings" command available from the Start menu for administrators.
- **Contributor guide:** the architecture overview, how to run the seams, the DCO policy (when it's enabled), and the open-core boundary rule.

### Market positioning (for context)

The closest products are Softros Transparent Lock Screen (paid, Windows) and Warden (paid, macOS 15.2+). OpenAI's "Locked use" is macOS-only. SoftLock's space is: free and open source, agent-first, capture-excluded overlay, cross-platform by design, and openly documented security limits.

### Iteration

To change behavior, update this spec first, then the tickets, then the code. The next step is `to-tickets`. The suggested first tickets are prototypes P1–P5 and P7, followed by a tracer bullet: hotkey → Service → Lock Host overlay plus guard → password unlock → revert, with fail-closed.
