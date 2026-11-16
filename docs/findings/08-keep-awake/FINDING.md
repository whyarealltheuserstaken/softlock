# Finding: keep-awake on Modern Standby and with the lid closed

Part of #8.

The results below are blank until the four laptop runs are done. The acceptance boxes stay unticked until those blanks are filled and the logs are saved.

## What was measured

The proof held a system-required power request and a display-required power request, and `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`. It did not set `ES_DISPLAY_REQUIRED`. The overlay was dim, stayed on top, let clicks through, and closed on any key or when the run ended.

The Windows lid-close action was recorded and not changed. Each progress tick wrote a UTC timestamp, one UI Automation sample, the battery percent, and the thermal and power-throttling signals Windows exposes (cooling mode, CPU speed limit, process execution-speed throttling, battery saver, and the low or critical battery flag). A 60-minute run is the data for choosing a default battery and heat floor. This finding does not choose that threshold, and the proof does not fall back to the real lock.

## Hardware

- Laptop model: (blank)
- Windows version: (blank)
- Modern Standby (AoAc): (blank)
- Lid sensor: (blank)
- Lid-close action on power: (blank)
- Lid-close action on battery: (blank)
- Hardware limits: (blank)

## Results

| Scenario | Gaps longer than 30 s | Longest gap | Kept working? | Lowest battery % | Thermal or throttling |
| --- | --- | --- | --- | --- | --- |
| lid-open-plugged | (blank) | (blank) | (blank) | (blank) | (blank) |
| lid-open-battery | (blank) | (blank) | (blank) | (blank) | (blank) |
| lid-closed-plugged | (blank) | (blank) | (blank) | (blank) | (blank) |
| lid-closed-battery | (blank) | (blank) | (blank) | (blank) | (blank) |

## Lid-close default

(blank)

`lid_close` stays off by default. It is offered only where this spike says the hardware can stay awake with the lid closed.

## Battery and heat floor

While locked, a later change falls back to the real lock and releases keep-awake requests if the battery drops below a threshold or a thermal problem is reported. The threshold is chosen from these logs. It is not chosen here.

- Default battery percent: (blank)
- What counts as a thermal problem: (blank)

## Dependents

- #20 Keep awake and online while locked, the power button, and the battery and heat floor
- #27 Optional takeover: Win+L, idle lock and lid close become SoftLock

## Acceptance

Copied from the issue. Leave these unticked.

- [ ] Proof build reports Modern Standby capability (AoAc) and holds the requests; Henry runs 4 scenarios (lid open/closed x plugged/battery) for 60 min each (manual, Henry's hardware)
- [ ] Progress log gaps longer than 30 s are counted per scenario and attached as evidence (manual, log attached)
- [ ] Written finding states which scenarios keep working, any hardware limits, the lid-close default, and names dependents 20 and 27

## Analyzer summary

(blank)
