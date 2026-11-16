# Spike P6 checklist: Windows Hello and the offline password check

This is for Henry, on his own Windows laptop that already has Windows Hello. The runs below take about 35 to 45 minutes. Installing the tools, if they are missing, takes another 20 to 30 minutes.

Do this before anything else.

**This test blocks your keyboard and mouse.** While the dark screen is up, ordinary keys and the mouse do nothing.

**It always stops by itself after at most 3 minutes.** You do not have to do anything for that to happen.

**Physical escape, and it does not need Hello to work:** hold Ctrl, Alt, and Shift, then press Q. The dark screen goes away and your keyboard comes back.

**If the tool crashes, it takes the block off.** If the escape chord does not work and the dark screen is still there, press Ctrl+Alt+Del. Windows handles that before any app, so it still works. Choose Cancel, or sign out if you need to. If keys are still dead after the window is gone, wait until 3 minutes from the start have passed.

**If you are stuck:** press Ctrl+Alt+Del, open Task Manager, and end `hello-spike.exe`.

Save your work first. Close games. Open Notepad and leave it in front. You will use it to see whether a typed key lands.

The tool is not the product. It is a one-off proof.

Leave every box in `FINDING.md` empty until you have finished. Do not tick Go or Fallback early.

## Before you build

You need Git, rustup with Rust toolchain 1.100.0 (the project file picks that version), and the Visual Studio C++ Build Tools with the "Desktop development with C++" workload. If those are already installed, skip this. If they are missing, installing them takes about 20 to 30 minutes.

If a build fails, copy the error text and paste it to the SoftLock bot.

## Build it (about 5 minutes)

From the project folder, in a terminal:

```text
cargo run -p softlock-hello-spike -- hello
```

You get 5 seconds to press Ctrl+C before the block starts. Read the dark screen. It repeats the escape chord.

If Windows SmartScreen warns you, the build is unsigned. Choose More info, then Run anyway.

When a run ends, the terminal prints `keyboard and mouse restored`. Type in Notepad and confirm the letters appear. Then start the next run.

## 1. Face or fingerprint (about 5 minutes)

1. Uncover the camera. If you use a fingerprint, rest your finger when the prompt asks.
2. Run `cargo run -p softlock-hello-spike -- hello`.
3. Write down whether the Hello window was fully visible above the dark screen, or whether the dark screen covered it.
4. Write down the line that starts with `hello result:` or `hello availability:`.
5. Also write down the `signature file=` lines and the `foreground pid=` line.
6. If Hello is not set up, stop this section and write "not enrolled". Skip the PIN runs.

If you have both face and fingerprint, do this once for each. If you only have one, write "not enrolled" for the other.

## 2. PIN through the passthrough (about 4 minutes)

Cover the camera and do not touch the fingerprint reader, so Hello asks for the PIN.

1. Run the hello command again.
2. When the PIN box is in front, the dark screen changes. It says you can type the PIN.
3. Type the right PIN.
4. Write down whether the digits showed up in the Hello window, and whether Hello accepted them.
5. Write down the `hello result:` line and the `signature file=` line. The verify part should say `catalog` for the system credential window.

The mouse does nothing on purpose. You cannot click the PIN box. If it is not ready for typing, write that down.

## 3. Win, Alt+Tab, and Ctrl+Shift+Esc (about 4 minutes)

Cover the camera again so you get the PIN box. Do not finish the PIN yet.

1. Run the hello command.
2. While the PIN box is in front, press the Windows key. The Start menu should not open.
3. Press Alt+Tab. Windows should not switch apps.
4. Press Ctrl+Shift+Esc. Task Manager should not open.
5. Then type the right PIN, or press the escape chord if you are done looking.
6. Write down what each chord did. "Nothing" is a result. If something opened, write what it was.

## 4. Passthrough ends (about 8 minutes)

Do these as two runs. Cover the camera both times. The second run includes a one-minute wait.

**Dialog closes**

1. Run the hello command and wait for the PIN box.
2. Press Esc once. That cancels Hello. Do not use the escape chord yet.
3. The dark screen says the dialog closed. Type a letter. It should not appear in Notepad or anywhere else.
4. Wait for the tool to close itself (about 15 seconds), or press the escape chord.
5. Type in Notepad. The letter should appear now.
6. Write down both results: the key during the dark screen, and the key after the tool exited.

**60 seconds**

1. Run the hello command and wait until the PIN box is visible.
2. Look at a clock. Wait one minute. Do not type the PIN.
3. The dark screen should say that one minute is up. Type a digit. It should not appear in the PIN box.
4. Esc will not cancel Hello after that minute, because passthrough is over. Use the escape chord (Ctrl+Alt+Shift+Q).
5. Write down whether the digit was swallowed.

## 5. Password check with the network off (about 12 minutes)

A wrong password counts toward Windows' account-lockout counter. Use the correct password. If you want to see a failure, type one wrong password, then the right one, then stop. Do not try more than two wrong passwords. If the tool says the account is locked, stop and wait. Do not keep guessing.

This command does not block the keyboard. The tool prints `logon form tried:` before it asks for the password. Write that line down. It is the exact account form that was checked.

Turn the network off before either password run:

1. Turn Wi-Fi off, and unplug the ethernet cable if there is one.
2. Open a website and confirm it does not load.
3. Turn the network back on only after both password runs.

**Microsoft account** (you, about 4 minutes)

If you do not know your Microsoft account password, or you sign in without a password, skip this and write that in the finding.

Replace `<your email>` with the email you use for the Microsoft account. Keep `MicrosoftAccount\` in front of it. Quotes matter in PowerShell.

```text
cargo run -p softlock-hello-spike -- password --account "MicrosoftAccount\<your email>"
```

Type `yes`, then your Windows password. Write down `account kind`, `logon form tried`, `result`, and `win32 error` if there is one. The password is not printed.

**Local account** (about 8 minutes)

You stay signed in as yourself. Do not sign out, and do not copy the tool anywhere.

If you cannot add a user because you are not an admin, or the PC is family-managed, skip this and write that in the finding.

Make a temporary local user named `slp6test`. Do not use your real password for it. Invent a short one for today.

Windows 11: Settings, Accounts, Other users (on some PCs this is Family & other users), Add account, "I don't have this person's sign-in information", "Add a user without a Microsoft account". Name it `slp6test` and set the temporary password.

Windows 10: Settings, Accounts, Family & other users, Add someone else to this PC, then the same "I don't have this person's sign-in information" path.

If Settings will not offer a local account, open a terminal as Administrator and run:

```text
net user slp6test * /add
```

It asks for the temporary password twice.

Then, still signed in as yourself, with the network off:

```text
cargo run -p softlock-hello-spike -- password --account ".\slp6test"
```

Type `yes` and the temporary password. Write down the same lines as before. `logon form tried` should be `.\slp6test`.

Delete the test user. Deleting it removes that user's files. Do not save anything in it.

Windows 11 or 10: Settings, Accounts, Other users (or Family & other users), select `slp6test`, Remove.

Or, from an Administrator terminal:

```text
net user slp6test /delete
```

Settings sometimes leaves the folder `C:\Users\slp6test`. Delete that folder too. If it will not delete, write that in the finding and leave it for later.

Turn the network back on.

## 6. Write the finding (about 5 minutes)

Open `docs/findings/09-hello/FINDING.md`. Copy the results into the blank lines. Leave the Go box and the Fallback box unchecked until you have chosen, after every run above.
