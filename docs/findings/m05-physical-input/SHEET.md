# m05 sheet

Copied from [Henry's sheet](../../tickets/macos/henry-sheet.md). Fill the rows after a kit run. This file is not a result until those rows have state ids.

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
| External displays | Unknown. Still open. This kit does not use the lid. |
| Second keyboard | Unknown. Still open |

Do not paste a hardware serial number.

| Row | state id | source pid | target received | notes |
|---|---|---|---|---|
| `sw_vers` / `uname -m` | | | | must be `x86_64` on this Intel Mac |
| Physical key | | | | |
| Physical click | | | | |
| Trackpad scroll | | | | |
| Post `kCGEventSourceStatePrivate` (second process) | | | | |
| Post `kCGEventSourceStateCombinedSessionState` (second process) | | | | |
| Post `kCGEventSourceStateHIDSystemState` (second process) | | | | |
| Secure input (quiet / `kCGEventTapDisabledByUserInput` / still seeing) | | | | key reached another app? |
| USB keyboard hot-plug | | | | or no-second-keyboard |
| Draft rule confirmed or replaced | | | | name m14 |
| Tap released | | | | `tap-released=true` |
