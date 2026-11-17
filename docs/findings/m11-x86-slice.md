# m11: the x86_64 slice runs natively on GitHub Intel macOS runners

CI is the primary evidence. Henry's Intel Mac stays on the ticket as an optional kit (lead, 16 November 2026).

The m01 `universal 2` job on `macos-latest` uploads the fat product binaries and the fat m02 probe. The `native x86_64` job downloads that artifact onto GitHub Intel. It does not rebuild. The native Intel job does not run on push to main; only the arm64 universal build does.

Measured on 17 November 2026 UTC from [run 44009010873](https://github.com/whyarealltheuserstaken/softlock/actions/runs/44009010873) on `264654cc1f532f48bebad105c24804bb74615b4f`.

## Runner labels

GitHub's [hosted runners reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners), read 16 November 2026, still lists Intel labels `macos-15-intel` and `macos-26-intel` for public repositories (4 CPU, 14 GB RAM, 14 GB SSD). The same page lists arm64 `macos-latest`, `macos-15`, and `macos-26`. Decision D4 keeps those Intel labels off the m01 build job. m11's native job uses them so the x86_64 slice is executed on Intel hardware GitHub still hosts.

GitHub said on 19 September 2025, in [runner-images #13045](https://github.com/actions/runner-images/issues/13045) and the [changelog](https://github.blog/changelog/2025-09-19-github-actions-macos-13-runner-image-is-closing-down/), that `macos-15-intel` lasts until August 2027 as the last x86_64 image, after which GitHub Actions would not support that architecture. No end date is published for `macos-26-intel`. When those labels go, the m11 CI evidence disappears.

This is native Intel. It is not a Rosetta run on an arm64 runner. `arch -x86_64` is not used. Under Rosetta, `uname -m` can print `x86_64` while `sysctl.proc_translated` is `1` and `hw.optional.arm64` is `1`. The job also requires `hw.cputype` is `7` and `machdep.cpu.brand_string` contains Intel.

Henry's Intel Mac remains the machine in the D1 paragraph and [Henry's sheet](../tickets/macos/henry-sheet.md). No hardware serial belongs in this repo. A green GitHub Intel smoke does not record his lid, Touch ID, TCC prompts, or a second display. Those stay on his Mac (m07, m10, m12, m06). The optional m11 kit is the same `--smoke` on that machine. macOS 11 through 14 remain untested. `macos-15-intel` is a native x86_64 point on 15; `macos-26-intel` is a native x86_64 point on 26.

Apple's universal-binary page says an Intel Mac executes the `x86_64` slice and cannot execute the `arm64` slice. The arm64 slice's execution remains m01's smoke launch on `macos-latest`.

The probe `--smoke` path calls `CGMainDisplayID` / `CGDisplayBounds` and `CGSessionCopyCurrentDictionary` in-process. It `dlopen`s the private login image and `dlsym`s the D5 lock symbol. It does not call that symbol, lock the session, or load a LaunchAgent.

## Job

Both Intel jobs downloaded artifact `universal-fat` from the `universal 2` job. SHA-256 of `fat/softlock-app`, `tauri/softlock-app`, and `probe/macos-desktop-proof` matched before any codesign. `lipo -info` and `lipo -archs` on each named `x86_64` and `arm64`.

### macos-15-intel

[Job 133794683601](https://github.com/whyarealltheuserstaken/softlock/actions/runs/44009010873/job/133794683601)

```
ProductVersion:		15.7.9
uname -m is x86_64
hw.cputype=7
machdep.cpu.brand_string=Intel(R) Core(TM) i7-8700B CPU @ 3.20GHz
=== sha256 before codesign ===
fat/softlock-app: OK
tauri/softlock-app: OK
probe/macos-desktop-proof: OK
Architectures in the fat file: m01-fat/fat/softlock-app are: x86_64 arm64
codesign already valid: m01-fat/fat/softlock-app
x86_64 slice smoke launch is running: m01-fat/fat/softlock-app
Architectures in the fat file: m01-fat/tauri/softlock-app are: x86_64 arm64
x86_64 slice smoke launch is running: m01-fat/tauri/softlock-app
Architectures in the fat file: m01-fat/probe/macos-desktop-proof are: x86_64 arm64
codesign already valid: m01-fat/probe/macos-desktop-proof
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

[Job 133794683630](https://github.com/whyarealltheuserstaken/softlock/actions/runs/44009010873/job/133794683630)

```
ProductVersion:		26.6.1
uname -m is x86_64
hw.cputype=7
machdep.cpu.brand_string=Intel(R) Core(TM) i7-8700B CPU @ 3.20GHz
=== sha256 before codesign ===
fat/softlock-app: OK
tauri/softlock-app: OK
probe/macos-desktop-proof: OK
Architectures in the fat file: m01-fat/fat/softlock-app are: x86_64 arm64
codesign already valid: m01-fat/fat/softlock-app
x86_64 slice smoke launch is running: m01-fat/fat/softlock-app
Architectures in the fat file: m01-fat/tauri/softlock-app are: x86_64 arm64
x86_64 slice smoke launch is running: m01-fat/tauri/softlock-app
Architectures in the fat file: m01-fat/probe/macos-desktop-proof are: x86_64 arm64
codesign already valid: m01-fat/probe/macos-desktop-proof
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
