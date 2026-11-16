# Checklist: does the pink layer stay on top?

You are Henry. This is a test, not the real lock. A pink layer covers the screen. You find out what Windows draws on top of it. You write the answer in the table at the bottom. You take the photos with your phone.

Do the laptop first. Then do the school PC (the ThinkStation P7, the one with more than one screen).

How long it takes:

- The conversation with school IT is extra. Do that before the school PC.
- Laptop: 30-40 minutes. Add 30-45 minutes if git, rustup 1.100.0, or the Visual Studio C++ Build Tools are missing.
- School PC: 45-60 minutes. Do not install Rust there.

You can stop in the middle. The pink layer closes itself.

## 1. Ask school IT first

Before the school PC, ask the person who manages that computer. That is school IT, not a teacher.

You need permission to run an unsigned program. Windows SmartScreen may say "Windows protected your PC". Do not click through that until school IT says you may. After they say yes, the click is More info, then Run anyway.

Do not install git, rustup, or Visual Studio on the school PC.

## 2. Words you need

- **Pink layer.** The test window. It covers every screen. You can still click and type. Clicks go through it.
- **Exit.** Press Ctrl+Alt+Shift+Q together. The pink layer also closes itself after the time you set. If you set nothing, that time is 5 minutes.
- **Phone photo.** A picture from your phone of the real screen. The Print Screen key, the Snipping Tool, and Xbox Game Bar all hide the pink layer. Those pictures are useless for this test. Point the phone so the photo shows the plastic edge of the monitor. That proves it is a photo of the screen, not a screenshot.
- **Never click Yes.** One step opens a User Account Control window. Click **No**. Yes would only run a command that exits at once, and you still do not click it.
- **What you write.** `covered` or `drawn above`. On the second pass you may also write `mitigated by X`. X is `timer`, `window-position`, `close-shell`, or `suppress-toasts`. Do not write whether you could click the thing. Clicks go through the pink layer, so this test cannot tell. A later test (the input guard) answers that.

If the pink layer is still up and the exit keys do nothing, wait. It closes itself. You can also click the black PowerShell window (clicks go through the pink) and press Ctrl+C.

If the program says it is running as admin, close it and open a normal PowerShell. Do not choose Run as administrator. The tour refuses that unless you pass `--allow-admin`, and you only pass that if school IT told you to.

## 3. Get the laptop ready

You need these on the laptop only:

- git
- rustup, with Rust 1.100.0
- the Visual Studio C++ Build Tools

If any of those are missing, installing them takes about 30-45 minutes. Then:

1. Plug the laptop in. Unlock the phone and open the camera before you start. You get about 25 seconds for each photo.
2. Open PowerShell. Go to the folder that has this project.
3. Build the test. This is the command. It makes a release exe:

```powershell
cargo build --release -p softlock-spike-overlay
```

4. Copy these two files into a folder you can carry, for example `C:\Users\<you>\softlock-test`:

- `target\release\spike-overlay.exe`
- `spikes\overlay\open-surfaces.ps1`

The script looks for `spike-overlay.exe` in the same folder first. Every command below, including `restore-toasts`, works from that copied exe. You do not need cargo once the exe is copied.

5. Practice the exit from that folder. This pink layer stays for 60 seconds, or until you press the exit keys:

```powershell
cd C:\Users\<you>\softlock-test
.\spike-overlay.exe run --timeout-secs 60
```

6. Press Ctrl+Alt+Shift+Q. The pink layer must go away. If it does not, wait out the 60 seconds. Do not turn the computer off.

## 4. Laptop tours

Do these on the laptop before the school PC. The laptop pass is mainly the toast and the dim User Account Control screen. A laptop panel can show those differently from the school PC.

### 4.1 First pass: no fixes

This shows what Windows draws above the pink when nothing is fighting it.

```powershell
cd C:\Users\<you>\softlock-test
powershell -ExecutionPolicy Bypass -File .\open-surfaces.ps1 -PauseSecs 25 -TimeoutSecs 300
```

The pink text names the step. PowerShell prints the same line. For each step, take a phone photo, then write `covered` or `drawn above` in the table.

The seven steps, in order:

1. **Start menu.** The Windows key is pressed for you.
2. **Task View.** Win+Tab is pressed for you.
3. **Notification center.** Win+N is pressed for you on Windows 11. On Windows 10 it is Win+A, and the window is called Action Center.
4. **Toast banner.** A small banner from this test. It is usually at a corner.
5. **User Account Control.** The screen dims. Click **No** after the photo. Never click Yes.
6. **Green full-screen window.** It is bright green. Press Esc if you want it gone. It also closes itself within 30 seconds. Photograph whether the pink layer is still there or the green replaced it.
7. **Project menu.** Win+P is pressed for you. This is only a stand-in. The real plug-in is on the school PC.

You do not need to plug in a monitor on the laptop. If the green window does not go full screen, write `did not go exclusive` in the notes and still take the photo.

### 4.2 Second pass: fixes on

