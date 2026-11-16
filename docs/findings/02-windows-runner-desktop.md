# 02: GitHub-hosted Windows runners can host the desktop tests

Measured on 16 November 2026 UTC (15 November 2026 in Pacific time) by the workspace-excluded probe `spikes/desktop-proof`.

The review follow-up is [Desktop proof run 43807392388](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43807392388). Both images printed `result=pass`.

- [windows-2022 job](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43807392388/job/133101188841)
- [windows-2025 job](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43807392388/job/133101188609)

The first green measurement, before the marker was moved, is [Desktop proof run 43805604323](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43805604323) ([windows-2022](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43805604323/job/133095547751), [windows-2025](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43805604323/job/133095547957)). Product CI on that commit passed: [CI run 43805604226](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43805604226).

## Decision for seam 3

Use the GitHub-hosted `windows-2022` and `windows-2025` runners for seam 3. A self-hosted Windows runner is not required for low-level hooks, `SendInput`, a topmost capture-excluded window, GDI / DXGI Desktop Duplication / Windows.Graphics.Capture, or installing a service and launching a process onto the interactive desktop.

Fail-closed cases still belong in their own job. They end the runner's interactive session, which is a property of the test, not of these images. Spike 04 still has to prove that the real lock is reachable on these runners.

## CI only

Do not run this probe on a local machine. The orchestrator installs a SYSTEM service from the binary under `target/`, and that directory is user-writable. The probe refuses to install the service unless `GITHUB_ACTIONS=true` or `--allow-local-service` is passed. The workflow's last step runs `sc.exe stop` and `sc.exe delete` for `RunnerDesktopProbe` even when the proof step is cancelled, so cleanup does not depend on the probe process staying alive.

## What works, on both images

The job process runs as `runneradmin` on the console, session 2, window station `WinSta0`, desktop `Default`. The screen is a single 1024x768 display. Session 0 (`Services`) is disconnected. The active console session id is 2, not 1.

A throwaway service (not the product service) was created, started, stopped, and deleted. The first measurement registered it as `P7DesktopProbe`. That name was the school machine's, so the probe now registers `RunnerDesktopProbe`. It ran as `SYSTEM` in session 0. With `SeTcbPrivilege` enabled it called `WTSQueryUserToken` on the console session (`token-source=console-user`) and `CreateProcessAsUser` onto `winsta0\default`. The child was session 2, `WinSta0` / `Default`, user `runneradmin`, and exited 0.

In that child:

- `WH_KEYBOARD_LL` and `WH_MOUSE_LL` installed.
- `SendInput` inserted 5 events. The hook saw F13 (`vk=124`) key-down and left-button down (`message=513`), both with the injected flag.
- A topmost marker (140x140, 19600 pixels) sits on a topmost control window (320x320) at the bottom-right of the 1024x768 screen, away from the runner console.
- `topmost=true` is the `WS_EX_TOPMOST` style. `topmost-hit=true` means `WindowFromPoint` at the marker centre returned the marker. `adjacent-hit=true` means `WindowFromPoint` on the strip to the right of the marker returned the control window.
- `GetWindow` with `GW_HWNDPREV` reported `topmost-above=IME` on both images. That IME window is above the marker in z-order and does not cover it: the point test still hit the marker, and the marker rectangle contained exactly 19600 marker pixels.
- With capture exclusion off, GDI `BitBlt`, DXGI Desktop Duplication, and Windows.Graphics.Capture each saw 19600 marker pixels in the marker rectangle and 25200 control pixels in the adjacent strip (180x140). Global control pixels were 82800, which is the 320x320 window minus the marker.
- With `WDA_EXCLUDEFROMCAPTURE` on the marker only, all three captures reported 0 marker pixels. The marker rectangle then contained 19600 control pixels, so the colour under the marker is the control window, not the runner console. Global control pixels were the full 102400.

The first measurement had placed a non-topmost control window at the top-left. Its global control counts were about 25k–29k rather than the full window, which fits the runner console covering part of that window. The marker itself was topmost and fully visible (exactly 180x180 = 32400 pixels). Those first-run lines are below. They are not the check the probe uses now.

Both images printed the same capture lines on the follow-up run. windows-2022 child pid 724. windows-2025 child pid 5052.

```
topmost=true
topmost-hit=true
adjacent-hit=true
topmost-above=IME
gdi-exclusion-off=marker-present control-present marker-pixels=19600 control-pixels=82800 marker-region=19600 adjacent-control=25200 under-marker-control=0
gdi-exclusion-on=marker-absent control-present marker-pixels=0 control-pixels=102400 marker-region=0 adjacent-control=25200 under-marker-control=19600
dxgi-exclusion-off=marker-present control-present marker-pixels=19600 control-pixels=82800 marker-region=19600 adjacent-control=25200 under-marker-control=0
dxgi-exclusion-on=marker-absent control-present marker-pixels=0 control-pixels=102400 marker-region=0 adjacent-control=25200 under-marker-control=19600
wgc-exclusion-off=marker-present control-present marker-pixels=19600 control-pixels=82800 marker-region=19600 adjacent-control=25200 under-marker-control=0
wgc-exclusion-on=marker-absent control-present marker-pixels=0 control-pixels=102400 marker-region=0 adjacent-control=25200 under-marker-control=19600
```

