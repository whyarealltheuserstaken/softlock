# SoftLock v1: Product Spec

| | |
|---|---|
| **Status** | Approved. Ticket breakdown approved 15 Nov 2026 (31 v1 tickets, 1 deferred). Gaps resolved during ticketing are in Implementation Decision 17 |
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
- **It keeps the computer working.** While locked, SoftLock keeps the computer awake and online, including on Modern Standby laptops, so agents and downloads keep running.
- **It can replace the real lock (your choice).** A setting in the first-run wizard, easy to change later, lets SoftLock take over Win+L, the idle lock and lid-close locking. It's off by default in v1.
- **Settings are scriptable but tamper-proof.** You change settings in a clean settings app or in a readable TOML file that you or an AI agent can edit. Changes are accepted only while the computer is not locked, by SoftLock or by Windows.
- **Locking is safe to automate.** The service exposes a lock-only local interface. Agents and scripts can lock the computer, and nothing outside the lock screen can ever unlock it. The command-line tool and the remote chat-bot triggers that build on this come later.
- **It's easy to install and keep current.** A normal installer explains in plain words what the background service does and why it needs admin rights. An in-app updater uses signed update manifests and offers stable and dev channels. Builds start without code signing (a warning is expected), and signing is added at launch.
- **Disk encryption is part of the story.** Pulling the power or holding the power button can't be stopped by any software. It lands at the real Windows sign-in, so setup checks for BitLocker or Device Encryption and recommends turning it on.

---

## User Stories

Actors: **owner** (the person who installs SoftLock and owns the computer), **bystander** (anyone physically at the computer while it's SoftLocked, including the owner before they unlock), **agent** (any software running in the owner's session: AI agents, test runners, automation, remote-control tools), **administrator** (an IT person or power user managing the computer), **contributor** (an open-source developer), **maintainer** (Henry).

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

---
## Implementation Decisions

### 1. Threat model (this defines "no loopholes")

| Adversary | v1 position |
|---|---|
| **T1: A person physically at the computer** (keyboard, mouse, touch, pen, gamepad, USB devices, voice, power and lid buttons) | **In scope, and must be loophole-free.** They can't get to the desktop or any running app without an enabled unlock method. Their best options are a forced power-off or reboot (accepted, because it lands at the real Windows sign-in, which is why disk encryption is recommended) or disrupting the session (for example unplugging the network). Both are denial of service, not access. |
| **T2: Software running as the owner** (agents, scripts, remote-control tools) | **Trusted to run, not trusted to unlock.** Software input passes through by design. SoftLock's own interfaces offer no unlock path. SoftLock doesn't claim to stop *malicious* code running as the owner, because such code already has the owner's files and session. Remote-control tools such as TeamViewer, AnyDesk and VNC inject input like agents do, so they keep working while locked. This is documented. |
| **T3: Hardware and pre-boot attacks** (DMA, cold boot, keyloggers, booting another OS) | Out of scope. Covered by BitLocker or Device Encryption and firmware settings. |

**Physical vs. software input, defined:** on Windows, an input event is *software* if the low-level hook reports it as injected (`LLKHF_INJECTED` for keyboard, `LLMHF_INJECTED` for mouse) and *physical* otherwise. Some consequences:
- Input from RDP sessions, Hyper-V basic sessions, KVM-over-IP and other virtual-HID tools looks physical inside the guest, so it's blocked.
- Some accessibility tools inject input or act through UI Automation without any input event at all. That's why they're stopped while locked (Hardening section).

### 2. Windows limits we design around (stated honestly)

