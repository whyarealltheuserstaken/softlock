//! Which rows run, which wait for a person, and which stay skipped.

use serde_json::Value;

use crate::machine::{Machine, SessionKind, decode_facts};

pub const BUDGET_MS: u64 = 300_000;

pub const PROMPT_PLUG: &str = "Plug in a monitor, then press Enter. Type skip to skip.";
pub const PROMPT_UNPLUG: &str = "Unplug that monitor, then press Enter. Type skip to skip.";
pub const PROMPT_REPLUG: &str = "Plug that monitor back in, then press Enter. Type skip to skip.";
pub const PROMPT_LID: &str =
    "Close the lid, then open it. Sleep stays off. Then press Enter. Type skip to skip.";
pub const PROMPT_POWER: &str =
    "Tap the power button once. Sleep stays off. Then press Enter. Type skip to skip.";
pub const PROMPT_LOCK: &str =
    "Press Enter to lock Windows, then sign in with PIN or password. Type skip to skip.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flags {
    pub ci: bool,
    pub admin: bool,
    pub elevated: bool,
    pub github_actions: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Run,
    Prompt,
    Record,
    Skip,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Prompt => "prompt",
            Self::Record => "record",
            Self::Skip => "skip",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    pub id: &'static str,
    pub action: Action,
    pub reason: String,
    pub detail: String,
    pub prompts: Vec<&'static str>,
    pub physical: bool,
    pub needs_admin: bool,
    pub blocks_input: bool,
}

pub fn plan(machine: &Machine, flags: &Flags) -> Vec<Planned> {
    vec![
        record("detect-config"),
        run("parser-self-test"),
        overlay("overlay-topmost", machine),
        overlay("overlay-capture", machine),
        run("toast-hkcu-restore"),
        run("input-hook-injected"),
        run("keep-awake-request"),
        run("cad-hkcu-readback"),
        hklm("cad-hklm-readback", flags),
        fail_closed(flags),
        display_enum(machine),
        hotplug(machine, flags),
        lid(machine, flags),
        power_key(machine, flags),
        session_lock(machine, flags),
        record("ci-probes-stayed-off"),
    ]
}

pub fn plan_value(facts: &Value, flags: &Flags) -> Result<(Machine, Vec<Planned>), String> {
    if service_launch_allowed(flags.ci, flags.github_actions) {
        return Err("this suite does not install a SYSTEM service".into());
    }
    let machine = decode_facts(facts)?;
    let rows = plan(&machine, flags);
    debug_assert!(
        rows.iter().all(|row| !is_service_probe(row.id)),
        "the suite never plans a SYSTEM service probe"
    );
    Ok((machine, rows))
}

fn is_service_probe(id: &str) -> bool {
    id == "desktop-proof-service" || id == "input-hook-service"
}

#[allow(clippy::too_many_arguments)]
fn row(
    id: &'static str,
    action: Action,
    reason: &str,
    detail: &str,
    prompts: Vec<&'static str>,
    physical: bool,
    needs_admin: bool,
    blocks_input: bool,
) -> Planned {
    Planned {
        id,
        action,
        reason: reason.to_string(),
        detail: detail.to_string(),
        prompts,
        physical,
        needs_admin,
        blocks_input,
    }
}

fn run(id: &'static str) -> Planned {
    row(id, Action::Run, "", "", Vec::new(), false, false, false)
}

fn record(id: &'static str) -> Planned {
    row(id, Action::Record, "", "", Vec::new(), false, false, false)
}

fn skip(
    id: &'static str,
    reason: &str,
    detail: &str,
    physical: bool,
    needs_admin: bool,
) -> Planned {
    row(
        id,
        Action::Skip,
        reason,
        detail,
        Vec::new(),
        physical,
        needs_admin,
        false,
    )
}

fn overlay(id: &'static str, machine: &Machine) -> Planned {
    if machine.displays == 0 {
        skip(id, "not-applicable", "no display", false, false)
    } else {
        run(id)
    }
}

fn display_enum(machine: &Machine) -> Planned {
    if machine.displays == 0 {
        skip("display-enum", "not-applicable", "no display", false, false)
    } else {
        run("display-enum")
    }
}

fn hklm(id: &'static str, flags: &Flags) -> Planned {
    if !flags.admin && !flags.ci {
        return skip(id, "needs-admin", "needs --admin", false, true);
    }
    if !flags.elevated {
        return skip(id, "needs-admin", "needs an elevated prompt", false, true);
    }
    row(id, Action::Run, "", "", Vec::new(), false, true, false)
}

