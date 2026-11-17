# macOS lock kit

Henry runs this on his Intel Mac, from Terminal.app. Close or save anything important first. The kit has no unlock command. His normal login password is what unlocks the Mac.

The default command is read-only. It does not lock the Mac, and it does not change FileVault or the screen lock setting. An exercise command does lock the Mac. It refuses to run on a GitHub runner and refuses to run as root. It does not use sudo.

Build:

```sh
cargo build --release --manifest-path spikes/macos-lock-kit/Cargo.toml
```

Run every command below from that build, as yourself, not as root. Each step has its own `--output-dir`, so a later step does not overwrite an earlier result. Commit each `result.json` and `result.txt`. Those files record the OS version, the model identifier, the numeric user id, FileVault on/off, the screen-lock reading, and whether a lock path ran. They do not record an account name, a home directory, an Apple ID, a hardware serial, a list of other apps, or the name of another device. If a command prints one of those, the kit replaces it before it writes the file. Do not add output from About This Mac, `system_profiler`, or `fdesetup` with any verb other than `status` and `isactive`.

## Steps

1. Read-only. This does not lock the Mac.

   ```sh
   cargo run --release --manifest-path spikes/macos-lock-kit/Cargo.toml -- --read-only --output-dir lock-kit-output/read-only
   ```

   The result says whether `dlsym` resolved `SACLockScreenImmediate` (the symbol is not called), the `fdesetup status` and `fdesetup isactive` readings, the screen-lock defaults, and `sysadminctl -screenLock status`. `ask-for-lock` is macOS's `askForPassword`. `ask-for-lock-delay` is `askForPasswordDelay`. If `sysadminctl` asks for an admin password, the kit records `screen-lock-status=unavailable` and does not wait for a password. If the defaults and `sysadminctl` disagree, or both are missing, `immediate-lock-required` is false and an exercise of the documented path refuses to arm. `session-lock-check=available` means the current session dictionary could be read. `session-lock-key-present` and `session-lock-key-false` say whether `CGSSessionScreenIsLocked` is in that dictionary and whether its value is false.

2. If you have an external display, attach it now, before either exercise. Leave it attached until both exercises are done. If you do not have one, leave this step out and answer `none` later.

3. Private exercise. This locks the Mac.

   ```sh
   cargo run --release --manifest-path spikes/macos-lock-kit/Cargo.toml -- --exercise private --i-will-unlock --output-dir lock-kit-output/private
   ```

   If the symbol is missing, the kit records the miss and does not lock. That is not a crash. The chooser row for a missing symbol is the CI test `symbol_missing_requests_the_documented_path`.

   The kit calls the symbol on a worker thread. If the call is still running after the wait, the result says `private-call=still-running` and the kit tries the next path. After the call returns, it polls the session for about two seconds. `ms-until-locked` is how long until the session reported locked. `private-call-returned-ms` is only how long the function took to return. If the session dictionary says it is not locked, the call counts as failed and the kit falls back when that is still safe. If the dictionary cannot be read, the result says `lock-check=unavailable`. That is not a failed call, and the kit still tries the next path.

   After the lock, unlock with your normal login password first. Then answer the three questions in Terminal. The kit stays armed while it waits: the helper is still running, and `complete` is not written until the answers are in.

   - Did the real lock screen appear? `yes` or `no`.
   - Did a click or a key reach the desktop without the account password? `yes` or `no`.
   - With the external display attached, did the lock cover both screens? `yes`, `no`, or `none` when no external display is attached. That answer is `external-display-covered`.

   Ctrl-C at one of these questions locks the Mac again. The kit does not unlock it. The helper is already in its own session, so the question is abandoned and the helper locks.

4. The kit does not unlock anything. If you press Ctrl-C, close Terminal, or the parent is killed before a question, the helper still tries to leave the Mac locked. Unlock that lock with your normal login password.

5. Documented exercise. This locks the Mac by starting the screen saver, and only when the screen lock setting can require a password immediately.

   ```sh
   cargo run --release --manifest-path spikes/macos-lock-kit/Cargo.toml -- --exercise documented --i-will-unlock --output-dir lock-kit-output/documented
   ```

   If the setting is unknown, off, or the delay is not zero, the kit refuses to arm, leaves the session as it was, and records `state=never-unlocked`. The message in the result says how to turn the setting on. The kit does not change the setting for you.

   While the screen saver is up, try one click and one key. `documented-started-ms` is how long the kit waited until the screen-saver process was running. Unlock with your normal login password first, then answer the same three questions as step 3 in Terminal, including whether the lock covered both screens. The kit stays armed while it waits. Ctrl-C at a question locks the Mac again.

6. If Ctrl-C locked the Mac again, unlock with your normal login password. The kit does not unlock it.

7. Optional. If `filevault=on`, reboot once yourself and record whether the pre-boot sign-in appeared. This Mac has the T2 chip, so that screen is the Secure Enclave path. The kit does not reboot, and it does not turn FileVault on or off. If FileVault is off, the pre-boot screen stays unproven. The kit always writes `filevault-pre-boot-observed=false`.

## What the kit will not do

- It does not link `login.framework`. It loads that image with `dlopen` only.
- It does not write preferences, run `fdesetup` with any verb other than `status` and `isactive`, run `sysadminctl` with any arguments other than `-screenLock status`, install a launchd job, or unlock the session.
- It does not choose a third lock primitive. The login window is not a fallback.
- If the parent process dies after an exercise has started, the helper tries the same fail-closed path and does not unlock. A read-only run does not arm that helper. If the helper cannot be started, the exercise is refused and the lock path is not called.
