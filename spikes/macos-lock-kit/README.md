# macOS lock kit

Henry runs this on his Intel Mac. The default command is read-only. It does not lock the Mac, and it does not change FileVault or the screen-saver password setting.

An exercise command does lock the Mac. The kit has no unlock command. The way back is the account password. m02 did not show that a GitHub runner comes back after a lock, so an exercise command refuses to run when `GITHUB_ACTIONS=true`.

Build:

```sh
cargo build --release --manifest-path spikes/macos-lock-kit/Cargo.toml
```

## Read-only

```sh
cargo run --release --manifest-path spikes/macos-lock-kit/Cargo.toml -- --read-only --output-dir lock-kit-output
```

Run it as yourself, not as root. Paste `lock-kit-output/result.json` into the finding. The file records:

- whether `dlsym` resolved `SACLockScreenImmediate` (the symbol is not called)
- `fdesetup status` and `fdesetup isactive`
- `askForPassword` and `askForPasswordDelay`
- `sw_vers`, `uname -m`, and `hw.model`

`hw.model` is a model identifier. Do not add a hardware serial, and do not paste one from About This Mac.

## Private lock path

Run this from Terminal.app, then watch the screen.

```sh
cargo run --release --manifest-path spikes/macos-lock-kit/Cargo.toml -- --exercise private --i-will-unlock --output-dir lock-kit-output
```

If the symbol is missing, the kit records the miss and does not lock. That is not a crash. The chooser row for a missing symbol is the CI test `symbol_missing_requests_the_documented_path`.

If the call returns, the kit asks whether the real lock screen appeared. The `private-call-returned-ms` field is how long the call took to return. It is not, by itself, proof that the lock screen was visible within a second. That observation is yours.

If the call fails, or you answer that the lock screen did not appear, and the screen-saver password can be required immediately, the kit starts the screen saver. If that also fails, the result state is `never-unlocked`. The kit still does not unlock anything.

## Documented lock path

```sh
cargo run --release --manifest-path spikes/macos-lock-kit/Cargo.toml -- --exercise documented --i-will-unlock --output-dir lock-kit-output
```

The kit starts the screen saver only when "Require password after screen saver begins or display is turned off" is on and the delay is zero. Otherwise it refuses to arm, leaves the session as it was, and records `state=never-unlocked`. The message in the result says how to turn that setting on and set the delay to zero. The kit does not change the setting for you.

While the screen saver is up, try one click and one key. Then enter the account password. Answer the two questions. `documented-started-ms` is how long the kit waited until the screen-saver process was running. Whether a click or a key reached the desktop is the answer you type.

## FileVault pre-boot

The kit does not reboot, and it does not turn FileVault on or off. If `filevault=on`, reboot once yourself and record whether the pre-boot sign-in appeared. This Mac has the T2 chip, so that screen is the Secure Enclave path. If FileVault is off, the pre-boot screen stays unproven. Leave `filevault-pre-boot-observed` as you saw it; the kit always writes `false`.

## What the kit will not do

- It does not link `login.framework`. It loads that image with `dlopen` only.
- It does not write preferences, run `fdesetup` with any verb other than `status` and `isactive`, install a launchd job, or unlock the session.
- It does not choose a third lock primitive. The login window is not a fallback.
- If the parent process dies after an exercise has started, a helper tries the same fail-closed path and does not unlock. A read-only run does not arm that helper.
