# m03: posted events are software, a slow tap was not disabled, and the marker is the liveness check

Measured on 17 November 2026 UTC by the workspace-excluded probe `spikes/macos-desktop-proof`.

The proof log quoted below is [macOS desktop proof run 44009469018](https://github.com/whyarealltheuserstaken/softlock/actions/runs/44009469018), job [macos desktop proof](https://github.com/whyarealltheuserstaken/softlock/actions/runs/44009469018/job/133792709026), for commit `3ef5a4f`. That run printed `result=pass`. There is no physical keyboard on this runner. Physical keys, clicks, scrolling, and a secure password field stay on Henry's Intel Mac (m05).

The runner is `macos-latest`, `uname -m` `arm64`, macOS 26.7.2. m02 had already created a live session tap on this image, so this ticket did not move to m05. A later head of this branch re-runs the same probe. Process ids and millisecond timings move between runs. The ranges below are what the passing logs have shown. A run that prints `m03=no-tap` means the runner cannot host a tap, and that run fails. The log quoted here created the tap.

## Posted events

The probe installed a listen-only `kCGSessionEventTap` and posted keyCode 0 to `kCGSessionEventTap` under each source state. One poster was a second process. One poster was the tap's own process. The target was a key window in another process. `draft` is the rule in `docs/platforms/macos.md`: HID system state and source pid 0 is physical, and every other pair is software.

```
post source=kCGEventSourceStatePrivate poster=second-process poster-pid=2221 state-id=1102520059 state-name=other state-matches=false source-pid=2221 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStatePrivate poster=same-process poster-pid=2219 state-id=2044897763 state-name=other state-matches=false source-pid=2219 tap-saw=true window-received=false draft=software error=none
post source=kCGEventSourceStateCombinedSessionState poster=second-process poster-pid=2223 state-id=0 state-name=kCGEventSourceStateCombinedSessionState state-matches=true source-pid=2223 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStateCombinedSessionState poster=same-process poster-pid=2219 state-id=0 state-name=kCGEventSourceStateCombinedSessionState state-matches=true source-pid=2219 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStateHIDSystemState poster=second-process poster-pid=2224 state-id=1 state-name=kCGEventSourceStateHIDSystemState state-matches=true source-pid=2224 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStateHIDSystemState poster=same-process poster-pid=2219 state-id=1 state-name=kCGEventSourceStateHIDSystemState state-matches=true source-pid=2219 tap-saw=true window-received=true draft=software error=none
```

The header value of `kCGEventSourceStatePrivate` is -1. This log did not read -1. The private posts came back as `1102520059` and `2044897763` on every passing run. Combined session was 0 and HID system was 1, which matches the header. Every source pid was the posting process, never 0. The tap saw every post in this log. The target window received five of the six. It did not receive the private post from the tap's own process (`window-received=false`).

The draft rule stands for these posts. All six are software. HID state did not mean physical. The rule is not replaced. It is also not proved for a finger on a key, because this runner never sent one.

`window-received` is not stable across runs of this same probe. [Run 43982973565](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43982973565) on `1fc8dc6` printed `window-received=true` for all six posts. The lines quoted above are the `3ef5a4f` log.

`draft=software` means the event is not physical. It does not mean the event is allowed through. Remote-tool input, including Screen Sharing and VNC, is in the same posted-event class. It stays blocked until the remote pass. That pass is [m21](../tickets/macos/m21-capture-remote-spike.md) and [m22](../tickets/macos/m22-remote-session-lock.md), and SPEC.md Implementation Decision 20.

## Slow callback

The active session tap slept in the callback for about two seconds. No disable event arrived. The key posted during the sleep did not reach the target during the sleep. In this log it did reach the target after the callback returned. The probe did not re-enable the tap.

```
slow disable-event=none sleep-ms=2076 disable-wait-ms=3648 key-reached-during=false key-reached-after=true reenabled=false error=none
fail-closed=Locked
```

`disable-wait-ms=3648` is how long the probe watched, from entering the callback until it gave up on a disable event. The sleep itself was 2076 ms. Across passing runs, `sleep-ms` has been 2010–2076 and `disable-wait-ms` has been 3557–3648. This log does not show `kCGEventTapDisabledByTimeout` or `kCGEventTapDisabledByUserInput`. It does not show the system dropping a key while a tap is disabled, because the tap was not disabled. `key-reached-during` has been false on every passing run. `key-reached-after` has been observed both true and false: true on this log and on run 43982973565 (`1fc8dc6`), false on run 44008669740 (`cd1ff7d`) and run 43981349784 (`beb281c`). It is flaky. Fail closed stays either way. The probe does not re-enable the tap and call that success. A missing timeout is not proof the tap is healthy.

## Liveness marker

The marker is a key the callback returns unchanged. With the tap off the run loop, the callback never ran. Three misses of 250 ms were reported at 852 ms, inside one second. With the tap on the run loop, the callback saw the marker in the same millisecond it was posted, inside one heartbeat.

```
marker dead-ms=852 dead-misses=3 healthy-ms=0 healthy-seen=true dead-within-1s=true healthy-within-heartbeat=true reenabled=false error=none
```

Across passing runs, `dead-ms` has been 829–942 and `healthy-ms` has been 0 or 1. The dead row is the probe's own timer. It is not the operating system removing the tap.
