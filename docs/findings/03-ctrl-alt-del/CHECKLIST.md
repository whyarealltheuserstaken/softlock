# Ctrl+Alt+Del test

Do this on **your Windows laptop** and on the **school Lenovo ThinkStation P7**.

**Time:** about **1 hour 20 minutes to 1 hour 35 minutes** once Rust is already installed. If Rust or the Visual Studio build tools are missing, add **30 to 45 minutes**. The school computer also needs a conversation with school IT first. That conversation is extra and is not part of the hour.

The program turns six Windows settings on and off. It does not sign you out. It saves the old value of each setting, including a setting that was not there, and it can put that exact value back. If you forget, it puts them back by itself **10 minutes after your last command**. Closing the window does not cancel that.

A restart does **not** turn the six settings off. They stay on until `restore` runs. At the next sign-in, Windows may ask to run the program. Click **Yes**. If you click **No**, the settings stay on. Open an administrator terminal and run `restore` yourself.

`HideFastUserSwitching` (Switch user) is stored in **HKLM**. It changes that setting for **every account on the computer**, not only yours.

Fill the tables in as you go. Copy them into [FINDING.md](FINDING.md) at the end. Leave the decision cells in that file blank. Leave its checkboxes unchecked until both computers are done.

## Results

### Your laptop

Version: ______________ Edition: ______________ OS Build: ______________

| Setting | What you look for | On the first screen? | Hidden right after set? | Back right after clear? | Notes |
|---|---|---|---|---|---|
| DisableTaskMgr | Task Manager | | | | |
| NoLogoff | Sign out | | | | |
| DisableChangePassword | Change a password | | | | |
| HideFastUserSwitching | Switch user | | | | |
| DisableLockWorkstation | Lock | | | | |
| NoClose | Power icon | | | | |

### School Lenovo ThinkStation P7

Version: ______________ Edition: ______________ OS Build: ______________

| Setting | What you look for | On the first screen? | Hidden right after set? | Back right after clear? | Notes |
|---|---|---|---|---|---|
| DisableTaskMgr | Task Manager | | | | |
| NoLogoff | Sign out | | | | |
| DisableChangePassword | Change a password | | | | |
| HideFastUserSwitching | Switch user (every account) | | | | |
| DisableLockWorkstation | Lock | | | | |
| NoClose | Power icon | | | | |

### Windows 10

If neither computer is Windows 10, write **not tested, still needed** here. Do not go looking for another computer today.

Version: ______________ Edition: ______________ OS Build: ______________

| Setting | What you look for | On the first screen? | Hidden right after set? | Back right after clear? | Notes |
|---|---|---|---|---|---|
| DisableTaskMgr | Task Manager | | | | |
| NoLogoff | Sign out | | | | |
| DisableChangePassword | Change a password | | | | |
| HideFastUserSwitching | Switch user | | | | |
| DisableLockWorkstation | Lock | | | | |
| NoClose | Power icon | | | | |

### Icons that are still there when all six settings are on

| Question | Laptop | School P7 |
|---|---|---|
| Is the power icon still there? | | |
| What does its menu show? | | |
| Is the network icon still there? | | |
| Can the network icon open Settings or the desktop? | | |
| Is the accessibility icon still there? | | |
| Narrator: did it open? Could it reach the desktop? | | |
| On-screen keyboard: did it open? Could it type into a normal program? | | |
| Magnifier: did it open? Could it reach the desktop? | | |
| Anything else still on the screen | | |

## What to photograph

Keep the photos on that computer, for example in your Pictures folder. You can attach them to a comment on GitHub issue 3. Do not put the photos in git. Do not commit them. If a screen shows another person's name, do not photograph it. Write "showed other names" instead.

