# Checklist: does the pink layer stay on top?

You are Henry. This is a test, not the real lock. A pink layer covers the screen. You find out what Windows draws on top of it. You write the answer in the table at the bottom. You take the photos with your phone.

Do this on the school PC (the ThinkStation P7, the one with more than one screen) and on your laptop. The school PC is about 45 minutes. The laptop is about 30 minutes. You can stop in the middle. The pink layer closes itself.

## 1. Words you need

- **Pink layer.** The test window. It covers every screen. You can still click and type. Clicks go through it.
- **Exit.** Press Ctrl+Alt+Shift+Q together. The pink layer also closes itself after the time you set. If you set nothing, that time is 5 minutes.
- **Phone photo.** A picture from your phone of the real screen. The Print Screen key, the Snipping Tool, and Xbox Game Bar all hide the pink layer. Those pictures are useless for this test. Point the phone so the photo shows the plastic edge of the monitor. That proves it is a photo of the screen, not a screenshot.
- **Never click Yes.** One step opens a User Account Control window. Click **No**. Yes would only run a command that exits at once, and you still do not click it.

If the pink layer is still up and the exit keys do nothing, wait. It closes itself. You can also click the black PowerShell window (clicks go through the pink) and press Ctrl+C.

## 2. Get ready (both computers)

1. Plug the laptop in if you are on the laptop. Unlock the phone and open the camera before you start. You get about 25 seconds for each photo.
2. Open PowerShell. Go to the folder that has this project. The prompt should be in that folder.
3. Build the test:

```powershell
cargo build -p softlock-spike-overlay
```

4. Practice the exit. This pink layer stays for 60 seconds, or until you press the exit keys:

```powershell
cargo run -p softlock-spike-overlay -- run --timeout-secs 60
```

5. Press Ctrl+Alt+Shift+Q. The pink layer must go away. If it does not, wait out the 60 seconds. Do not turn the computer off.

## 3. School PC (ThinkStation P7)

The school PC has more than one screen. Every screen should be pink. If one screen is a normal desktop, write that down. That is a failed cover.

### 3.1 First pass: no fixes

This shows what Windows draws above the pink when nothing is fighting it.

```powershell
powershell -ExecutionPolicy Bypass -File spikes\overlay\open-surfaces.ps1 -PauseSecs 25 -TimeoutSecs 300
```

The pink text names the step. PowerShell prints the same line. For each step, take a phone photo, then write one of these in the table:

- `covered` — the pink layer is on top of that thing
- `visible-but-not-interactive` — you can see the thing above the pink, and clicking it does not use it (the click goes through)
- `mitigated by X` — only use this on the second pass, when a fix made it go away. X is `timer`, `window-position`, `close-shell`, or `suppress-toasts`

The seven steps, in order:

1. **Start menu.** The Windows key is pressed for you.
2. **Task View.** Win+Tab is pressed for you.
3. **Notification center.** Win+N is pressed for you on Windows 11. On Windows 10 it is Win+A, and the window is called Action Center.
4. **Toast banner.** A small banner from this test. It is usually at a corner.
5. **User Account Control.** The screen dims. Click **No** after the photo. Never click Yes.
6. **Green full-screen window.** It is bright green. Press Esc if you want it gone. It also closes itself within 30 seconds. Photograph whether the pink layer is still there or the green replaced it.
7. **Project menu.** Win+P is pressed for you. This is only a stand-in. The real plug-in is the next section.

### 3.2 Plug a screen in

1. Start a quiet pink layer for 3 minutes:

```powershell
cargo run -p softlock-spike-overlay -- run --timeout-secs 180
```

2. Plug a monitor into the school PC while the pink layer is up.
3. Photograph the new screen with your phone. Write whether it turned pink, and how long that took.
4. Unplug it if it was not connected before. Press Ctrl+Alt+Shift+Q.

### 3.3 Second pass: fixes on

Same seven steps. The fixes are on: the pink layer keeps jumping back on top, Start and Task View and the notification center get closed, and toast banners are turned off until the test exits.

```powershell
powershell -ExecutionPolicy Bypass -File spikes\overlay\open-surfaces.ps1 -PauseSecs 25 -TimeoutSecs 300 -ReassertTimer -ReassertPos -CloseShell -SuppressToasts
```

If a step that was visible last time is gone, write `mitigated by` and the fix you think did it. If you are not sure which fix, write `mitigated by one of the four switches` and say what you saw.

## 4. Laptop

Do section 2 again on the laptop if you have not built it there.

The laptop pass is mainly the toast and the dim User Account Control screen. A laptop panel can show those differently from the school PC. Do the same two tours:

1. No fixes. Same command as section 3.1.
2. Fixes on. Same command as section 3.3.

You do not need to plug in a monitor on the laptop. If the green window does not go full screen, write `did not go exclusive` and still take the photo.

Click **No** on User Account Control. Never click Yes.

## 5. If toasts stay off

The toast switch is put back when the test exits. If the test crashes and banners stay gone, run this, then check that a normal app can show a banner again:

```powershell
cargo run -p softlock-spike-overlay -- restore-toasts
```

## 6. What to photograph

Take these with your phone. One picture can cover more than one line if every screen is in the frame.

1. All of the school PC's screens at once, pink, before any menu opens. Include the bezels.
2. Each of the seven steps on the school PC, fixes off. For the Start menu, Task View, notification center, toast, and Project menu, get the thing and some pink in the same photo.
3. The dim User Account Control screen on the school PC and on the laptop. Get the whole panel, including the dim area, not just the small dialog.
4. The green full-screen window on the school PC.
5. The new monitor at the moment you plug it in, and again once it has settled.
6. The same steps on the laptop, especially the toast and User Account Control.
7. Any step that changed when the fixes were on. Photograph the "after" so the table is not only a sentence.

If a step does not appear, photograph the pink layer anyway and write `did not appear` in the notes.

## 7. Results

Leave this table blank until you have done the test. Write only `covered`, `visible-but-not-interactive`, or `mitigated by X`.

| Surface | School PC, no fixes | School PC, fixes on | Laptop, no fixes | Laptop, fixes on | Photo file |
| --- | --- | --- | --- | --- | --- |
| Start menu | | | | | |
| Task View | | | | | |
| Notification center | | | | | |
| Toast banner | | | | | |
| User Account Control dim screen | | | | | |
| Green full-screen window | | | | | |
| Project menu (Win+P) | | | | | |
| Monitor plugged in on the school PC | | | | | |

Notes (what surprised you, which screen was wrong, anything you could still click):

| Machine | Notes |
| --- | --- |
| School PC | |
| Laptop | |
