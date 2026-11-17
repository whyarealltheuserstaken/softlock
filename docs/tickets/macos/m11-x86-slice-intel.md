# m11: Run the x86_64 slice natively

**What to build:** m01's `macos-latest` job uploads the fat product binaries and the fat m02 probe. A `needs:` job on `macos-15-intel` and `macos-26-intel` downloads those artifacts, checks `lipo -info` / `lipo -archs`, confirms native Intel (`uname -m` is `x86_64`, `hw.cputype` is `7`, `machdep.cpu.brand_string` contains Intel, not Rosetta), and smoke-launches each fat binary's x86_64 slice. The probe's `--smoke` path calls public CoreGraphics in-process and resolves `SACLockScreenImmediate` with `dlopen`/`dlsym` without calling it, locking, or prompting TCC. GitHub's hosted-runners page still lists those Intel labels (read 16 November 2026). GitHub said on 19 September 2025 ([runner-images #13045](https://github.com/actions/runner-images/issues/13045) and the [changelog](https://github.blog/changelog/2025-09-19-github-actions-macos-13-runner-image-is-closing-down/)) that `macos-15-intel` lasts until August 2027 as the last x86_64 image. No end date is published for `macos-26-intel`. When those labels go, the m11 CI evidence disappears.

CI is the primary evidence. Henry's Intel Mac stays on this ticket as an optional kit: run the same `--smoke` there and commit the output. A Rosetta `arch -x86_64` on `macos-latest` is not this test. D4 still keeps the Intel labels off the m01 *build* job. The arm64 slice's execution is m01's smoke launch on `macos-latest`.

**Blocked by:** m01 and m02.

**Blocks:** m14. The lock host that ticket tries is a universal binary, and this ticket is the proof an Intel Mac runs that shape.

**Where:** CI on GitHub Intel (`macos-15-intel`, `macos-26-intel`), and Henry's Intel Mac (optional kit).

**Test machine:** GitHub-hosted Intel images `macos-15-intel` and `macos-26-intel`, and the paragraph in D1 of [`decisions.md`](decisions.md). [Henry's sheet](henry-sheet.md) points at the same paragraph. No hardware serial belongs in this repo. macOS 11 through 14 are still untested.

**Type:** Hardware kit plus CI. Deliverable is the command output in [`docs/findings/m11-x86-slice.md`](../../findings/m11-x86-slice.md).

**Status:** CI is the primary evidence. Optional kit on his Intel Mac remains open.

- [x] `lipo -info` and `lipo -archs` on the binary both contain `x86_64` and `arm64` (job log)
- [x] `uname -m` is `x86_64` and the probe's smoke path exits 0 (job log)
- [x] The finding says this was native Intel, not a Rosetta run on an arm64 runner
- [ ] Optional: the same `--smoke` on Henry's Intel MacBook Pro (output committed next to the finding). No `--allow-local-service`. No hardware serial. Confirm `uname -m` is `x86_64`, `hw.cputype` is `7`, and `machdep.cpu.brand_string` contains Intel, then run the fat `macos-desktop-proof` from the m01 artifact with `--smoke`.

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