1. The `winver` window (version and edition).
2. The Ctrl+Alt+Del screen before you change anything.
3. The Ctrl+Alt+Del screen after each `set` (six photos).
4. The Ctrl+Alt+Del screen after each `clear` (six photos).
5. The Ctrl+Alt+Del screen with all six settings on.
6. The power-icon menu, if it is still there. Do not shut down.
7. The network-icon popup, if it is still there. Do not change the network.
8. The accessibility menu.
9. Narrator, the on-screen keyboard, and Magnifier, if the menu has them.
10. The Ctrl+Alt+Del screen after `restore`. It should match photo 2.

## Steps

1. On the school computer, ask **school IT (whoever manages the computer)** before you do anything. Their OK has to cover two things: running a program that is not signed, and clicking **More info → Run anyway** if SmartScreen appears. If they say no, write "not allowed" in that table and skip that computer.

2. On your laptop, in a normal terminal:

   ```text
   git clone https://github.com/whyarealltheuserstaken/softlock.git
   cd softlock
   git fetch origin
   git switch cursor/spike-p1-ctrl-alt-del-8f28
   cargo build --release -p spike-ctrl-alt-del
   ```

   If `cargo` is not found, install Rust the way the README says, then run the build again. The program is `target\release\spike-ctrl-alt-del.exe`.

3. Copy that one file onto a USB stick so you can carry it to the school computer. Do not install Rust on the school computer. Do not run the program from the USB stick. The next-sign-in restore remembers the path of the file you actually ran, and a USB stick may not be there later.

4. On each computer, copy the file into a local folder and run it only from there:

   ```text
   mkdir C:\Temp\softlock-spike
   copy spike-ctrl-alt-del.exe C:\Temp\softlock-spike\
   cd C:\Temp\softlock-spike
   ```

   On the school computer, `copy` means copy it off the USB stick into `C:\Temp\softlock-spike`. If the school computer will not run the file, stop and write "could not run the program" in its table.

5. If Windows SmartScreen warns you, choose **More info → Run anyway**. That click is part of what school IT has to allow.

6. On each computer, do steps 7 through 18 from `C:\Temp\softlock-spike`. Use an account that is you, not a different person. Step 19 is only if you are stuck.

7. Right-click the Start button. Click **Terminal (Admin)** or **Windows PowerShell (Admin)**. Click **Yes**. If Windows asks for a password instead of just **Yes**, click **No**, stop, and write "asked for a password". Do not type a password.

   ```text
   cd C:\Temp\softlock-spike
   .\spike-ctrl-alt-del.exe status
   ```

   Read the account lines. They must be your account. If any of the six settings is not `absent`, stop before changing anything and write those values in Notes. On your own laptop, stop and ask the SoftLock bot. On the school computer, stop and ask school IT. If it says you need an Administrator window and clicking Yes does not get you one, write "not an administrator" and stop for that computer.

8. Press the Windows key, type `winver`, and press Enter. Write the version (Windows 10 or Windows 11), the edition (Home, Pro, or the words in the window), and the OS Build number in that computer's table. Take photo 1. Close the window.

9. Press Ctrl+Alt+Del. Take photo 2. This is the "before" screen. In the table, write yes or no for "On the first screen?" for each row. Press **Escape**. Do not click Sign out, Switch user, Lock, Task Manager, or Shut down.

10. For each name in this list, do steps 11, 12, and 13 before you go to the next name.

    - `DisableTaskMgr` — Task Manager
    - `NoLogoff` — Sign out
    - `DisableChangePassword` — Change a password
    - `HideFastUserSwitching` — Switch user. This one is HKLM and affects every account on the computer.
    - `DisableLockWorkstation` — Lock
    - `NoClose` — the power icon

11. In the administrator terminal, run (use the name you are on):

    ```text
    .\spike-ctrl-alt-del.exe set DisableTaskMgr
    ```

    The program prints the old value and says it read `dword 1` back. Each command starts the 10 minutes over. Do not wait more than 10 minutes between commands. If it says the safety timer already put every setting back, run that same command again.

12. Press Ctrl+Alt+Del. Do not sign out. Did that one thing disappear? Write yes or no under "Hidden right after set?". If it was not on the first screen, write "not on the first screen". Take a photo. Press **Escape**.

