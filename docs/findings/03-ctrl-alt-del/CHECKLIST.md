# Ctrl+Alt+Del test

Do this on **your Windows laptop** and on the **school Lenovo ThinkStation P7**.

**Time:** about 10 minutes to build the program the first time, then about 25 minutes on each computer. Both computers together are about **1 hour and 10 minutes**, after a teacher says the school computer is OK to use.

The program turns six Windows settings on and off. It does not sign you out. It saves the old value of each setting, including a setting that was not there, and it can put that exact value back. If you forget, it puts them back by itself **10 minutes after your last command**. Closing the window does not cancel that. If the computer restarts first, the next sign-in asks to put the settings back. Click **Yes**.

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
| HideFastUserSwitching | Switch user | | | | |
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

Save the photos in a folder on that computer. Write the file name in the Notes cell. Do not put the photos in git. If a screen shows another person's name, do not photograph it. Write "showed other names" instead.

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

1. Ask a teacher before you do anything on the school computer. If they say no, write "not allowed" in that table and skip that computer.

2. On your laptop, in a normal terminal:

   ```text
   git clone https://github.com/whyarealltheuserstaken/softlock.git
   cd softlock
   git fetch origin
   git switch cursor/spike-p1-ctrl-alt-del-8f28
   cargo build --release -p spike-ctrl-alt-del
   ```

   If `cargo` is not found, install Rust the way the README says, then run the build again. The program is `target\release\spike-ctrl-alt-del.exe`.

3. Copy that one file to a USB stick for the school computer. Do not install Rust on the school computer. If the school computer will not run the file, stop and write "could not run the program" in its table.

4. If Windows SmartScreen warns you, choose **More info → Run anyway**.

5. On each computer, do steps 6 through 18. Use an account that is you, not a different person.

6. Right-click the Start button. Click **Terminal (Admin)** or **Windows PowerShell (Admin)**. Click **Yes**.

   ```text
   cd C:\path\to\the\folder\with\the\exe
   .\spike-ctrl-alt-del.exe status
   ```

   If it says you need an Administrator window and clicking Yes does not get you one, write "not an administrator" and stop for that computer. Do not guess a password.

7. Press the Windows key, type `winver`, and press Enter. Write the version (Windows 10 or Windows 11), the edition (Home, Pro, or the words in the window), and the OS Build number in that computer's table. Take photo 1. Close the window.

8. Press Ctrl+Alt+Del. Take photo 2. This is the "before" screen. In the table, write yes or no for "On the first screen?" for each row. Press **Escape**. Do not click Sign out, Switch user, Lock, Task Manager, or Shut down.

9. For each name in this list, do steps 10, 11, and 12 before you go to the next name.

   - `DisableTaskMgr` — Task Manager
   - `NoLogoff` — Sign out
   - `DisableChangePassword` — Change a password
   - `HideFastUserSwitching` — Switch user
   - `DisableLockWorkstation` — Lock
   - `NoClose` — the power icon

10. In the administrator terminal, run (use the name you are on):

    ```text
    .\spike-ctrl-alt-del.exe set DisableTaskMgr
    ```

    The program prints the old value and says it read `dword 1` back. Each command starts the 10 minutes over. Do not wait more than 10 minutes between commands. If it says the safety timer already put every setting back, run that same command again.

11. Press Ctrl+Alt+Del. Do not sign out. Did that one thing disappear? Write yes or no under "Hidden right after set?". If it was not on the first screen, write "not on the first screen". Take a photo. Press **Escape**.

12. Run the same name with `clear`:

    ```text
    .\spike-ctrl-alt-del.exe clear DisableTaskMgr
    ```

    Press Ctrl+Alt+Del again. Did that thing come back right away, with no sign-out? Write yes or no under "Back right after clear?". Take a photo. Press **Escape**.

13. After all six names, put the computer back, then turn all six on together:

    ```text
    .\spike-ctrl-alt-del.exe restore
    .\spike-ctrl-alt-del.exe set all
    ```

14. Press Ctrl+Alt+Del. Take photo 5. Then, still on that screen:

    - If you see a **power** icon, click it only to open the menu. Take a photo. Write what the menu shows. Do not click Shut down, Restart, or Sleep. Press **Escape**.
    - If you see a **network** icon, click it. Take a photo. Do not change the network. Write whether it can open Settings or the desktop. Press **Escape**.
    - If you see an **accessibility** icon, click it. Take a photo of the menu. Open Narrator, the on-screen keyboard, and Magnifier one at a time, if they are in the menu. For each one, write whether it opened and whether you could type, click, or see the normal desktop. Close it before you open the next one. Narrator turns off with **Caps Lock + Esc**. Magnifier turns off with the **Windows key + Esc**.

15. Press **Escape** until you are back at the normal desktop. Run:

    ```text
    .\spike-ctrl-alt-del.exe restore
    ```

    It should say the read-back matched the saved values. Press Ctrl+Alt+Del one last time. It should look like photo 2. Take photo 10. Press **Escape**.

16. If a setting will not stay, or comes back by itself within a minute, write "came back by itself" in Notes and go on. Do not keep fighting it. The school computer may put its own settings back.

17. If you are stuck and a button is still missing, open an administrator terminal and run `.\spike-ctrl-alt-del.exe restore`. If the computer restarted, sign in, open an administrator terminal, and run that same command. If Windows asks "Do you want to allow this app to make changes?", click **Yes**.

18. Copy both tables, the icon table, and the photo file names into [FINDING.md](FINDING.md). Leave every decision cell blank. Leave the checkboxes at the bottom of that file unchecked.