fn fail_closed(flags: &Flags) -> Planned {
    if flags.ci {
        return skip("fail-closed-blocked", "ci", "not run in --ci", false, true);
    }
    if !flags.admin {
        return skip(
            "fail-closed-blocked",
            "needs-admin",
            "needs --admin",
            false,
            true,
        );
    }
    if !flags.elevated {
        return skip(
            "fail-closed-blocked",
            "needs-admin",
            "needs an elevated prompt",
            false,
            true,
        );
    }
    row(
        "fail-closed-blocked",
        Action::Run,
        "",
        "",
        Vec::new(),
        false,
        true,
        true,
    )
}

fn physical(
    id: &'static str,
    flags: &Flags,
    machine: &Machine,
    prompts: Vec<&'static str>,
) -> Planned {
    if flags.ci {
        return skip(id, "ci", "physical step", true, false);
    }
    if machine.session == SessionKind::Remote {
        return skip(id, "not-applicable", "remote session", true, false);
    }
    let blocks = id == "session-lock";
    row(id, Action::Prompt, "", "", prompts, true, false, blocks)
}

fn hotplug(machine: &Machine, flags: &Flags) -> Planned {
    if machine.displays == 0 {
        return skip(
            "display-hotplug",
            "not-applicable",
            "no display",
            true,
            false,
        );
    }
    let prompts = if machine.displays >= 2 {
        vec![PROMPT_UNPLUG, PROMPT_REPLUG]
    } else {
        vec![PROMPT_PLUG, PROMPT_UNPLUG]
    };
    physical("display-hotplug", flags, machine, prompts)
}

fn lid(machine: &Machine, flags: &Flags) -> Planned {
    if flags.ci {
        return skip("lid-cycle", "ci", "physical step", true, false);
    }
    if machine.session == SessionKind::Remote {
        return skip("lid-cycle", "not-applicable", "remote session", true, false);
    }
    if machine.lid == "absent" {
        return skip("lid-cycle", "not-applicable", "no lid", true, false);
    }
    physical("lid-cycle", flags, machine, vec![PROMPT_LID])
}

fn power_key(machine: &Machine, flags: &Flags) -> Planned {
    if flags.ci {
        return skip("power-key", "ci", "physical step", true, false);
    }
    if machine.session == SessionKind::Remote {
        return skip("power-key", "not-applicable", "remote session", true, false);
    }
    if !machine.power.known {
        return skip(
            "power-key",
            "not-applicable",
            "could not read powercfg /a",
            true,
            false,
        );
    }
    if !machine.power.any_sleep() {
        return skip("power-key", "not-applicable", "no sleep state", true, false);
    }
    physical("power-key", flags, machine, vec![PROMPT_POWER])
}

fn session_lock(machine: &Machine, flags: &Flags) -> Planned {
    physical("session-lock", flags, machine, vec![PROMPT_LOCK])
}

