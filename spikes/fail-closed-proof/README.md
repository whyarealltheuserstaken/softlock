# Fail-closed proof

CI only until the written finding is reviewed. The one-laptop steps live in `docs/findings/04-fail-closed/CHECKLIST.md`. Nobody runs those steps from this readme.

The probe applies the lockdown policy values, times the three OS-lock paths, and writes `report.txt`. It journals every previous value and restores them when it finishes, on Ctrl+C, on a panic, and from a helper process if it is killed. `--revert` restores them again and does not install the service.

The helper waits until the probe exits, then puts the values back and removes the temporary service. It stops waiting after 15 minutes and restores then, even if the probe is still running.

The orchestrator installs a Windows service that runs as SYSTEM. The service name is `FailClosedProbe`. It is a throwaway probe, not the product service. The probe refuses to install it unless `GITHUB_ACTIONS=true` or `--allow-local-service` is passed. The workflow also runs `--revert`, `sc.exe stop`, and `sc.exe delete` in a step that runs even when the proof is cancelled.

`--panic-after-arm` exists only in a build that passes `--features panic-after-arm`. A normal `cargo run` does not include it. The proof job uses that feature for one restore check, then runs the timing proof from a normal build.
