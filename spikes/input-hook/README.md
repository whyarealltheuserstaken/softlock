# Input hook proof

CI only. Do not run this probe on a local machine.

The orchestrator installs a Windows service that runs as SYSTEM. The service binary is the copy under `target/`, and that directory is user-writable. Cleanup inside the process does not run if the process is killed.

The probe refuses to install the service unless `GITHUB_ACTIONS=true` or you pass `--allow-local-service`. The flag is only for a machine you can wipe. The input-hook workflow also runs `sc.exe stop` and `sc.exe delete` for `InputHookProbe`, then stops any leftover `input-hook` process, in a step that runs even when the proof is cancelled.

The service name is `InputHookProbe`. It is a throwaway probe for the GitHub-hosted Windows runners, not the product service. From that service the probe duplicates the SYSTEM token, sets the token session to the console session, and calls `CreateProcessAsUser` onto `winsta0\default`.