- **Ctrl+Alt+Del (the Secure Attention Sequence) can't be intercepted or swallowed** by any user-mode code. Winlogon handles it and switches to the secure Winlogon desktop. `BlockInput` is undone by it. SoftLock therefore doesn't try to block it. Instead, it removes the options on that screen through policy values, so **Cancel** is the only meaningful action, and Cancel returns to the Default desktop, where SoftLock is still showing. The power, network and accessibility icons on that screen aren't all removable by policy. Power is accepted (a restart lands at the real sign-in), and the accessibility tools are stopped by hardening. Keyboard Filter (Enterprise, Education and IoT only) can suppress the sequence itself and is used when available and enabled.
- **The secure desktop is unreachable.** No SoftLock process can draw on or read input from the Winlogon desktop, and neither can agents. That's why SoftLock never calls the real lock except for hard lock and fail-closed.
- **UAC prompts appear on the secure desktop** (by default), outside SoftLock's control. While locked, SoftLock changes the elevation prompt for administrators to require credentials, and sets standard-user prompts to auto-deny, so a bystander can't approve them with one click. Agent elevations will wait or time out while locked, which is expected and documented.
- **Win+L is handled by the system, not delivered to low-level hooks** in the normal case. The `DisableLockWorkstation` policy is the only reliable way to neutralize it. That same policy also makes `LockWorkStation()` fail, so every code path that needs the real lock (hard lock, fail-closed) **clears that policy first** or uses the session-disconnect primitive (section 7).
- **No uiAccess in v1.** uiAccess windows sit in a higher z-band, but they need an Authenticode-signed binary in a secure location. v1 is unsigned, so the overlay is a normal topmost window. Shell surfaces (Start menu, Task View, some toasts, the secure-desktop dim background) can draw above it if an agent opens them. Physical input to them is still blocked. The remaining risk is that their contents are visible on screen (see Risks).
- **Low-level hooks are removed silently** if a callback exceeds `LowLevelHooksTimeout`. The hook therefore runs on a dedicated high-priority thread, does no I/O and no allocation in the callback, and is checked continuously for liveness.
- **Capture exclusion** (`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`) needs Windows 10 version 2004 (build 19041) or later. That sets the minimum supported version. DWM applies it, so BitBlt/GDI capture, DXGI Desktop Duplication and Windows.Graphics.Capture all skip the overlay.

### 3. Architecture and modules

SoftLock is a Rust workspace with thin native layers. Each module is deep: a small interface hiding a lot of behavior.

| Module | Runs as | Responsibility | Platform-specific? |
|---|---|---|---|
| **Core** (`softlock-core` library) | linked into the others | Lock state machine, config model, schema and validation, IPC protocol types, unlock-method interface, backoff policy, policy-journal model, heartbeat rules. **No OS calls.** | No |
| **Platform interfaces** (in Core) | n/a | Traits that every OS backend implements (section 4) | No (definitions only) |
| **Windows backend** | linked into Service, Lock Host, Broker | Implements the platform traits with `windows-rs` and Win32 | Windows |
| **Service** (`softlock-service`) | Windows service, LocalSystem, automatic start | The single authority. Owns the state machine, IPC server, policy apply/revert with journal, device hardening, power requests, watchdog, config authority, update installation, BitLocker detection | Thin Windows shell around Core |
| **Lock Host** (`softlock-lockhost`) | Started by the Service into the owner's interactive session on `winsta0\default`, with a SYSTEM token (see note) | Native overlay windows (one per monitor), the input guard (low-level hooks), the unlock prompt UI, the heartbeat. Pre-spawned and idle at sign-in, so locking is instant | Native Win32 + Direct2D/DirectComposition |
| **Unlock Broker** (`softlock-broker`) | Started on demand by the Service as the owner (user token) | Calls Windows Hello (`UserConsentVerifier`), which has to run in the user's context. Reports to the Service over a private inherited pipe, then exits | Windows |
| **App** (`softlock-app`) | Tauri 2, the owner, medium integrity, starts at sign-in | Tray icon, global hotkey registration, settings UI, setup wizard, lock preview, status, updater UI, user-editable config file watcher | Mostly portable. Tray and hotkey via Tauri plugins |
| **Installer** | n/a | WiX-built MSI. Installs the Service (MSI `ServiceInstall`), the binaries and the App autostart, and shows the "what the service does" page. MSIX isn't used, because unsigned MSIX packages can't be installed outside developer mode | Windows |

**Why the Lock Host runs as SYSTEM in the user's session:** a low-level hook only sees input headed to windows at or below its own integrity level. A hook at medium integrity misses input to elevated windows, which would be a loophole. Running the guard as SYSTEM in the session covers every window on the Default desktop, and UIPI stops lower-integrity processes (agents) from sending messages to or tampering with it. The Unlock Broker exists because Windows Hello must be called in the user's context. *This choice is confirmed by prototype P2. The fallback is to use the owner's elevated linked token for administrators.*