### windows-2022, first measurement

`ImageOS=win22`. Windows Server 2022 Datacenter, 21H2, build 20348, installation type Server. Child pid 4640.

```
gdi-exclusion-off=marker-present control-present marker-pixels=32400 control-pixels=24600
gdi-exclusion-on=marker-absent control-present marker-pixels=0 control-pixels=25320
dxgi-exclusion-off=marker-present control-present marker-pixels=32400 control-pixels=24600
dxgi-exclusion-on=marker-absent control-present marker-pixels=0 control-pixels=25320
wgc-exclusion-off=marker-present control-present marker-pixels=32400 control-pixels=24600
wgc-exclusion-on=marker-absent control-present marker-pixels=0 control-pixels=25320
```

### windows-2025, first measurement

`ImageOS=win25-vs2026`. Windows Server 2025 Datacenter, 24H2, build 26100, installation type Server. Child pid 1472.

```
gdi-exclusion-off=marker-present control-present marker-pixels=32400 control-pixels=28180
gdi-exclusion-on=marker-absent control-present marker-pixels=0 control-pixels=29428
dxgi-exclusion-off=marker-present control-present marker-pixels=32400 control-pixels=28180
dxgi-exclusion-on=marker-absent control-present marker-pixels=0 control-pixels=29428
wgc-exclusion-off=marker-present control-present marker-pixels=32400 control-pixels=28180
wgc-exclusion-on=marker-absent control-present marker-pixels=0 control-pixels=29428
```

## What does not work, or is not shown

Nothing in the measured set failed on either image.

These hosted images do not cover the client-only checks the spec already keeps on the manual list: the Ctrl+Alt+Del screen, Windows Hello, Modern Standby, and the lid. The desktop is one 1024x768 monitor, so multi-monitor layout is not shown. The probe does not send input at an elevated window, so the integrity-level half of spike 05 is still open. It does not take the fail-closed path, because that would end the session.

The first run's "Record the runner session" step printed `query user` and `query session`, then reported exit code 1. That is PowerShell keeping the native exit code from `query session`. The step is `continue-on-error` and is not a desktop failure. The workflow now exits 0 after printing that table, and the follow-up run's session step completed without that annotation.

Seam 3 tests must not assume the interactive session id is 1. On these runners it was 2. A service that needs the desktop has to launch into the console session; code running as the service itself stays on the disconnected Services desktop.

## Tickets that depend on this

Named dependents: [#4](https://github.com/whyarealltheuserstaken/softlock/issues/4) (fail-closed), [#5](https://github.com/whyarealltheuserstaken/softlock/issues/5) (hook visibility in CI), [#11](https://github.com/whyarealltheuserstaken/softlock/issues/11) (seam 3 probe harness).

Later tickets whose work includes a seam 3 test, and which can stay on these hosted images: [#12](https://github.com/whyarealltheuserstaken/softlock/issues/12), [#13](https://github.com/whyarealltheuserstaken/softlock/issues/13), [#14](https://github.com/whyarealltheuserstaken/softlock/issues/14), [#15](https://github.com/whyarealltheuserstaken/softlock/issues/15), [#16](https://github.com/whyarealltheuserstaken/softlock/issues/16), [#17](https://github.com/whyarealltheuserstaken/softlock/issues/17), [#18](https://github.com/whyarealltheuserstaken/softlock/issues/18), [#19](https://github.com/whyarealltheuserstaken/softlock/issues/19), [#20](https://github.com/whyarealltheuserstaken/softlock/issues/20), [#21](https://github.com/whyarealltheuserstaken/softlock/issues/21), [#22](https://github.com/whyarealltheuserstaken/softlock/issues/22), [#23](https://github.com/whyarealltheuserstaken/softlock/issues/23), [#25](https://github.com/whyarealltheuserstaken/softlock/issues/25), [#26](https://github.com/whyarealltheuserstaken/softlock/issues/26), [#27](https://github.com/whyarealltheuserstaken/softlock/issues/27), [#28](https://github.com/whyarealltheuserstaken/softlock/issues/28), [#29](https://github.com/whyarealltheuserstaken/softlock/issues/29), [#30](https://github.com/whyarealltheuserstaken/softlock/issues/30).

[#32](https://github.com/whyarealltheuserstaken/softlock/issues/32) mentions seam 3 and is deferred until an Enterprise or Education machine exists. [#24](https://github.com/whyarealltheuserstaken/softlock/issues/24) and [#31](https://github.com/whyarealltheuserstaken/softlock/issues/31) do not. [#33](https://github.com/whyarealltheuserstaken/softlock/issues/33) is the roadmap.