Same seven steps. The fixes are on: the pink layer keeps jumping back on top, Start and Task View and the notification center are dismissed with Escape (Explorer is not closed), and the user toast setting is turned off until the test exits.

```powershell
powershell -ExecutionPolicy Bypass -File .\open-surfaces.ps1 -PauseSecs 25 -TimeoutSecs 300 -ReassertTimer -ReassertPos -CloseShell -SuppressToasts
```

On the toast step, look for a banner. If a banner still appears, write `not suppressed until sign-out` in the notes. Do not sign out in the middle of the tour. The toast setting sometimes applies only after you sign out.

If a step that was drawn above last time is gone, write `mitigated by` and the fix you think did it. If you are not sure which fix, write `mitigated by one of the four switches` and say what you saw.

## 5. School PC (ThinkStation P7)

School IT has already said yes, including the unsigned exe and SmartScreen. The two files are in a local folder, for example `C:\Users\<you>\softlock-test`. Open PowerShell there. Do not install Rust.

The school PC has more than one screen. Every screen should be pink. If one screen is a normal desktop, write that down. That is a failed cover.

### 5.1 Practice the exit

```powershell
cd C:\Users\<you>\softlock-test
.\spike-overlay.exe run --timeout-secs 60
```

Press Ctrl+Alt+Shift+Q, or wait out the 60 seconds.

### 5.2 First pass: no fixes

Same command as the laptop first pass:

```powershell
powershell -ExecutionPolicy Bypass -File .\open-surfaces.ps1 -PauseSecs 25 -TimeoutSecs 300
```

Write `covered` or `drawn above` for each step. Take a phone photo of each one.

### 5.3 Plug a screen in

1. Start a quiet pink layer for 3 minutes:

```powershell
.\spike-overlay.exe run --timeout-secs 180
```

2. Plug a monitor into the school PC while the pink layer is up.
3. Photograph the new screen with your phone. Write whether it turned pink, and how long that took.
4. Unplug it if it was not connected before. Press Ctrl+Alt+Shift+Q.

### 5.4 Second pass: fixes on

Same command as the laptop second pass:

```powershell
powershell -ExecutionPolicy Bypass -File .\open-surfaces.ps1 -PauseSecs 25 -TimeoutSecs 300 -ReassertTimer -ReassertPos -CloseShell -SuppressToasts
```

Watch the toast step again. If a banner still appears, write `not suppressed until sign-out`. Do not sign out until the pink layer is gone.

## 6. Put toasts back

The toast setting is put back when the test exits. A crash, a kill, or a reboot also puts it back the next time you sign in, because the program writes a one-time startup entry before it changes the setting.

After both passes, run this from the copied folder and confirm a banner appears again:

```powershell
cd C:\Users\<you>\softlock-test
.\spike-overlay.exe restore-toasts
.\spike-overlay.exe tour --only Toast --no-prompt --pause-secs 15 --timeout-secs 40
```

You should see a banner. Photograph it if the earlier toast photos were missing. If no banner appears, write that in the notes and tell school IT. Do not keep trying.

## 7. If you get stuck

- Pink layer will not go: press Ctrl+Alt+Shift+Q. Or click the black window and press Ctrl+C. Or wait. It closes itself. Do not turn the computer off.
- Taskbar is gone, or Start will not open: press Ctrl+Shift+Esc to open Task Manager, then restart Windows Explorer. Or sign out and back in.
- Green window will not go: press Esc, or wait 30 seconds. It stops itself.
- Toasts stay off: from the copied folder run `.\spike-overlay.exe restore-toasts`. Then do the toast command in section 6 and confirm a banner appears.
- The program says `school IT controls this, skip`: school IT already set a toast policy. Skip the toast-suppression part and write that in the notes. Do not try to change the policy.
- User Account Control is up: click **No**. Never click Yes.

## 8. What to photograph

Take these with your phone. One picture can cover more than one line if every screen is in the frame.

1. All of the school PC's screens at once, pink, before any menu opens. Include the bezels.
2. Each of the seven steps on the school PC, fixes off. For the Start menu, Task View, notification center, toast, and Project menu, get the thing and some pink in the same photo.
3. The dim User Account Control screen on the school PC and on the laptop. Get the whole panel, including the dim area, not just the small dialog.
4. The green full-screen window on the school PC.
5. The new monitor at the moment you plug it in, and again once it has settled.
6. The same steps on the laptop, especially the toast and User Account Control.
7. Any step that changed when the fixes were on. Photograph the "after" so the table is not only a sentence.
8. A toast banner after `restore-toasts`, so you know banners work again.

If a step does not appear, photograph the pink layer anyway and write `did not appear` in the notes.

## 9. Results

Leave this table blank until you have done the test. Write only `covered`, `drawn above`, or `mitigated by X`.

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

Notes (what surprised you, which screen was wrong, whether a toast still appeared while suppression was on):

| Machine | Notes |
| --- | --- |
| School PC | |
| Laptop | |