**The overlay never takes focus or mouse input.** Its windows use `WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` plus `WDA_EXCLUDEFROMCAPTURE`. Agents' injected clicks fall through to the real windows underneath, and keyboard focus stays where it was. The overlay receives physical keystrokes only from its own input guard, which swallows them in the hook and forwards them in-process to the unlock prompt (translated with `ToUnicodeEx` and the owner's active keyboard layout). The overlay re-asserts topmost on a timer and on `WM_WINDOWPOSCHANGED`, and re-covers monitors on `WM_DISPLAYCHANGE`. IME composition isn't supported in the unlock field in v1.

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
pub trait Power { fn hold(&self, req: AwakeRequest) -> AwakeGuard; fn capabilities(&self) -> PowerCaps; }
pub trait Credentials { fn verify_password(&self, user: &UserId, pw: &Secret) -> Verdict; }
pub trait EncryptionStatus { fn system_volume(&self) -> DiskEncryption; }
```

### 5. Lock state machine (owned by the Service)

```rust
enum LockState {
    Unlocked,
    Arming   { source: LockSource, step: ArmStep },           // journal -> guard -> overlay -> hardening -> power
    Locked   { source: LockSource, prompt: PromptState },
    Unlocking{ step: DisarmStep },                            // hardening revert -> power release -> overlay hide -> guard disarm
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
// Locked     --Verified{via: LockHostChannel}--> Unlocking          // the ONLY way out of Locked
// Locked     --HeartbeatMissed|HostExited|GuardUnhealthy|OverlayUnhealthy--> FailingClosed
// Locked     --OsLockObserved-->                OsLocked
// Unlocking  --Done-->                          Unlocked
// Unlocking  --RevertFailed-->                  OsLocked (after os_lock) + journal kept for retry
// FailingClosed --OsLockConfirmed-->            OsLocked
// OsLocked   --OsUnlocked, revert succeeds-->   Unlocked (journal reverted, user notified if after_fail)
// OsLocked   --OsUnlocked, revert still failing--> OsLocked (guard and overlay disarmed, journal kept, Repair notice)
// Recovering --SessionActiveAndJournalSaysLocked--> FailingClosed
// Recovering --NoActiveLock-->                  Unlocked (journal reverted)
// Any        --ServiceStopRequested while Locked--> FailingClosed, then stop
```

Invariants (each one is a test):
1. `Locked → Unlocking` happens only on a `Verified` verdict from the Broker this service launched, checked against that launch's CSPRNG nonce, or on the credential verifier returning `Verified` for a `SubmitSecret`. The Lock Host does not verify and has no verdict that unlocks. No public IPC verb produces it.
2. Every hardening change is in the persisted journal **before** it's applied.
3. Any failure while `Arming` or `Locked` ends in `OsLocked`, never `Unlocked`.
4. Config writes are rejected in every state except `Unlocked` with the session not OS-locked.
5. After `Unlocked` is reached, the journal is empty and the system settings equal the values recorded before the lock.

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
| `GetStatus` | all | all | state, lock source, guard and overlay health, last fall-back |
| `Lock { source, note? }` | App, LockClient | Unlocked | `Accepted` or `AlreadyLocked` |
| `HardLock` | App | Unlocked, Locked\* | calls the real OS lock |
| `GetConfig` | App, LockClient | all | effective config plus revision |
| `ValidateConfig { toml }` | App, LockClient | all | `Ok` or `Invalid { errors[] }` |
| `PutConfig { toml, base_revision }` | App | Unlocked only | `Applied`, `NeedsConfirmation { fields[] }`, `Invalid`, `Conflict`, `Locked` |
| `ConfirmSensitiveChange { token }` | App | Unlocked only | runs a Hello or password check through the Broker, then applies |
| `SetSoftLockPin { pin }` / `ClearSoftLockPin` | App | Unlocked only, after a confirmation check | `Ok` |
| `Subscribe` | App | all | event stream: `StateChanged`, `ConfigChanged`, `FellBackToOsLock { reason, at }`, `UpdateAvailable` |
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
- **After fall-back:** policies stay applied until the OS lock is confirmed, so there's no window of reverted policies on an open desktop. They're then reverted from the journal. When the owner signs back in, the App shows "SoftLock switched to the Windows lock at <time> because <reason>."
- **Service lifecycle:**
  - SCM recovery restarts the Service on crash.
  - While Locked, the Service doesn't accept `SERVICE_CONTROL_STOP`. An uninstall or stop request leads to fail-closed first.
  - If the Service process is killed outright (an administrator-level action), the Lock Host's fall-back covers it.

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

The journal is a JSON document in a directory that only SYSTEM and Administrators can write. It holds, for each change, the target, the previous value (or "absent"), and the new value. It's written and flushed before each change. Revert restores the previous values exactly, preserving administrator-set policy, and is idempotent.

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

### 10. Keep awake and online

- While Locked, the Service holds `PowerCreateRequest`/`PowerSetRequest(PowerRequestSystemRequired)`. The Lock Host also calls `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` as a belt-and-braces measure.
- **Modern Standby** is detected with `GetPwrCapabilities` (`AoAc`). On those devices, turning the display off can push the system into standby, where the Desktop Activity Moderator throttles desktop apps. With `keep_display_on = "auto"`, SoftLock also holds `PowerRequestDisplayRequired`, so the panel stays on and shows the opaque or dim overlay. *Prototype P5 measures whether this is enough with the lid closed.*
- Network: no aggressive changes in v1. The Service watches connectivity with `INetworkListManager` events, logs drops and reconnects, and reports them in status. "Keep online" amounts to preventing sleep plus reporting.
- All requests are released on unlock, and all power-scheme edits are journaled and reverted.

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
- **Schema:** generated from the Rust types (`schemars`), published as JSON Schema, and used by the App UI and by agents. `schema_version` allows migrations.

```rust
#[derive(Serialize, Deserialize, JsonSchema)] #[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,                  // 1
    pub lock: LockSection,                    // hotkey: String = "Ctrl+Alt+Shift+L", display: Display, message: Option<String>, show_clock: bool
    pub unlock: UnlockSection,                // methods: Vec<MethodId> (ordered, ≥1), softlock_pin_enabled: bool
    pub keep_awake: KeepAwakeSection,         // enabled: bool = true, keep_display_on: Auto|Always|Never
    pub network: NetworkSection,              // keep_online: bool = true, notify_on_drop: bool
    pub takeover: TakeoverSection,            // win_l: bool = false, idle_lock: bool = false, idle_minutes: u16 = 10, lid_close: bool = false
    pub hardening: HardeningSection,          // disable_touch_pen: true, disable_game_controllers: true, uac_require_credentials: true,
                                              // suppress_notifications: true, stop_accessibility_input: true, power_button: DoNothing|Default,
                                              // keyboard_filter: Auto|Off
    pub updates: UpdatesSection,              // channel: Stable|Dev, auto_check: bool = true
}
pub enum Display { Opaque, Dimmed { percent: u8 }, Transparent }
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
  7. Keep awake and online.
  8. Takeover options (all off, each explained).
  9. Disk-encryption check.
  10. Try it: a guided 10-second test lock.
