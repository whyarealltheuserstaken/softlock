# m03: posted events are software, a slow tap was not disabled, and the marker is the liveness check

Measured on 16 November 2026 UTC by the workspace-excluded probe `spikes/macos-desktop-proof`.

The later proof run is [macOS desktop proof run 43981349784](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43981349784), job [macos desktop proof](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43981349784/job/133698325260), on `beb281cce8eea55f9a40329bf91b0103dd3f928b`. The probe code is `9f895b66caee303c67a396ad823044fd355f582e`, which printed `result=pass` on [run 43980478472](https://github.com/whyarealltheuserstaken/softlock/actions/runs/43980478472). The later run also printed `result=pass`. There is no physical keyboard on this runner. Physical keys, clicks, scrolling, and a secure password field stay on Henry's Intel Mac (m05).

The runner is `macos-latest`, `uname -m` `arm64`, macOS 26.7.2. m02 had already created a live session tap on this image, so this ticket did not move to m05.

## Posted events

The probe installed a listen-only `kCGSessionEventTap` and posted keyCode 0 to `kCGSessionEventTap` under each source state. One poster was a second process. One poster was the tap's own process. The target was a key window in another process. `draft` is the rule in `docs/platforms/macos.md`: HID system state and source pid 0 is physical, and every other pair is software.

On `beb281c`:

```
post source=kCGEventSourceStatePrivate poster=second-process poster-pid=2231 state-id=1102520059 state-name=other state-matches=false source-pid=2231 tap-saw=false window-received=false draft=software error=tap did not see the key; fields are from the poster
post source=kCGEventSourceStatePrivate poster=same-process poster-pid=2228 state-id=2044897763 state-name=other state-matches=false source-pid=2228 tap-saw=true window-received=false draft=software error=none
post source=kCGEventSourceStateCombinedSessionState poster=second-process poster-pid=2235 state-id=0 state-name=kCGEventSourceStateCombinedSessionState state-matches=true source-pid=2235 tap-saw=true window-received=false draft=software error=none
post source=kCGEventSourceStateCombinedSessionState poster=same-process poster-pid=2228 state-id=0 state-name=kCGEventSourceStateCombinedSessionState state-matches=true source-pid=2228 tap-saw=true window-received=true draft=software error=none
post source=kCGEventSourceStateHIDSystemState poster=second-process poster-pid=2237 state-id=1 state-name=kCGEventSourceStateHIDSystemState state-matches=true source-pid=2237 tap-saw=true window-received=false draft=software error=none
post source=kCGEventSourceStateHIDSystemState poster=same-process poster-pid=2228 state-id=1 state-name=kCGEventSourceStateHIDSystemState state-matches=true source-pid=2228 tap-saw=true window-received=true draft=software error=none
```

The header value of `kCGEventSourceStatePrivate` is -1. Neither run read -1. Private posts came back as `1102520059` and `2044897763`, including the one row the tap did not see, whose fields were taken from the poster. Combined session was 0 and HID system was 1, which matches the header. Every source pid was the posting process, never 0.

The draft rule stands for these posts. All six are software on both runs. HID state did not mean physical. The rule is not replaced. It is also not proved for a finger on a key, because this runner never sent one.

Whether the target window received the post is not stable. On the earlier run the three second-process posts all reached the window, and the tap process's combined and HID posts did not. On the later run the tap process's combined and HID posts reached the window, and the second-process posts did not. The private second-process post was missed by the tap on the later run and seen by it on the earlier run. The job prints the boolean either way. m05 is the place to compare that with a physical key.

## Slow callback

The active session tap slept in the callback for about two seconds. No disable event arrived on either run. The key posted during the sleep did not reach the target during the sleep. The earlier run then saw that key after the callback returned (`key-reached-after=true`). The later run did not (`key-reached-after=false`). The probe did not re-enable the tap.

On `beb281c`:

```
slow disable-event=none sleep-ms=2076 disable-wait-ms=3653 key-reached-during=false key-reached-after=false reenabled=false error=none
fail-closed=Locked
```

Earlier run: `sleep-ms=2057`, `disable-wait-ms=3562`, `key-reached-during=false`, `key-reached-after=true`, `disable-event=none`, `reenabled=false`.

`disable-wait-ms` is how long the probe watched, from entering the callback until it gave up on a disable event. These runs do not show `kCGEventTapDisabledByTimeout` or `kCGEventTapDisabledByUserInput`. They do not show the system dropping a key while a tap is disabled, because the tap was not disabled. They also do not show a key getting through during the sleep. Fail closed stays. The probe does not re-enable the tap and call that success. A missing timeout is not proof the tap is healthy. That is the Windows liveness lesson, and it is why the marker exists.

## Liveness marker

The marker is a key the callback returns unchanged. With the tap off the run loop, the callback never ran. Three misses of 250 ms were inside one second on both runs. With the tap on the run loop, the callback saw the marker inside one heartbeat.

On `beb281c`:

```
marker dead-ms=829 dead-misses=3 healthy-ms=1 healthy-seen=true dead-within-1s=true healthy-within-heartbeat=true reenabled=false error=none
```

Earlier run: `dead-ms=942`, `healthy-ms=0`, `healthy-seen=true`, three misses, both bounds true. The dead row is the probe's own timer. It is not the operating system removing the tap.

## What m05 still has to measure

On Henry's Intel Mac, and not on this runner:

- A physical key, a physical click, and a trackpad scroll: state id, source pid, and whether the target received the event.
- Whether a physical key's source pid is 0. The draft rule swallows only that row. Neither run saw pid 0.
- Whether a physical event created with the private source state carries state id -1. Posted private events here did not.
- A secure password field: the tap goes quiet, receives `kCGEventTapDisabledByUserInput`, or keeps seeing keys.
- A spare USB keyboard, if he has one, plugged in while the tap is up.
- The window-delivery rows above, which already disagree between two runner runs.

No hardware serial belongs in this repo.
