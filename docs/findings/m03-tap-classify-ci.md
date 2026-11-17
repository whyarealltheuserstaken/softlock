# m03: posted events are software, a slow tap was not disabled, and the marker is the liveness check

Measured on 16 November 2026 UTC by the workspace-excluded probe `spikes/macos-desktop-proof`.

The proof log for commit `1fc8dc60896cb3a7a02e3f7ec8174cea9cfe5a13` is [macOS desktop proof run 43982973565](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43982973565), job [macos desktop proof](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43982973565/job/133703849121). That commit printed `result=pass`. The lines below are from that log. There is no physical keyboard on this runner. Physical keys, clicks, scrolling, and a secure password field stay on Henry's Intel Mac (m05).

The runner is `macos-latest`, `uname -m` `arm64`, macOS 26.7.2. m02 had already created a live session tap on this image, so this ticket did not move to m05.

## Posted events

The probe installed a listen-only `kCGSessionEventTap` and posted keyCode 0 to `kCGSessionEventTap` under each source state. One poster was a second process. One poster was the tap's own process. The target was a key window in another process. `draft` is the rule in `docs/platforms/macos.md`: HID system state and source pid 0 is physical, and every other pair is software.

```
post source=kCGEventSourceStatePrivate poster=second-process poster-pid=2965 state-id=1102520059 state-name=other state-matches=false source-pid=2965 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStatePrivate poster=same-process poster-pid=2962 state-id=2044897763 state-name=other state-matches=false source-pid=2962 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStateCombinedSessionState poster=second-process poster-pid=2967 state-id=0 state-name=kCGEventSourceStateCombinedSessionState state-matches=true source-pid=2967 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStateCombinedSessionState poster=same-process poster-pid=2962 state-id=0 state-name=kCGEventSourceStateCombinedSessionState state-matches=true source-pid=2962 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStateHIDSystemState poster=second-process poster-pid=2969 state-id=1 state-name=kCGEventSourceStateHIDSystemState state-matches=true source-pid=2969 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStateHIDSystemState poster=same-process poster-pid=2962 state-id=1 state-name=kCGEventSourceStateHIDSystemState state-matches=true source-pid=2962 tap-saw=true window-received=true draft=software error=none
```

The header value of `kCGEventSourceStatePrivate` is -1. This log did not read -1. The private posts came back as `1102520059` and `2044897763`. Combined session was 0 and HID system was 1, which matches the header. Every source pid was the posting process, never 0. The tap saw every post, and the target window received every post.

The draft rule stands for these posts. All six are software. HID state did not mean physical. The rule is not replaced. It is also not proved for a finger on a key, because this runner never sent one.

Earlier passing runs on the same probe did not all agree about `window-received`. [Run 43980478472](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43980478472) left the tap process's combined and HID posts unseen by the window. [Run 43981349784](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43981349784) left the second-process posts unseen, and the private second-process post was missed by the tap. This commit's log is the one cited above: `window-received=true` on all six rows.

## Slow callback

The active session tap slept in the callback for about two seconds. No disable event arrived. The key posted during the sleep did not reach the target during the sleep. It did reach the target after the callback returned. The probe did not re-enable the tap.

```
slow disable-event=none sleep-ms=2010 disable-wait-ms=3557 key-reached-during=false key-reached-after=true reenabled=false error=none
fail-closed=Locked
```

`disable-wait-ms=3557` is how long the probe watched, from entering the callback until it gave up on a disable event. The sleep itself was 2010 ms. This log does not show `kCGEventTapDisabledByTimeout` or `kCGEventTapDisabledByUserInput`. It does not show the system dropping a key while a tap is disabled, because the tap was not disabled. The key was held and then delivered (`key-reached-after=true`). An earlier run on `beb281c` printed `key-reached-after=false` for the same probe. Fail closed stays either way. The probe does not re-enable the tap and call that success. A missing timeout is not proof the tap is healthy.

## Liveness marker

The marker is a key the callback returns unchanged. With the tap off the run loop, the callback never ran. Three misses of 250 ms were reported at 928 ms, inside one second. With the tap on the run loop, the callback saw the marker in the same millisecond it was posted, inside one heartbeat.

```
marker dead-ms=928 dead-misses=3 healthy-ms=0 healthy-seen=true dead-within-1s=true healthy-within-heartbeat=true reenabled=false error=none
```

The dead row is the probe's own timer. It is not the operating system removing the tap.

## What m05 still has to measure

On Henry's Intel Mac, and not on this runner:

- A physical key, a physical click, and a trackpad scroll: state id, source pid, and whether the target received the event.
- Whether a physical key's source pid is 0. The draft rule swallows only that row. This log never saw pid 0.
- Whether a physical event created with the private source state carries state id -1. Posted private events here did not.
- A secure password field: the tap goes quiet, receives `kCGEventTapDisabledByUserInput`, or keeps seeing keys.
- A spare USB keyboard, if he has one, plugged in while the tap is up.

No hardware serial belongs in this repo.
