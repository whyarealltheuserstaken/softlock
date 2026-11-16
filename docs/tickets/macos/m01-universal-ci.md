# m01: Universal 2 CI job: both slices, lipo, release scan on each

**What to build:** A GitHub Actions job on `macos-latest` that builds the product crates for `x86_64-apple-darwin` and `aarch64-apple-darwin`, merges them with `lipo`, and runs the existing release-binary scan on each slice. The same job builds the settings app with Tauri's `universal-apple-darwin` target and checks that binary too. `MACOSX_DEPLOYMENT_TARGET` is `11.0` (Henry, D1). The job prints `sw_vers` and does not assume the image is macOS 15 or macOS 26. It does not add `macos-15-intel` or `macos-26-intel` (D4, accepted by the lead).

**Blocked by:** none. Can start with m02.

**Blocks:** m11, m14.

**Where:** CI only.

**Type:** Build job, not a spike. It is the first macOS ticket because later probes run from a universal binary. No lock behavior.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

Pinned to `crates/softlock-lockhost/tests/release_binary.rs` at `23b5a39bc92c2ee32f470256d78be27d030dc59f`. The test reads `SOFTLOCK_RELEASE_SCAN_DIR` and prints `scanned release binaries`. The marker bytes on that commit are `softlock-simulated-physical-marker` and `softlock_simulated_physical_marker`. Once PR #48 merges, the same scan also names the dev-launch markers `softlock-dev-launch-marker` and `softlock_dev_launch_marker`, and this job fails a slice that contains either pair.

- [ ] The job installs both Rust targets and builds the product crates `--release --locked` for `x86_64-apple-darwin` and for `aarch64-apple-darwin` (job log)
- [ ] `lipo -info` and `lipo -archs` on each merged executable both contain `x86_64` and `arm64` (job log)
- [ ] `lipo -thin x86_64` and `lipo -thin arm64` each get their own directory, and `product_release_binaries` prints `scanned release binaries` for each directory and for the fat file (job log)
- [ ] `npm run tauri build -- --target universal-apple-darwin` produces an app executable whose `lipo -info` also contains both architectures, and that executable is included in the scan (job log)
- [ ] The arm64 slice runs a smoke launch on this runner (`uname -m` is `arm64`). The x86_64 slice is not treated as tested by this job
- [ ] A build that contains a simulated-physical marker fails the scan. Once PR #48 is on main, a build that contains `softlock-dev-launch-marker` or `softlock_dev_launch_marker` fails the same scan. The existing contaminated-build test covers the scanner; this job uses the clean release build

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
