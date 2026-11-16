# Keep-awake checklist

Four laptop tests. Each one lasts 60 minutes. You start it, then you can leave. You do not watch the whole hour.

The whole job is about 4 hours 45 minutes of machine time and about 40 minutes of hands-on time. Add 20 to 40 minutes of charging if the battery is below 80%. The four runs can be split across days.

Times in the log are UTC. The length of a run uses a Windows clock that keeps counting through sleep and ignores a manual clock change. Logs are not committed. Running a scenario again replaces that scenario's log.

Do not change the Windows setting for what happens when you close the lid. If the laptop sleeps when the lid closes, that is a result. Write it down.

## Safety

Keep the laptop on a hard surface for the whole run. Never put it in a bag, on a bed, or on a blanket, especially with the lid closed.

A battery run starts only when Windows shows 80% or more. If it is lower, charge first. Windows' own critical-battery action still applies. The run stops itself at 20% battery or on heat. That stop is a result. Write the stop reason down and run that scenario again only if you meant to finish the hour.

A stray key ends the run. Run that scenario again. Ctrl+C also stops it.

## 1. Practice (about 3 minutes)

Open a terminal in the folder that contains `Cargo.toml`. The first command compiles. That takes several minutes. Later commands are quicker.

```
cargo run -p softlock-spike-keep-awake -- run --scenario practice --minutes 3
```

Read the line that starts with `Modern Standby (AoAc):`.

If it says `Modern Standby (AoAc): no`, stop. Do not run the four long tests. Write "no" and the laptop model in the finding. The model and Windows version are in Settings, then System, then About.

A dim screen appears. Clicks still reach the windows under it. Press any key to stop early.

Stay for this one. It lasts 3 minutes, which is longer than the 2-minute heat check. Each progress line prints `passive_limit` and `throttle_reasons`. If a heat counter stays tripped, the run stops early and says `heat-guard`. Write that down. That early stop is a result.

The log is `spikes/keep-awake/logs/practice.log`.

## 2. Lid open, plugged in (60 minutes)

Leave the lid open. Leave the power cord plugged in. Keep the laptop on a hard surface.

```
cargo run -p softlock-spike-keep-awake -- run --scenario lid-open-plugged
```

The screen says `Leave the lid open. This run lasts 60 minutes. Press a key only to stop early.`

Leave it for 60 minutes. Pressing a key stops the test, so do not type on this laptop until it finishes.

The log is `spikes/keep-awake/logs/lid-open-plugged.log`.

## 3. Lid open, on battery (60 minutes)

Unplug the power cord. Leave the lid open. Wait until Windows shows the laptop is on battery and the battery is 80% or more, then run:

```
cargo run -p softlock-spike-keep-awake -- run --scenario lid-open-battery
```

The log is `spikes/keep-awake/logs/lid-open-battery.log`.

## 4. Lid closed, plugged in (60 minutes)

Plug the cord back in. Start the command, then wait until you see this exact line:

`You can close the lid now.`

Then close the lid. The laptop stays on a hard surface. Do not put it in a bag, on a bed, or on a blanket. Open the lid again when the hour is up, or to stop early with any key.

```
cargo run -p softlock-spike-keep-awake -- run --scenario lid-closed-plugged
```

The log is `spikes/keep-awake/logs/lid-closed-plugged.log`.

## 5. Lid closed, on battery (60 minutes)

Unplug the cord. Windows must show 80% or more. Same rule: wait for `You can close the lid now.` before you close the lid. Hard surface only.

```
cargo run -p softlock-spike-keep-awake -- run --scenario lid-closed-battery
```

The log is `spikes/keep-awake/logs/lid-closed-battery.log`.

## 6. Read the logs

From the same folder:

```
cargo run -p softlock-spike-keep-awake -- analyze
```

Paste the whole summary into the finding. Copy each scenario's gap count, longest gap, lowest battery line, heat line, policy or cap line, and whether the result is INCOMPLETE into the table. Every cell starts as `(blank)`. Fill them from the summary. A scenario is INCOMPLETE if it ended early or the log is shorter than the planned hour (about a minute of slack is allowed). `ac mismatches` means a tick said the cord was in the wrong state for that scenario.

Do not tick the GitHub checkboxes until the blanks are filled and the four logs are saved.

| Scenario | Gaps longer than 30 s | Longest gap | Kept working? | Lowest battery % | Heat | Policy or cap | Complete? |
| --- | --- | --- | --- | --- | --- | --- | --- |
| lid-open-plugged | (blank) | (blank) | (blank) | (blank) | (blank) | (blank) | (blank) |
| lid-open-battery | (blank) | (blank) | (blank) | (blank) | (blank) | (blank) | (blank) |
| lid-closed-plugged | (blank) | (blank) | (blank) | (blank) | (blank) | (blank) | (blank) |
| lid-closed-battery | (blank) | (blank) | (blank) | (blank) | (blank) | (blank) | (blank) |

The lowest battery, the heat line, and the policy or cap line are the data for choosing a default threshold later. Leave the threshold itself blank.

## 7. Share the four logs

Drag these four files into a comment on issue #8:

- `spikes/keep-awake/logs/lid-open-plugged.log`
- `spikes/keep-awake/logs/lid-open-battery.log`
- `spikes/keep-awake/logs/lid-closed-plugged.log`
- `spikes/keep-awake/logs/lid-closed-battery.log`

They contain no private info. There are no window titles and no typed keys.
