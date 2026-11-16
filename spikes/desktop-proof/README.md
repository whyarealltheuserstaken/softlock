# Desktop proof

CI only. Do not run this probe on a local machine.

The orchestrator installs a Windows service that runs as SYSTEM. The service binary is the copy under `target/`, and that directory is user-writable. Cleanup inside the process does not run if the process is killed.

The probe refuses to install the service unless `GITHUB_ACTIONS=true` or you pass `--allow-local-service`. The flag is only for a machine you can wipe. The desktop-proof workflow also runs `sc.exe stop` and `sc.exe delete` for `RunnerDesktopProbe` in a step that runs even when the proof is cancelled.

The service name is `RunnerDesktopProbe`. It is a throwaway probe for the GitHub-hosted Windows runners, not the product service and not the physical school machine.
