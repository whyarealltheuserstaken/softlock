# m11: the x86_64 slice runs natively on GitHub Intel macOS runners

Measured on 16 November 2026 UTC. The probe is the workspace-excluded binary from m02, `spikes/macos-desktop-proof`, merged with `lipo` the same way m01 merges product crates.

## Runner labels

GitHub's [hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners), read 16 November 2026, still lists Intel labels `macos-15-intel` and `macos-26-intel` for public repositories (4 CPU, 14 GB RAM, 14 GB SSD). The same page lists arm64 `macos-latest`, `macos-15`, and `macos-26`. Decision D4 keeps those Intel labels off m01. This ticket uses them so the x86_64 slice is executed on Intel hardware GitHub still hosts.

This is native Intel. It is not a Rosetta run on an arm64 runner. `arch -x86_64` is not used. Under Rosetta, `uname -m` can print `x86_64` while `sysctl.proc_translated` is `1` and `hw.optional.arm64` is `1`. The job fails in that case.

This is not Henry's Intel Mac. His machine is the 2020 MacBook Pro (13-inch, Four Thunderbolt 3 ports) in [Henry's sheet](../tickets/macos/henry-sheet.md): laptop, Touch ID, T2, macOS Tahoe 26.7.3. No hardware serial belongs in this repo. A green GitHub Intel smoke does not record his lid, Touch ID, TCC prompts, or a second display. Those stay on his Mac (m07, m10, m12, m06). macOS 11 through 14 remain untested. `macos-15-intel` is a native x86_64 point on 15; `macos-26-intel` is a native x86_64 point on 26.

Apple's universal-binary page says an Intel Mac executes the `x86_64` slice and cannot execute the `arm64` slice. The arm64 slice's execution remains m01's smoke launch on `macos-latest`.

## Job (filled after the first green run)

The proof workflow is `.github/workflows/macos-x86-slice.yml`. It builds the probe for `x86_64-apple-darwin` and `aarch64-apple-darwin`, `lipo`s them, and runs `--smoke` on each Intel label. The smoke path does not load a LaunchAgent or LaunchDaemon.

Head SHA and run URL will be recorded here once the pull request's Intel jobs finish.

Expected lines from `--smoke` on a native Intel host:

```
uname -m
x86_64
sysctl.proc_translated=0
hw.optional.arm64=0
native-intel=true
rosetta=false
smoke=pass
```

`lipo -info` and `lipo -archs` on the binary that ran both contain `x86_64` and `arm64`.