13. Run the same name with `clear`:

    ```text
    .\spike-ctrl-alt-del.exe clear DisableTaskMgr
    ```

    Press Ctrl+Alt+Del again. Did that thing come back right away, with no sign-out? Write yes or no under "Back right after clear?". Take a photo. Press **Escape**. If a setting will not stay, or comes back by itself within a minute, write "came back by itself" in Notes and go on. Do not keep fighting it. The school computer may put its own settings back.

14. After all six names, put the computer back, then turn all six on for the icon step. This step can take a while, so give it 20 minutes. 30 is the longest the program allows. Run `status` before each photo in this step so you know the timer has not already put the settings back.

    ```text
    .\spike-ctrl-alt-del.exe restore
    .\spike-ctrl-alt-del.exe set all --minutes 20
    .\spike-ctrl-alt-del.exe status
    ```

15. Press Ctrl+Alt+Del. Take photo 5. Then, still on that screen:

    - If you see a **power** icon, click it only to open the menu. Take a photo. Write what the menu shows. Do not click Shut down, Restart, or Sleep. Press **Escape**.
    - If you see a **network** icon, click it. Take a photo. Do not change the network. Write whether it can open Settings or the desktop. Press **Escape**.
    - If you see an **accessibility** icon, click it. Take a photo of the menu. Open Narrator, the on-screen keyboard, and Magnifier one at a time, if they are in the menu. For each one, write whether it opened and whether you could type, click, or show the normal desktop. Close it before you open the next one. Narrator turns off with **Caps Lock + Esc**. Magnifier turns off with the **Windows key + Esc**.

16. Press **Escape** until you are back at the normal desktop. Run:

    ```text
    .\spike-ctrl-alt-del.exe restore
    ```

    A window stays open until you press Enter. It should say the read-back matched the saved values. The same text is in `C:\ProgramData\spike-ctrl-alt-del\restore-result.txt`. Press Ctrl+Alt+Del one last time. It should look like photo 2. Take photo 10. Press **Escape**.

17. Run `status` again. It must say **Nothing is waiting to be put back.** If it does not, run `restore` again and check `status` once more. Do not leave the computer until it says that.

18. After that status says nothing is waiting, clean up from the administrator terminal. Do this on your laptop and on the school computer. If you want to keep the result, copy it to the Desktop first. Then delete both folders:

    ```text
    copy C:\ProgramData\spike-ctrl-alt-del\restore-result.txt %USERPROFILE%\Desktop
    rmdir /s /q C:\Temp\softlock-spike
    rmdir /s /q C:\ProgramData\spike-ctrl-alt-del
    ```

    This is the last thing you do on that computer before you walk away.

19. If you are stuck and `restore` fails, do not delete `C:\ProgramData\spike-ctrl-alt-del\journal.json`. Send that file to the SoftLock bot first. Paste it in the chat, or attach it. From the administrator terminal, copy it to the Desktop with:

    ```text
    copy C:\ProgramData\spike-ctrl-alt-del\journal.json %USERPROFILE%\Desktop
    ```

    A restart still leaves all six settings on until restore runs.

    Only if restore still fails, open Registry Editor and delete **only** the values that the first `status` showed as `absent`. If the first status showed anything other than `absent`, do not delete that value.

    - `HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\System`
      - `DisableTaskMgr`
      - `DisableChangePassword`
      - `DisableLockWorkstation`
    - `HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer`
      - `NoLogoff`
      - `NoClose`
    - `HKLM\Software\Microsoft\Windows\CurrentVersion\Policies\System`
      - `HideFastUserSwitching`
    - `HKCU\Software\Microsoft\Windows\CurrentVersion\RunOnce`
      - `spike-ctrl-alt-del`

    On the school P7, after that manual cleanup, run `gpupdate /force`.

20. Copy both tables, the icon table, and the photo file names into [FINDING.md](FINDING.md). Leave every decision cell blank. Leave the checkboxes at the bottom of that file unchecked. Attach the photos to a comment on issue 3 if you want them with the finding. Do not commit them.
