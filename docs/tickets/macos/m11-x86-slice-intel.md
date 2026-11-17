# m11: Run the x86_64 slice natively

**What to build:** Take the m02 probe, build it as a Universal 2 binary the same way m01 merges product crates, and run the fat file on GitHub-hosted Intel macOS. GitHub's hosted-runners page still lists `macos-15-intel` and `macos-26-intel` (read 16 November 2026). Those labels are native x86_64. A Rosetta `arch -x86_64` on `macos-latest` is not this test: under Rosetta `uname -m` can still print `x86_64`. The job requires `sysctl.proc_translated` is not `1` and `hw.optional.arm64` is not `1`. `lipo -info` and `lipo -archs` on the binary that ran both contain `x86_64` and `arm64`. The probe's `--smoke` path exits 0. Apple's universal-binary page says an Intel Mac executes the `x86_64` slice and cannot execute the `arm64` slice. The arm64 slice's execution is m01's smoke launch on `macos-latest`.

This is not Henry's Intel Mac. His lid, Touch ID, TCC prompts, and extra displays stay on his machine. D4 still keeps the Intel labels off m01.

**Blocked by:** m01 and m02.

**Blocks:** m14. The lock host that ticket tries is a universal binary, and this ticket is the proof an Intel Mac runs that shape.

**Where:** CI only, on `macos-15-intel` and `macos-26-intel`.

**Test machine:** GitHub-hosted Intel images `macos-15-intel` and `macos-26-intel`, and the paragraph in D1 of [`decisions.md`](decisions.md). [Henry's sheet](henry-sheet.md) points at the same paragraph. No hardware serial belongs in this repo. macOS 11 through 14 are still untested.

**Type:** Hardware kit, run on GitHub Intel because those labels are still listed. Deliverable is the command output committed next to the m02 finding, in [`docs/findings/m11-x86-slice.md`](../../findings/m11-x86-slice.md).

**Status:** implemented on GitHub Intel. His Mac is not required for this slice execution.

- [x] `lipo -info` and `lipo -archs` on the binary both contain `x86_64` and `arm64` (job log)
- [x] `uname -m` is `x86_64` and the probe's smoke path exits 0 (job log)
- [x] The finding says this was native Intel, not a Rosetta run on an arm64 runner

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
