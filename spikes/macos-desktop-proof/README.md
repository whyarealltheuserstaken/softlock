# macOS desktop proof

CI only. Do not run this probe on a local machine.

The orchestrator loads a throwaway LaunchAgent and a throwaway LaunchDaemon. The daemon binary is the copy under `target/`, and that directory is user-writable. Cleanup inside the process does not run if the process is killed.

The probe refuses to install the daemon unless `GITHUB_ACTIONS=true` or you pass `--allow-local-service`. The flag is only for a machine you can wipe. The workflow does not run on push. Its last step runs `launchctl bootout` for `com.runner.macosproof.agent` and `com.runner.macosproof.daemon` even when the proof is cancelled.

The labels are throwaway probe ids. They are not the product's launchd labels. The probe does not lock the session. A missed observation is a line in the job log.

Apple silicon CI is `macos-latest`. The probe prints `sw_vers` and `uname -m`, and `uname -m` has to be `arm64`. Anything that needs a physical Mac is written up in the finding, in its own section, and is not run here.

The same job records m03. For each of the three `CGEventSource` states it prints the state id, the source pid, and whether a second process's posted key reached the target window. It sleeps inside an active tap callback and records the disable event, or that none arrived. It does not re-enable that tap. The liveness marker posts a key the callback returns unchanged. A callback whose run loop is not running is a dead callback: three misses, and the report has to land within one second. A callback that sees the marker has to do so within one heartbeat of 250 ms. A failure ends `Locked`.

`simulated-physical` is a test-only feature of this probe. The proof binary is built without it. The feature is how a test nonce can classify a posted event as physical. It is not in the proof run.
