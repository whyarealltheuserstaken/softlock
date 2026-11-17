# m11: Run the x86_64 slice natively

**What to build:** Henry takes the universal binary m01's job built (the probe from m02, or the merged product binaries if the probe is not in that artifact) and runs it on his Intel Mac. `uname -m` is `x86_64`. `lipo -info` on the binary he ran contains `x86_64` and `arm64`. The process is not under Rosetta. Apple's universal-binary page says an Intel Mac executes the `x86_64` slice and cannot execute the `arm64` slice. This ticket is that execution. The arm64 slice's execution is m01's smoke launch on `macos-latest`.

**Blocked by:** m01 and m02.

**Blocks:** m14. The lock host he tries is a universal binary, and this ticket is the proof his Mac runs that shape.

**Where:** Henry's Intel Mac.

**Test machine:** The paragraph in D1 of [`decisions.md`](decisions.md). [Henry's sheet](henry-sheet.md) points at the same paragraph. `uname -m` on this machine is the native x86_64 run.

**Type:** Hardware kit. Short. Deliverable is the command output committed next to the m02 finding.

**Status:** waiting for Henry. Not ready-for-agent. Not a GitHub issue.

- [ ] `lipo -info` and `lipo -archs` on the binary both contain `x86_64` and `arm64` (output)
- [ ] `uname -m` is `x86_64` and the probe's smoke path exits 0 (output)
- [ ] The finding says this was native Intel, not a Rosetta run on an arm64 runner

Common to every ticket: red-green TDD at the named seams, CI green, no planned paid or enterprise features (allowlists, rules, fleet management, remote triggers, the CLI, MDM profiles), the product name only via the name constant.
