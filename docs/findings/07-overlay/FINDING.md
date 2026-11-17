# Spike P3 finding: can the overlay stay on top and private without uiAccess?

Status: waiting for Henry's photos. Do not fill the results in from a CI run or from a guess. The smoke test only checks that the proof window is topmost, click-through, capture-excluded, and not blocking keys. It does not answer this spike.

Issue: #7. Dependents that must read this file before they are built: #18 and #30.

## Question

v1 is unsigned, so it cannot use uiAccess. uiAccess is the Windows flag that puts a signed window above the Start menu and some other shell surfaces. Without it, the overlay is an ordinary topmost window.

While that window is up, which of these draw above it, and which of the four switches actually hide them?

- Start menu
- Task View
- Notification center (Action Center on Windows 10)
- A toast banner
- The User Account Control dim screen (is that dim a snapshot of the real desktop that still shows the overlay, or does capture exclusion hide the overlay from it?)
- A full-screen exclusive window
- A monitor plugged in while the overlay is up

Whether those surfaces can still be clicked is the input guard's job (ticket #30). This overlay is click-through, so this spike does not judge that.

## What the proof is

`spikes/overlay` is an unsigned proof. It is not installed, and no product crate depends on it. `SHIPS_IN_RELEASE` is false.

The window uses topmost, layered, click-through, no-activate, and tool-window styles, plus `WDA_EXCLUDEFROMCAPTURE`. It covers every monitor. It does not block typing or clicking. It closes on Ctrl+Alt+Shift+Q and after a timeout (5 minutes unless the command says otherwise).

The 5-minute stop is an in-process thread. Killing the process also removes the windows. The green full-screen child stops itself within 30 seconds.

The switches, each off unless named on the command line:

- `--reassert-timer` puts the window back on top every 250 ms
- `--reassert-pos` puts it back on top when its position changes
- `--close-shell` dismisses Start, Task View, the notification center, Quick settings, and the Project menu by sending Escape, or by sending the same chord again. It does not post a close message to Explorer or any other shell host. If the shell gets stuck, open Task Manager and restart Windows Explorer, or sign out and back in
- `--suppress-toasts` writes only the current user's `ToastEnabled` setting. It does not write a policy value. If a toast policy value already exists, it refuses and says `school IT controls this, skip`. Before it changes the setting it writes a RunOnce entry, `"<exe>" restore-toasts`, so a crash, a kill, or a reboot runs the copied exe and puts toasts back. After a restore it deletes that entry and the empty sentinel key

`spikes/overlay/open-surfaces.ps1` injects the keys for the seven surfaces. It looks for `spike-overlay.exe` in the same folder first, so the school PC does not need Rust. The User Account Control step launches `cmd.exe /c exit` and never sends an accept key. The full-screen step is a green Direct3D window that leaves exclusive mode on Esc and never stays up longer than 30 seconds. Run, tour, smoke, and restore-toasts refuse to run elevated unless `--allow-admin` is passed. The fullscreen child stays allowed so the tour can still open the green window.

## Results

Copy the filled table from `docs/findings/07-overlay/CHECKLIST.md` after both computers are done. Until then every cell stays blank.

| Surface | School PC, no fixes | School PC, fixes on | Laptop, no fixes | Laptop, fixes on |
| --- | --- | --- | --- | --- |
| Start menu | | | | |
| Task View | | | | |
| Notification center | | | | |
| Toast banner | | | | |
| User Account Control dim screen | | | | |
| Green full-screen window | | | | |
| Project menu (Win+P) | | | | |
| Monitor plugged in on the school PC | | | | |

Allowed words, same as the checklist: `covered`, `drawn above`, `mitigated by X`.

### Did capture exclusion survive the User Account Control dim?

| Machine | What the phone showed in the dim area |
| --- | --- |
| School PC | |
| Laptop | |

### Monitor plug-in

| What happened | Answer |
| --- | --- |
| The new screen was pink | |
| How long that took | |
| Anything from the desktop was visible on it | |

## Mitigations to build

#18 builds the mitigations this spike shows are worth having. Leave each line blank until the table above is filled. Write "build" or "do not build", and one sentence why.

- Re-assert topmost on a timer:
- Re-assert topmost on window-position change:
- Close shell surfaces:
- Suppress toast banners while locked, and restore the previous values on unlock:

## Limitations to document

#30 writes the user guide, including what can still appear above the overlay, and the input guard that decides whether a bystander can use what they see. Leave these blank until the photos exist.

- Surfaces that can show above the overlay:
- User Account Control dim screen:
- Full-screen exclusive apps:
- A monitor connected while locked:
- Unsigned build, no uiAccess:

## Acceptance

These are Henry's checks. Leave them unchecked in this file.

- [ ] Proof overlay build plus an injected-input script that opens each surface; Henry photographs each case (manual, Henry's hardware, photos attached)
- [ ] Each surface marked: covered / drawn above / mitigated by X (manual, Henry's hardware)
- [ ] Written finding states the mitigations to build and the limitations to document, and names dependents 18 and 30
