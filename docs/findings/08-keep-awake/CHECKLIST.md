# Keep-awake checklist

Four laptop tests. Each one lasts 60 minutes. You start it, then you can leave. You do not watch the whole hour.

Times in the log are UTC. Logs are not committed. Running a scenario again replaces that scenario's log.

Do not change the Windows setting for what happens when you close the lid. If the laptop sleeps when the lid closes, that is a result. Write it down.

## 1. Practice (about 1 minute)

Open a terminal in the folder that contains `Cargo.toml`. The first command compiles. That takes several minutes. Later commands are quicker.

```
cargo run -p softlock-spike-keep-awake -- run --scenario practice --minutes 1
```

Read the line that starts with `Modern Standby (AoAc):`.

If it says `Modern Standby (AoAc): no`, stop. Do not run the four long tests. Write "no" and the laptop model in the finding. The model and Windows version are in Settings, then System, then About.

A dim screen appears. Clicks still reach the windows under it. Press any key to stop early.

The log is `spikes/keep-awake/logs/practice.log`.

## 2. Lid open, plugged in (60 minutes)

Leave the lid open. Leave the power cord plugged in.

```
cargo run -p softlock-spike-keep-awake -- run --scenario lid-open-plugged
```

The screen says `Leave the lid open. This run lasts 60 minutes. Press a key only to stop early.`

Leave it for 60 minutes. Pressing a key stops the test, so do not type on this laptop until it finishes.

The log is `spikes/keep-awake/logs/lid-open-plugged.log`.

## 3. Lid open, on battery (60 minutes)

Unplug the power cord. Leave the lid open. Wait until Windows shows the laptop is on battery, then run:

```
cargo run -p softlock-spike-keep-awake -- run --scenario lid-open-battery
```

The log is `spikes/keep-awake/logs/lid-open-battery.log`.

## 4. Lid closed, plugged in (60 minutes)

Plug the cord back in. Start the command, then wait until you see this exact line:

`You can close the lid now.`

Then close the lid. Open it again when the hour is up, or to stop early with any key.

```
cargo run -p softlock-spike-keep-awake -- run --scenario lid-closed-plugged
```

The log is `spikes/keep-awake/logs/lid-closed-plugged.log`.

## 5. Lid closed, on battery (60 minutes)

Unplug the cord. Same rule: wait for `You can close the lid now.` before you close the lid.

```
cargo run -p softlock-spike-keep-awake -- run --scenario lid-closed-battery
```

The log is `spikes/keep-awake/logs/lid-closed-battery.log`.

## 6. Read the logs

From the same folder:

```
cargo run -p softlock-spike-keep-awake -- analyze
```

Paste the whole summary into the finding. Copy each scenario's gap count, longest gap, lowest battery line, and thermal or throttling line into the table. Every cell starts as `(blank)`. Fill them from the summary. Do not tick the GitHub checkboxes until the blanks are filled and the four logs are saved.

| Scenario | Gaps longer than 30 s | Longest gap | Kept working? | Lowest battery % | Thermal or throttling |
| --- | --- | --- | --- | --- | --- |
| lid-open-plugged | (blank) | (blank) | (blank) | (blank) | (blank) |
| lid-open-battery | (blank) | (blank) | (blank) | (blank) | (blank) |
| lid-closed-plugged | (blank) | (blank) | (blank) | (blank) | (blank) |
| lid-closed-battery | (blank) | (blank) | (blank) | (blank) | (blank) |

The lowest battery and the thermal or throttling counts are the data for choosing a default threshold later. Leave the threshold itself blank.
