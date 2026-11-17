# m11: the x86_64 slice runs natively on GitHub Intel macOS runners

CI is the primary evidence. Henry's Intel Mac stays on the ticket as an optional kit (lead, 16 November 2026).

The m01 `universal 2` job on `macos-latest` uploads the fat product binaries and the fat m02 probe. The `native x86_64` job downloads that artifact onto GitHub Intel. It does not rebuild. The native Intel job does not run on push to main; only the arm64 universal build does.

Measured on 16 November 2026 UTC. Head SHA and the Intel job log lines land here after the first green run of that `needs:` job.

## Runner labels

GitHub's [hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners), read 16 November 2026, still lists Intel labels `macos-15-intel` and `macos-26-intel` for public repositories (4 CPU, 14 GB RAM, 14 GB SSD). The same page lists arm64 `macos-latest`, `macos-15`, and `macos-26`. Decision D4 keeps those Intel labels off the m01 build job. m11's native job uses them so the x86_64 slice is executed on Intel hardware GitHub still hosts.

GitHub said on 19 September 2025, in [runner-images #13045](https://github.com/actions/runner-images/issues/13045) and the [changelog](https://github.blog/changelog/2025-09-19-github-actions-macos-13-runner-image-is-closing-down/), that `macos-15-intel` lasts until August 2027 as the last x86_64 image, after which GitHub Actions would not support that architecture. No end date is published for `macos-26-intel`. When those labels go, the m11 CI evidence disappears.

This is native Intel. It is not a Rosetta run on an arm64 runner. `arch -x86_64` is not used. Under Rosetta, `uname -m` can print `x86_64` while `sysctl.proc_translated` is `1` and `hw.optional.arm64` is `1`. The job also requires `hw.cputype` is `7` and `machdep.cpu.brand_string` contains Intel.

Henry's Intel Mac remains the 2020 MacBook Pro (13-inch, Four Thunderbolt 3 ports) in [Henry's sheet](../tickets/macos/henry-sheet.md): laptop, Touch ID, T2, macOS Tahoe 26.7.3. No hardware serial belongs in this repo. A green GitHub Intel smoke does not record his lid, Touch ID, TCC prompts, or a second display. Those stay on his Mac (m07, m10, m12, m06). The optional m11 kit is the same `--smoke` on that machine. macOS 11 through 14 remain untested. `macos-15-intel` is a native x86_64 point on 15; `macos-26-intel` is a native x86_64 point on 26.

Apple's universal-binary page says an Intel Mac executes the `x86_64` slice and cannot execute the `arm64` slice. The arm64 slice's execution remains m01's smoke launch on `macos-latest`.

The probe `--smoke` path calls `CGMainDisplayID` / `CGDisplayBounds` and `CGSessionCopyCurrentDictionary` in-process. It `dlopen`s the private login image and `dlsym`s the D5 lock symbol. It does not call that symbol, lock the session, or load a LaunchAgent.

## Job (filled after the first green `needs:` run)

Expected host lines:

```
uname -m is x86_64
hw.cputype=7
machdep.cpu.brand_string=...Intel...
native-intel=true
rosetta=false
smoke=pass
sac-lock-called=false
```

`lipo -info` and `lipo -archs` on each fat binary that ran both contain `x86_64` and `arm64`.
