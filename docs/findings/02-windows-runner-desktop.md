# 02: GitHub-hosted Windows runners can host the desktop tests

Measured on 16 November 2026 UTC (15 November 2026 in Pacific time) by the workspace-excluded probe `spikes/desktop-proof`, workflow [Desktop proof run 43805604323](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43805604323).

- [windows-2022 job](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43805604323/job/133095547751)
- [windows-2025 job](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43805604323/job/133095547957)

Both jobs printed `result=pass`. The product CI run on the same commit also passed: [CI run 43805604226](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43805604226).

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
- A topmost tool window painted the marker colour (180x180, 32400 pixels) over a control-coloured window.
- With capture exclusion off, GDI `BitBlt`, DXGI Desktop Duplication, and Windows.Graphics.Capture all reported the marker present and the control colour present.
- With `WDA_EXCLUDEFROMCAPTURE` on the marker only, all three captures reported the marker absent (0 pixels) and the control colour still present.

The first measurement placed a non-topmost control window at the top-left. Its global control-pixel counts were about 25k–29k, not the full window, which fits the runner console covering part of that window. The marker was topmost and fully visible (exactly 32400 pixels), so the marker count was not the console. The follow-up probe moves both windows to the bottom-right, makes the control window topmost as well, counts marker pixels inside the marker rectangle, and requires the control colour in the strip immediately to the right of the marker. `topmost=` is still the style flag. `topmost-hit=` is `WindowFromPoint` at the marker centre, and `adjacent-hit=` is `WindowFromPoint` on that strip. The window above the marker in z-order is logged as `topmost-above=`.

### windows-2022

`ImageOS=win22`. Windows Server 2022 Datacenter, 21H2, build 20348, installation type Server. Child pid 4640.

```
gdi-exclusion-off=marker-present control-present marker-pixels=32400 control-pixels=24600
gdi-exclusion-on=marker-absent control-present marker-pixels=0 control-pixels=25320
dxgi-exclusion-off=marker-present control-present marker-pixels=32400 control-pixels=24600
dxgi-exclusion-on=marker-absent control-present marker-pixels=0 control-pixels=25320
wgc-exclusion-off=marker-present control-present marker-pixels=32400 control-pixels=24600
wgc-exclusion-on=marker-absent control-present marker-pixels=0 control-pixels=25320
```

### windows-2025

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

The measured run's "Record the runner session" step printed `query user` and `query session`, then reported exit code 1. That is PowerShell keeping the native exit code from `query session`. The step is `continue-on-error` and is not a desktop failure. The workflow now exits 0 after printing that table.

Seam 3 tests must not assume the interactive session id is 1. On these runners it was 2. A service that needs the desktop has to launch into the console session; code running as the service itself stays on the disconnected Services desktop.

## Tickets that depend on this

Named dependents: [#4](https://github.com/whyarealltheuserstaken/softlock/issues/4) (fail-closed), [#5](https://github.com/whyarealltheuserstaken/softlock/issues/5) (hook visibility in CI), [#11](https://github.com/whyarealltheuserstaken/softlock/issues/11) (seam 3 probe harness).

Later tickets whose work includes a seam 3 test, and which can stay on these hosted images: [#12](https://github.com/whyarealltheuserstaken/softlock/issues/12), [#13](https://github.com/whyarealltheuserstaken/softlock/issues/13), [#14](https://github.com/whyarealltheuserstaken/softlock/issues/14), [#15](https://github.com/whyarealltheuserstaken/softlock/issues/15), [#16](https://github.com/whyarealltheuserstaken/softlock/issues/16), [#17](https://github.com/whyarealltheuserstaken/softlock/issues/17), [#18](https://github.com/whyarealltheuserstaken/softlock/issues/18), [#19](https://github.com/whyarealltheuserstaken/softlock/issues/19), [#20](https://github.com/whyarealltheuserstaken/softlock/issues/20), [#21](https://github.com/whyarealltheuserstaken/softlock/issues/21), [#22](https://github.com/whyarealltheuserstaken/softlock/issues/22), [#23](https://github.com/whyarealltheuserstaken/softlock/issues/23), [#25](https://github.com/whyarealltheuserstaken/softlock/issues/25), [#26](https://github.com/whyarealltheuserstaken/softlock/issues/26), [#27](https://github.com/whyarealltheuserstaken/softlock/issues/27), [#28](https://github.com/whyarealltheuserstaken/softlock/issues/28), [#29](https://github.com/whyarealltheuserstaken/softlock/issues/29), [#30](https://github.com/whyarealltheuserstaken/softlock/issues/30).

[#32](https://github.com/whyarealltheuserstaken/softlock/issues/32) mentions seam 3 and is deferred until an Enterprise or Education machine exists. [#24](https://github.com/whyarealltheuserstaken/softlock/issues/24) and [#31](https://github.com/whyarealltheuserstaken/softlock/issues/31) do not. [#33](https://github.com/whyarealltheuserstaken/softlock/issues/33) is the roadmap.
