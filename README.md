# SoftLock

SoftLock is a pre-release Windows lock screen for computers that run AI agents and other software. It blocks physical keyboard and mouse input while software on the computer keeps running. A person at the computer sees the lock screen. Agents keep seeing and driving the real desktop. The project is free and open source under the Apache License 2.0.

Builds are unsigned. Windows SmartScreen will warn you; choose More info → Run anyway.

If SoftLock fails while locked, it switches to the real Windows lock, and your agents will pause until you sign in.

## Layout

- `crates/softlock-core` — shared library with no operating-system calls
- `crates/softlock-windows` — Windows backend
- `crates/softlock-macos` — macOS backend stub (not implemented)
- `crates/softlock-linux` — Linux backend stub (not implemented)
- `crates/softlock-service` — background service
- `crates/softlock-lockhost` — lock screen and input guard
- `crates/softlock-broker` — Windows Hello broker
- `crates/softlock-app` — Tauri 2 and Svelte 5 shell
- `docs/SPEC.md` — the approved product spec

Crates and the npm package are not published. The name on screen lives in one Rust constant, `PRODUCT_DISPLAY_NAME`, and in the app string resources. The settings app package is named `softlock` and its bundle id is `com.softlock.app`. The background service is shown as "SoftLock Agent". Its service name and registry subkey (`SoftLockAgent`) and its named pipe (`softlock-agent`) are computed from that display name.

## Develop

Install the Rust toolchain in `rust-toolchain.toml` and Node.js 22.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo deny check licenses advisories sources
```

Seam 4 runs the Tauri front end against an empty fake IPC bridge:

```bash
cd crates/softlock-app
npm ci
npm run test:e2e
```

The desktop shell opens an empty window:

```bash
cd crates/softlock-app
npm ci
npm run tauri dev
```