pub fn service_launch_allowed(ci: bool, github_actions: bool) -> bool {
    // The suite never installs RunnerDesktopProbe or InputHookProbe.
    // Those binaries stay in their own workflows, and only on a GitHub runner.
    let _ = (ci, github_actions);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::decode_facts;
    use serde_json::json;

    fn machine() -> Machine {
        decode_facts(&json!({
            "current_build": "26100",
            "installation_type": "Client",
            "powercfg_a": "The following sleep states are available on this system:\n    Standby (S0 Low Power Idle)\n    Hibernate\nThe following sleep states are not available on this system:\n    Standby (S3)\n",
            "monitor_count": 1,
            "biometric": true,
            "pin": true,
            "client_protocol": 0,
            "elevated": false,
            "chassis_laptop": true,
            "ac_line": 1
        }))
        .unwrap()
    }

    fn flags(ci: bool, admin: bool, elevated: bool) -> Flags {
        Flags {
            ci,
            admin,
            elevated,
            github_actions: ci,
        }
    }

    fn row<'a>(rows: &'a [Planned], id: &str) -> &'a Planned {
        rows.iter().find(|row| row.id == id).expect(id)
    }

    #[test]
    fn the_school_profile_skips_hklm_and_keeps_physical_prompts() {
        let rows = plan(&machine(), &flags(false, false, false));
        assert_eq!(row(&rows, "cad-hklm-readback").action, Action::Skip);
        assert_eq!(row(&rows, "cad-hklm-readback").reason, "needs-admin");
        assert_eq!(row(&rows, "fail-closed-blocked").reason, "needs-admin");
        assert!(
            row(&rows, "fail-closed-blocked").blocks_input
                || row(&rows, "session-lock").blocks_input
        );
        assert!(row(&rows, "session-lock").blocks_input);
        assert_eq!(row(&rows, "cad-hkcu-readback").action, Action::Run);
        assert_eq!(row(&rows, "display-hotplug").action, Action::Prompt);
        assert_eq!(
            row(&rows, "display-hotplug").prompts,
            vec![PROMPT_PLUG, PROMPT_UNPLUG]
        );
        assert_eq!(row(&rows, "lid-cycle").action, Action::Prompt);
        assert_eq!(row(&rows, "session-lock").prompts, vec![PROMPT_LOCK]);
        assert!(rows.iter().all(|row| !is_service_probe(row.id)));
        assert!(!service_launch_allowed(false, false));
        assert!(!service_launch_allowed(true, true));
    }

    #[test]
    fn ci_skips_physical_rows_and_runs_hklm_only_when_already_elevated() {
        let skipped = plan(&machine(), &flags(true, false, false));
        assert_eq!(row(&skipped, "display-hotplug").reason, "ci");
        assert_eq!(row(&skipped, "lid-cycle").reason, "ci");
        assert_eq!(row(&skipped, "power-key").reason, "ci");
        assert_eq!(row(&skipped, "session-lock").reason, "ci");
        assert_eq!(row(&skipped, "fail-closed-blocked").reason, "ci");
        assert_eq!(row(&skipped, "cad-hklm-readback").reason, "needs-admin");
        assert_eq!(row(&skipped, "overlay-topmost").action, Action::Run);

        let elevated = plan(&machine(), &flags(true, false, true));
        assert_eq!(row(&elevated, "cad-hklm-readback").action, Action::Run);
        assert_eq!(row(&elevated, "fail-closed-blocked").action, Action::Skip);
    }

    #[test]
    fn admin_without_elevation_does_not_prompt_for_uac() {
        let rows = plan(&machine(), &flags(false, true, false));
        assert_eq!(row(&rows, "cad-hklm-readback").reason, "needs-admin");
        assert_eq!(
            row(&rows, "cad-hklm-readback").detail,
            "needs an elevated prompt"
        );
        assert_eq!(row(&rows, "fail-closed-blocked").reason, "needs-admin");
        let elevated = plan(&machine(), &flags(false, true, true));
        assert_eq!(row(&elevated, "cad-hklm-readback").action, Action::Run);
        assert_eq!(row(&elevated, "fail-closed-blocked").action, Action::Run);
        assert!(row(&elevated, "fail-closed-blocked").blocks_input);
    }

    #[test]
    fn a_desktop_without_a_lid_or_sleep_skips_those_rows() {
        let mut desktop = machine();
        desktop.lid = "absent".to_string();
        desktop.power.known = true;
        desktop.power.s3 = false;
        desktop.power.modern_standby = false;
        desktop.power.hibernate = false;
        desktop.displays = 2;
        let rows = plan(&desktop, &flags(false, false, false));
        assert_eq!(row(&rows, "lid-cycle").reason, "not-applicable");
        assert_eq!(row(&rows, "lid-cycle").detail, "no lid");
        assert_eq!(row(&rows, "power-key").reason, "not-applicable");
        assert_eq!(row(&rows, "power-key").detail, "no sleep state");
        assert_eq!(
            row(&rows, "display-hotplug").prompts,
            vec![PROMPT_UNPLUG, PROMPT_REPLUG]
        );
    }

    #[test]
    fn a_remote_session_skips_the_bench_steps() {
        let mut remote = machine();
        remote.session = SessionKind::Remote;
        let rows = plan(&remote, &flags(false, false, false));
        assert_eq!(row(&rows, "display-hotplug").reason, "not-applicable");
        assert_eq!(row(&rows, "lid-cycle").reason, "not-applicable");
        assert_eq!(row(&rows, "session-lock").reason, "not-applicable");
        assert_eq!(row(&rows, "display-hotplug").action, Action::Skip);
        assert_eq!(row(&rows, "keep-awake-request").action, Action::Run);
    }
}
