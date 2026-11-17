# m11: the x86_64 slice runs natively on GitHub Intel macOS runners

Measured on 16 November 2026 UTC by the workspace-excluded probe `spikes/macos-desktop-proof`, merged with `lipo` the same way m01 merges product crates.

The proof run is [macOS x86_64 slice run 43977157764](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43977157764) on head `872cd14aea22482c3b8297052ad1d84ff3001e08`:

- [native x86_64 (macos-15-intel)](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43977157764/job/133683967883)
- [native x86_64 (macos-26-intel)](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43977157764/job/133683968374)

Both jobs printed `smoke=pass`. The smoke path does not load a LaunchAgent or LaunchDaemon.

## Runner labels

GitHub's [hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners), read 16 November 2026, still lists Intel labels `macos-15-intel` and `macos-26-intel` for public repositories (4 CPU, 14 GB RAM, 14 GB SSD). The same page lists arm64 `macos-latest`, `macos-15`, and `macos-26`. Decision D4 keeps those Intel labels off m01. This ticket uses them so the x86_64 slice is executed on Intel hardware GitHub still hosts.

This is native Intel. It is not a Rosetta run on an arm64 runner. `arch -x86_64` is not used. Under Rosetta, `uname -m` can print `x86_64` while `sysctl.proc_translated` is `1` and `hw.optional.arm64` is `1`. The job fails in that case. On both Intel images those sysctl keys were absent, which is not a translated process.

This is not Henry's Intel Mac. His machine is the 2020 MacBook Pro (13-inch, Four Thunderbolt 3 ports) in [Henry's sheet](../tickets/macos/henry-sheet.md): laptop, Touch ID, T2, macOS Tahoe 26.7.3. No hardware serial belongs in this repo. A green GitHub Intel smoke does not record his lid, Touch ID, TCC prompts, or a second display. Those stay on his Mac (m07, m10, m12, m06). macOS 11 through 14 remain untested. `macos-15-intel` is a native x86_64 point on 15; `macos-26-intel` is a native x86_64 point on 26.

Apple's universal-binary page says an Intel Mac executes the `x86_64` slice and cannot execute the `arm64` slice. The arm64 slice's execution remains m01's smoke launch on `macos-latest`.

## macos-15-intel

`sw_vers` and the native checks:

```
ProductName:		macOS
ProductVersion:		15.7.9
BuildVersion:		24G830
uname -m is x86_64
sysctl.proc_translated absent
hw.optional.arm64 absent
```

The fat probe:

```
=== lipo -info spikes/macos-desktop-proof/target/universal/macos-desktop-proof ===
Architectures in the fat file: spikes/macos-desktop-proof/target/universal/macos-desktop-proof are: x86_64 arm64
=== lipo -archs spikes/macos-desktop-proof/target/universal/macos-desktop-proof ===
x86_64 arm64
```

`--smoke` on that binary:

```
uname -m
x86_64
sysctl.proc_translated=absent
hw.optional.arm64=absent
lipo -info Architectures in the fat file: /Users/runner/work/softlock/softlock/spikes/macos-desktop-proof/target/universal/macos-desktop-proof are: x86_64 arm64
lipo -archs x86_64 arm64
native-intel=true
rosetta=false
fat=true
smoke=pass
runner-image=macos-15-intel
```

## macos-26-intel

`sw_vers` and the native checks:

```
ProductName:		macOS
ProductVersion:		26.6.1
BuildVersion:		25G76
uname -m is x86_64
sysctl.proc_translated absent
hw.optional.arm64 absent
```

The fat probe:

```
=== lipo -info spikes/macos-desktop-proof/target/universal/macos-desktop-proof ===
Architectures in the fat file: spikes/macos-desktop-proof/target/universal/macos-desktop-proof are: x86_64 arm64
=== lipo -archs spikes/macos-desktop-proof/target/universal/macos-desktop-proof ===
x86_64 arm64
```

`--smoke` on that binary:

```
uname -m
x86_64
sysctl.proc_translated=absent
hw.optional.arm64=absent
lipo -info Architectures in the fat file: /Users/runner/work/softlock/softlock/spikes/macos-desktop-proof/target/universal/macos-desktop-proof are: x86_64 arm64
lipo -archs x86_64 arm64
native-intel=true
rosetta=false
fat=true
smoke=pass
runner-image=macos-26-intel
```