- **Tray menu:** Lock now, Lock for real, Open settings, status line, Quit (Quit doesn't stop the Service).
- The native lock-screen overlay is **not** a webview. It's drawn with Direct2D. The App only *previews* it, by asking the Lock Host to render a preview into a normal window.

### 14. Installer, service transparency and updates

- **Installer:** a WiX MSI (per-machine) that installs the Service, Lock Host, Broker and App, the autostart entry, and an uninstall custom action that refuses while Locked and otherwise reverts all journals and takeover changes. The installer UI and the App's "About the service" page show the same plain-language text: what the Service is, what it changes while locked (the hardening table above), what it never does (no network except update checks, no telemetry, no reading your files), and how to remove it.
- **Updater:**
  - The App checks a static manifest per channel (`stable`, `dev`) hosted on GitHub Releases. The manifest and every artifact are signed with an Ed25519 (minisign) key held by the maintainer. This is independent of Authenticode, so it's free and required from day one.
  - The App downloads the update and asks the Service to install it.
  - The **Service verifies the signature itself** with the embedded public key before running `msiexec` silently. It never installs while Locked and refuses downgrades unless the request carries an explicit user-confirmed downgrade flag.
  - The update key's private half never sits in CI secrets for the stable channel. Stable releases are signed locally by the maintainer. Dev-channel builds use a separate key.
- **Unsigned start:** releases ship without Authenticode signing, and the README and install guide show the SmartScreen "More info → Run anyway" step. Code signing (and notarization on Mac) is a launch task.
- **Disk encryption:** the Service reads `Win32_EncryptableVolume` (BitLocker and Device Encryption) for the system volume and reports On, Off or Unknown.

### 15. Logging, privacy and diagnostics

- Structured local logs (`tracing`), rotated, readable by Administrators and the owner. They hold state transitions, lock sources, fall-back reasons and hardening apply/revert results. **Secrets, typed characters and window titles are never logged.**
- No telemetry. The only outbound network traffic is update checks, which can be turned off.
- "Export diagnostics" puts logs, version and redacted config into a zip.

### 16. Later platforms (abstraction now, implementation later)

- **macOS 11+ (Apple Silicon and Intel), phase 2:**
  - Root `LaunchDaemon` as the Service and a per-user `LaunchAgent` as the Lock Host.
  - Input guard: a `CGEventTap` at the HID level, classifying events by source (`kCGEventSourceStateID` / source PID). Physical events are swallowed and synthetic ones passed.
  - Overlay: a borderless window at shielding level with `sharingType = .none`, plus presentation options (disable force quit, process switching and session termination, hide the Dock and menu bar).
  - Unlock: `LAContext` (Touch ID or password).
  - Requires Accessibility and Input Monitoring permission, granted by the user.
  - Known risks: Secure Input mode can starve event taps, and ScreenCaptureKit on newer macOS versions may ignore `sharingType`, which needs verifying. The Mac App Store is not a target, because event posting is blocked in the sandbox.
- **Linux, phase 3:**
  - Root daemon plus a session client.
  - The leading design is an **evdev exclusive grab (`EVIOCGRAB`) of physical input devices**. Virtual `uinput` devices used by agents stay ungrabbed, and the daemon reads the grabbed devices for the unlock prompt. This works under both Wayland and X11.
  - `ext-session-lock-v1` is evaluated for the lock surface, but compositors route *all* input to the lock client (including `uinput` devices) and may stop presenting other surfaces. That likely conflicts with agent passthrough, so it may serve only "lock for real".
  - Overlay: a `wlr-layer-shell` overlay where available, with an X11 override-redirect fallback.
  - Capture exclusion has no portable Wayland mechanism and is an open risk.
  - Unlock: PAM.

### 17. Decisions resolved during ticketing (15 November 2026)

Agreed with Henry when the ticket breakdown was approved:
- **Takeover on = weakening change.** Enabling `win_l`, `idle_lock` or `lid_close` (in the App or the file) returns `NeedsConfirmation` and applies only after a Hello or password check.
- **Win+L takeover survives a fall-back.** Fail-closed and hard lock clear `DisableLockWorkstation` to reach the real lock; when the owner signs back in, the Service re-applies it if `win_l` is on.
- **SoftLock PIN before confirmation exists.** Development builds may set the SoftLock PIN without the Hello or password check until the sensitive-change confirmation is built. No release (stable or dev channel) ships before that confirmation lands, enforced by a release gate.
- **Dev install script.** Until the MSI exists, a development install script registers the Service and lays out the binaries so the App role check and the Lock Host spawn work. The MSI replaces it.
- **Admin-only repair command.** The "Repair SoftLock settings" Start menu entry (recovery guide) reverts leftover journals, refuses while Locked, and has no lock or unlock function, so it isn't the out-of-scope CLI.
- **Keyboard Filter deferred.** The optional Keyboard Filter item (section 8) is out of v1 until an Enterprise or Education machine or VM is available to test it.

---
## Testing Decisions

### What makes a good test here

- Tests check **external behavior at public boundaries**: "a lock request leads to physical input being blocked, injected input arriving, the overlay missing from captures, and policies set", not which function called which.
- Internal modules are refactored freely as long as the seam tests pass.
- Every state-machine invariant in Implementation Decision 5 and every hardening row in Decision 8 has at least one test.
- **Red-green TDD** at the seams below, one fresh agent per ticket, following the `implement` workflow.
- Security properties are tested negatively too, for example "no public IPC message sequence reaches `Unlocked` from `Locked`", "config writes while locked are rejected", and "a killed Lock Host ends in the OS lock". Property-based tests (`proptest`) generate random event and request sequences against the state machine.
- **Simulating physical input in CI:** GitHub-hosted runners have no real keyboard, and anything a test injects is flagged as injected. The Lock Host therefore has a **test-only build feature**. With it, injected events whose extra-info field carries a per-run random nonce are classified as physical. Release builds are compiled without the feature, and CI checks the release binaries to make sure the feature's code and marker strings are absent. The ticket that adds this test-only feature must add that release-binary check in the same change. The check is not a later follow-up. This simulates the classification input, not the hook. The real hook and real `SendInput` are still exercised.
- **Prior art:** none yet (new repo). Conventions: Rust unit and integration tests with `cargo test` / `cargo nextest`, `proptest`, `insta` snapshots for IPC JSON and config errors, and Playwright against the Tauri front end.

### Proposed test seams

1. **Lock core (`LockController` public API with the fake platform).** This is the state machine, config validation, unlock verdict handling, backoff, journal, and heartbeat and watchdog rules, driven by events against an in-memory `Fake` implementation of every platform trait. It runs on any OS and covers most logic and all invariants.
2. **Service IPC contract (the named pipe, black box).** The real Service runs on a Windows runner, and tests act as `App`, `LockClient` and an impostor client speaking the documented JSON protocol. This covers roles, rejections (`Locked`, `Unauthorized`, `Invalid`, `Conflict`), the absence of any unlock path, `PutConfig` and confirmation flows, update-manifest verification against a local test server, and event streams.
3. **Windows desktop end to end (probe harness).** A probe app on a Windows runner, with the test-only physical feature, checks what the OS actually does:
   - Injected input reaches the probe window, and simulated-physical input doesn't.
   - Focus is unchanged.
   - Captures (GDI, DXGI duplication, Windows.Graphics.Capture) don't contain the overlay's marker color.
   - The hardening registry values, device states and power requests are applied and then restored exactly.
   - Killing or suspending the Lock Host, or stopping the Service, ends in an OS-locked or disconnected session.
   - Recovery after a simulated crash (journal present at Service start) restores everything.
   The fail-closed cases run last, in their own job, because they end the runner's interactive session.
4. **Settings app UI (front end against a mocked IPC).** The wizard flow, settings validation errors, read-only state while locked, sensitive-change confirmation and updater prompts, tested with Playwright against the Tauri front end and a fake IPC bridge.

Everything else (the Windows backend internals, overlay drawing, hook details) is covered by these seams and doesn't get tests of its own.

### CI

- **GitHub Actions:**
  - `cargo fmt` and `clippy` (deny warnings) plus seam 1 on Linux, Windows and macOS (keeps Core portable).
  - Seams 2 and 3 on `windows-2022` and `windows-2025` runners (Server SKUs with an interactive desktop. This must be confirmed in prototype P7).
  - Seam 4 on Linux.
  - MSI build, plus an install, uninstall and "nothing left behind" check on Windows.
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
8. TeamViewer or AnyDesk control still works (document the result).
Each item is a one-line pass or fail in the release notes.

---

## Out of Scope

- **Per-app allowlists or rules** (which apps may run, show through or get input). This is a planned future feature and is deliberately not designed here, because parts of it may become paid.
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
- Multiple simultaneous interactive users, and RDP-session (non-console) SoftLocking.
- IME composition in the unlock field, and localization beyond English (strings are externalized for later).

---

## Further Notes

### Top technical risks: prototype these first, before building the full v1

| # | Risk | Prototype (spike) | What happens if it fails |
|---|---|---|---|
| **P1** | **Do the Ctrl+Alt+Del policies take effect instantly, without a sign-out, and what does that screen still show?** `DisableTaskMgr`, `NoLogoff`, `DisableChangePassword`, `HideFastUserSwitching` and `DisableLockWorkstation` must hide their options the very next time Ctrl+Alt+Del is pressed, on Windows 10 and 11, Home and Pro. Also check whether `NoClose` hides the power icon, what the accessibility icon can launch, and whether the network icon matters. | A service that sets and clears each value, then manual Ctrl+Alt+Del presses on two machines | If an option needs a sign-out to disappear, that piece can't be fully loophole-free on that edition. We document it, use Keyboard Filter where available, or keep that option and make sure its action can't reach the desktop |
| **P2** | **Physical vs. software input classification gaps.** Will a SYSTEM-in-session hook see input to elevated windows? Does blocking at the low-level hook also stop Raw Input (`WM_INPUT`) and apps that read HID devices directly? Do touch, pen and gamepads really need device disabling? Do Voice Access and the on-screen keyboard bypass the guard? Is the hook ever dropped under load? How do remote tools classify (RDP, VMs and KVMs look physical and are blocked; TeamViewer and AnyDesk are injected and pass)? | A hook prototype plus a test matrix of input sources and target apps (elevated window, raw-input app, game, browser) | Each gap gets a hardening item (device disable, process stop) or a documented limitation. Raw Input leakage is the most serious possible result |
| **P3** | **Can the overlay stay on top and private?** Without uiAccess (unsigned builds), a topmost window can be drawn over by the Start menu, Task View, toasts, UAC's secure-desktop dimmed background (which may be a snapshot of the real desktop that ignores capture exclusion), and full-screen exclusive apps. | Open each surface by injected input while locked and photograph the physical screen | Physical input still can't reach them, so this is information exposure, not access. Mitigations: suppress toasts, close shell surfaces when they open, document it. uiAccess becomes possible once the builds are signed |
| **P4** | **Does fail-closed work under the policies?** `LockWorkStation` fails while `DisableLockWorkstation` is set. Check that clearing the policy and then locking, or `WTSDisconnectSession` from session 0 and from the user session, reliably reaches the sign-in screen within 1 s. Also check the Lock Host's own fall-back when the Service is killed. | Kill and suspend tests on a real machine and on a CI runner | This is the core promise. If any path is unreliable, it gets fixed before anything else ships |
| **P5** | **Keep-awake on Modern Standby and with the lid closed.** Do the System- plus Display-required power requests stop standby and Desktop Activity Moderator throttling when the screen is dim or black, and with the lid closed (internal panel off)? | A 60-minute agent run on a Modern Standby laptop: lid open with the screen dimmed, then lid closed, plugged in and on battery | Document hardware limits. `lid_close` stays off by default, and some devices may only be able to keep the lid open |
| P6 | **Unlocking with Windows Hello under the input guard.** The Hello dialog must come up above the overlay and accept the PIN through credential passthrough without opening a hole. `LogonUserW` must work offline for Microsoft accounts (cached credentials). | Prototype the Broker plus passthrough | Fall back to the password plus SoftLock PIN in our own field, and Hello face or fingerprint only |
| P7 | **CI feasibility.** Do GitHub-hosted Windows runners give a usable interactive desktop for low-level hooks, `SendInput`, overlays and capture, and can the Service be installed there? | A one-day CI spike | Use a self-hosted Windows runner for seam 3 |
| P8 | **Antivirus and SmartScreen.** An unsigned input-blocking "screen locker" that runs as SYSTEM is likely to trigger false positives. | Scan release candidates with VirusTotal and Microsoft Defender | Submit false-positive reports to Microsoft. Bring code signing forward if blocking is widespread |

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

- **User guide:** what SoftLock protects against and what it doesn't, unlock methods, takeover options, display modes, and what to do after a fall-back.
- **"What the background service does":** the same text as the installer page and the hardening table.
- **Terms and disclaimer:** Apache-2.0 "as is". It must include: "If SoftLock fails while locked, it switches to the real Windows lock, and your agents will pause until you sign in." It also says SoftLock is not anti-theft and not a substitute for disk encryption.
- **Recovery guide:** how the Service repairs settings after a crash or power loss, and a manual "repair settings" command available from the Start menu for administrators.
- **Contributor guide:** the architecture overview, how to run the seams, the DCO policy (when it's enabled), and the open-core boundary rule.

### Market positioning (for context)

The closest products are Softros Transparent Lock Screen (paid, Windows) and Warden (paid, macOS 15.2+). OpenAI's "Locked use" is macOS-only. SoftLock's space is: free and open source, agent-first, capture-excluded overlay, cross-platform by design, and openly documented security limits.

### Iteration

To change behavior, update this spec first, then the tickets, then the code. The next step is `to-tickets`. The suggested first tickets are prototypes P1–P5 and P7, followed by a tracer bullet: hotkey → Service → Lock Host overlay plus guard → password unlock → revert, with fail-closed.
