# m11: the x86_64 slice runs natively on GitHub Intel macOS runners

CI is the primary evidence. Henry's Intel Mac stays on the ticket as an optional kit (lead, 16 November 2026).

The m01 `universal 2` job on `macos-latest` uploads the fat product binaries and the fat m02 probe. The `native x86_64` job downloads that artifact onto GitHub Intel. It does not rebuild. The native Intel job does not run on push to main; only the arm64 universal build does.

Measured on 16 November 2026 UTC from [run 43980483196](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43980483196) on `d9983cda1b05dbe49555ab0ef8ee20eec0391954`.

## Runner labels

GitHub's [hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners), read 16 November 2026, still lists Intel labels `macos-15-intel` and `macos-26-intel` for public repositories (4 CPU, 14 GB RAM, 14 GB SSD). The same page lists arm64 `macos-latest`, `macos-15`, and `macos-26`. Decision D4 keeps those Intel labels off the m01 build job. m11's native job uses them so the x86_64 slice is executed on Intel hardware GitHub still hosts.

GitHub said on 19 September 2025, in [runner-images #13045](https://github.com/actions/runner-images/issues/13045) and the [changelog](https://github.blog/changelog/2025-09-19-github-actions-macos-13-runner-image-is-closing-down/), that `macos-15-intel` lasts until August 2027 as the last x86_64 image, after which GitHub Actions would not support that architecture. No end date is published for `macos-26-intel`. When those labels go, the m11 CI evidence disappears.

This is native Intel. It is not a Rosetta run on an arm64 runner. `arch -x86_64` is not used. Under Rosetta, `uname -m` can print `x86_64` while `sysctl.proc_translated` is `1` and `hw.optional.arm64` is `1`. The job also requires `hw.cputype` is `7` and `machdep.cpu.brand_string` contains Intel.

Henry's Intel Mac remains the 2020 MacBook Pro (13-inch, Four Thunderbolt 3 ports) in [Henry's sheet](../tickets/macos/henry-sheet.md): laptop, Touch ID, T2, macOS Tahoe 26.7.3. No hardware serial belongs in this repo. A green GitHub Intel smoke does not record his lid, Touch ID, TCC prompts, or a second display. Those stay on his Mac (m07, m10, m12, m06). The optional m11 kit is the same `--smoke` on that machine. macOS 11 through 14 remain untested. `macos-15-intel` is a native x86_64 point on 15; `macos-26-intel` is a native x86_64 point on 26.

Apple's universal-binary page says an Intel Mac executes the `x86_64` slice and cannot execute the `arm64` slice. The arm64 slice's execution remains m01's smoke launch on `macos-latest`.

The probe `--smoke` path calls `CGMainDisplayID` / `CGDisplayBounds` and `CGSessionCopyCurrentDictionary` in-process. It `dlopen`s the private login image and `dlsym`s the D5 lock symbol. It does not call that symbol, lock the session, or load a LaunchAgent.

## Job

Both Intel jobs downloaded artifact `universal-fat` from the `universal 2` job. Each smoked three fat Mach-O files: `m01-fat/fat/softlock-app`, `m01-fat/tauri/softlock-app`, and `m01-fat/probe/macos-desktop-proof`. `lipo -info` and `lipo -archs` on each named `x86_64` and `arm64`.

### macos-15-intel

[Job 133701793116](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43980483196/job/133701793116)

```
ProductName:		macOS
ProductVersion:		15.7.9
uname -m is x86_64
hw.cputype=7
machdep.cpu.brand_string=Intel(R) Core(TM) i7-8700B CPU @ 3.20GHz
Architectures in the fat file: m01-fat/fat/softlock-app are: x86_64 arm64
x86_64 arm64
x86_64 slice smoke launch is running: m01-fat/fat/softlock-app
Architectures in the fat file: m01-fat/tauri/softlock-app are: x86_64 arm64
x86_64 arm64
x86_64 slice smoke launch is running: m01-fat/tauri/softlock-app
Architectures in the fat file: m01-fat/probe/macos-desktop-proof are: x86_64 arm64
x86_64 arm64
sysctl.proc_translated=absent
hw.optional.arm64=absent
hw.cputype=7
machdep.cpu.brand_string=Intel(R) Core(TM) i7-8700B CPU @ 3.20GHz
lipo -info Architectures in the fat file: /Users/runner/work/softlock/softlock/m01-fat/probe/macos-desktop-proof are: x86_64 arm64
lipo -archs x86_64 arm64
cg-main-display-id=25165824
cg-main-display-bounds=0,0,1920,1080
cg-session-dictionary=non-null
sac-lock-symbol=resolved
sac-lock-called=false
native-intel=true
rosetta=false
smoke=pass
smoked 3 fat binaries on native x86_64
```

### macos-26-intel

[Job 133701793073](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43980483196/job/133701793073)

```
ProductName:		macOS
ProductVersion:		26.6.1
uname -m is x86_64
hw.cputype=7
machdep.cpu.brand_string=Intel(R) Core(TM) i7-8700B CPU @ 3.20GHz
Architectures in the fat file: m01-fat/fat/softlock-app are: x86_64 arm64
x86_64 arm64
x86_64 slice smoke launch is running: m01-fat/fat/softlock-app
Architectures in the fat file: m01-fat/tauri/softlock-app are: x86_64 arm64
x86_64 arm64
x86_64 slice smoke launch is running: m01-fat/tauri/softlock-app
Architectures in the fat file: m01-fat/probe/macos-desktop-proof are: x86_64 arm64
x86_64 arm64
sysctl.proc_translated=absent
hw.optional.arm64=absent
hw.cputype=7
machdep.cpu.brand_string=Intel(R) Core(TM) i7-8700B CPU @ 3.20GHz
lipo -info Architectures in the fat file: /Users/runner/work/softlock/softlock/m01-fat/probe/macos-desktop-proof are: x86_64 arm64
lipo -archs x86_64 arm64
cg-main-display-id=25165824
cg-main-display-bounds=0,0,1920,1080
cg-session-dictionary=non-null
sac-lock-symbol=resolved
sac-lock-called=false
native-intel=true
rosetta=false
smoke=pass
smoked 3 fat binaries on native x86_64
```
