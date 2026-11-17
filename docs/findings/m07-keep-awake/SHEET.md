# m07 sheet

Copied from [Henry's sheet](../../tickets/macos/henry-sheet.md). Fill the rows after a kit run. This file is not a result until those rows have numbers and slept/awake text.

| | |
|---|---|
| Model | MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports) |
| Processor | 2 GHz quad-core Intel Core i5 |
| Graphics | Intel Iris Plus Graphics |
| Memory | 16 GB LPDDR4X |
| macOS | Tahoe 26.7.3 |
| Laptop or desktop | Laptop |
| Touch ID | Present |
| T2 | Present |
| External displays | Unknown. Still open. The kit still has the lid-closed-with-external-display row. |
| Second keyboard | Unknown. Still open |

Do not paste a hardware serial number. Do not paste a full `pmset -g assertions` dump.

| Row | Result |
|---|---|
| `sw_vers` / `uname -m` | |
| Assertion names that compiled | |
| Filtered `pmset` shows reason string | |
| Timer count, overlay down (60 s on `--kit`, 8 s on `--runner`) | |
| Timer count, overlay up (same duration) | |
| Timer occlusion visible down / up | |
| Covered process slowed? | |
| App Nap (60 s overlay-up drift, automatic) | |
| Lid closed on power (`assertion-held`) | slept / awake (not "no lid") |
| Lid closed with an external display (`assertion-held`) | slept / awake / no-external-display |
| Power key tap (`assertion-held`; Touch ID button, unlock first if it locks) | slept / restart-panel / nothing |
| Assertions released after those rows | |
| Keep-awake claim for m16 | idle sleep only, pending these rows |

Filtered `pmset` excerpt while held (summary counts and this probe's pid/reason only):

```
```
