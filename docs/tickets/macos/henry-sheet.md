# Henry's Intel Mac sheet

Header for the sheets under `docs/findings/`. The kit rows are still blank. This file records the machine. It does not record a test result.

Recorded 16 November 2026 from About This Mac. The same list is decision D1 in [`decisions.md`](decisions.md).

| | |
|---|---|
| Model | MacBook Pro (13-inch, 2020, Four Thunderbolt 3 ports) |
| Processor | 2 GHz quad-core Intel Core i5 |
| Graphics | Intel Iris Plus Graphics |
| Memory | 16 GB LPDDR4X |
| macOS | Tahoe 26.7.3 |
| Laptop or desktop | Laptop |
| Touch ID | Present |
| T2 | Present. That model has Touch ID and the T2 chip |
| External displays | He sometimes uses an external display. How many are attached at once is still unknown |
| Second keyboard | Unknown. Still open |

macOS 26.7.3 is well above the macOS 11 floor. Arm64 CI on `macos-latest` plus this Intel machine on 26.x means macOS 11 through 15 are untested. That is a known gap. A finding on this machine does not close it.

Each hardware ticket has a test-machine section that points here. A sheet written after a kit run copies this header and then fills that ticket's rows.
