# m03: posted events are software, a slow tap was not disabled, and the marker is the liveness check

Measured on 16 November 2026 UTC by the workspace-excluded probe `spikes/macos-desktop-proof`.

The proof run is [macOS desktop proof run 43980478472](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43980478472), job [macos desktop proof](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43980478472/job/133695338142). The measured head is `9f895b66caee303c67a396ad823044fd355f582e`. The job printed `result=pass`. There is no physical keyboard on this runner. Physical keys, clicks, scrolling, and a secure password field stay on Henry's Intel Mac (m05).

The runner is `macos-latest`, `uname -m` `arm64`, macOS 26.7.2. m02 had already created a live session tap on this image, so this ticket did not move to m05.

## Posted events

The probe installed a listen-only `kCGSessionEventTap` and posted keyCode 0 to `kCGSessionEventTap` under each source state. One poster was a second process. One poster was the tap's own process. The target was a key window in another process. The state id and source pid below are what the tap read. `draft` is the rule in `docs/platforms/macos.md`: HID system state and source pid 0 is physical, and every other pair is software.

```
post source=kCGEventSourceStatePrivate poster=second-process poster-pid=4430 state-id=1102520059 state-name=other state-matches=false source-pid=4430 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStatePrivate poster=same-process poster-pid=4428 state-id=2044897763 state-name=other state-matches=false source-pid=4428 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStateCombinedSessionState poster=second-process poster-pid=4433 state-id=0 state-name=kCGEventSourceStateCombinedSessionState state-matches=true source-pid=4433 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStateCombinedSessionState poster=same-process poster-pid=4428 state-id=0 state-name=kCGEventSourceStateCombinedSessionState state-matches=true source-pid=4428 tap-saw=true window-received=false draft=software error=none
post source=kCGEventSourceStateHIDSystemState poster=second-process poster-pid=4434 state-id=1 state-name=kCGEventSourceStateHIDSystemState state-matches=true source-pid=4434 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStateHIDSystemState poster=same-process poster-pid=4428 state-id=1 state-name=kCGEventSourceStateHIDSystemState state-matches=true source-pid=4428 tap-saw=true window-received=false draft=software error=none
```

The header value of `kCGEventSourceStatePrivate` is -1. The tap did not read -1. The two private posts came back as `1102520059` and `2044897763`. Combined session was 0 and HID system was 1, which matches the header. Every source pid was the posting process, never 0.

The draft rule stands for these posts. All six are software. HID state did not mean physical. The rule is not replaced. It is also not proved for a finger on a key, because this runner never sent one.

A second process's post reached the target window under all three states. The tap process's own combined-session post and HID post were seen by the tap and did not reach that window.

## Slow callback

The active session tap slept in the callback for about two seconds. No disable event arrived. The key posted during the sleep did not reach the target during the sleep. It reached the target after the callback returned. The probe did not re-enable the tap.

```
slow disable-event=none sleep-ms=2057 disable-wait-ms=3562 key-reached-during=false key-reached-after=true reenabled=false error=none
fail-closed=Locked
```

`disable-wait-ms=3562` is how long the probe watched, from entering the callback until it gave up on a disable event. The sleep itself was 2057 ms. This run does not show `kCGEventTapDisabledByTimeout` or `kCGEventTapDisabledByUserInput`. It does not show the system dropping a key while a tap is disabled, because the tap was not disabled. The key was held and then delivered. That is the same shape as the Windows finding: a timeout report is not proof the tap is gone, and no timeout report is not proof the tap is healthy. Fail closed stays. The probe does not re-enable the tap and call that success.

## Liveness marker

The marker is a key the callback returns unchanged. With the tap off the run loop, the callback never ran. Three misses of 250 ms were reported at 942 ms, inside one second. With the tap on the run loop, the callback saw the marker in the same millisecond it was posted, inside one heartbeat.

```
marker dead-ms=942 dead-misses=3 healthy-ms=0 healthy-seen=true dead-within-1s=true healthy-within-heartbeat=true reenabled=false error=none
```

The dead row is the probe's own timer. It is not the operating system removing the tap.

## What m05 still has to measure

On Henry's Intel Mac, and not on this runner:

- A physical key, a physical click, and a trackpad scroll: state id, source pid, and whether the target received the event.
- Whether a physical key's source pid is 0. The draft rule swallows only that row. This run never saw pid 0.
- Whether a physical event created with the private source state carries state id -1. Posted private events here did not.
- A secure password field: the tap goes quiet, receives `kCGEventTapDisabledByUserInput`, or keeps seeing keys.
- A spare USB keyboard, if he has one, plugged in while the tap is up.

No hardware serial belongs in this repo.
