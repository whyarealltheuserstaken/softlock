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

Do not paste a hardware serial number.

| Row | Result |
|---|---|
| `sw_vers` / `uname -m` | |
| Assertion names that compiled | |
| `pmset` shows reason string | |
| Timer count, overlay down | |
| Timer count, overlay up | |
| Covered process slowed? | |
| Lid closed on power | slept / awake (not "no lid") |
| Lid closed with an external display | slept / awake / no-external-display |
| Power key tap | slept / restart-panel / nothing |
| Assertions released | |
| Keep-awake claim for m16 | idle sleep only, pending these rows |

`pmset -g assertions` excerpt while held:

```
```
