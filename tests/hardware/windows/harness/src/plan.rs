//! Which rows run, which wait for a person, and which stay skipped.

use serde_json::Value;

use crate::machine::{HelloKind, Machine, SessionKind, decode_facts};

pub const BUDGET_MS: u64 = 300_000;

pub const PROMPT_PLUG: &str = "Plug in a monitor, then press Enter. Type skip to skip.";
pub const PROMPT_UNPLUG: &str = "Unplug that monitor, then press Enter. Type skip to skip.";
pub const PROMPT_REPLUG: &str = "Plug that monitor back in, then press Enter. Type skip to skip.";
pub const PROMPT_UNPLUG_EXTERNAL: &str =
    "Unplug the external monitor, then press Enter. Type skip to skip.";
pub const PROMPT_REPLUG_EXTERNAL: &str =
    "Plug the external monitor back in, then press Enter. Type skip to skip.";
pub const PROMPT_EITHER: &str =
    "Either plug in a monitor or unplug one, then press Enter. Type skip to skip.";
pub const PROMPT_LID: &str = "Close the lid, then open it. The lid action is Do nothing. Then press Enter. Type skip to skip.";
pub const PROMPT_LOCK_HELLO: &str =
    "Press Enter to lock Windows, then sign in with Windows Hello. Type skip to skip.";
pub const PROMPT_LOCK_PIN: &str =
    "Press Enter to lock Windows, then sign in with your PIN. Type skip to skip.";
pub const PROMPT_LOCK_PASSWORD: &str =
    "Press Enter to lock Windows, then sign in with your password. Type skip to skip.";
pub const PROMPT_LOCK_ANY: &str = "Press Enter to lock Windows, then sign in. Type skip to skip.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flags {
    pub ci: bool,
    pub admin: bool,
    pub elevated: bool,
    pub github_actions: bool,
    pub school: bool,
}

/// `school` is only the explicit school profile. It does not change which rows run.
pub fn profile_name(flags: &Flags) -> &'static str {
    if flags.ci {
        "ci"
    } else if flags.admin {
        "admin"
    } else if flags.school {
        "school"
    } else {
        "bench"
    }
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
        keep_awake(flags),
        run("cad-hkcu-readback"),
        hklm("cad-hklm-readback", flags),
        fail_closed(flags),
        display_enum(machine),
        hotplug(machine, flags),
        lid(machine, flags),
        power_key(machine, flags),
        session_lock(machine, flags),
        run("restore"),
    ]
}

fn on_runner(flags: &Flags) -> bool {
    flags.ci && flags.github_actions
}

pub fn plan_value(facts: &Value, flags: &Flags) -> Result<(Machine, Vec<Planned>), String> {
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
    if !flags.admin && !on_runner(flags) {
        return skip(id, "needs-admin", "needs --admin", false, true);
    }
    if !flags.elevated {
        return skip(id, "needs-admin", "needs an elevated prompt", false, true);
    }
    row(id, Action::Run, "", "", Vec::new(), false, true, false)
}

fn fail_closed(flags: &Flags) -> Planned {
    if on_runner(flags) {
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

fn keep_awake(flags: &Flags) -> Planned {
    if !flags.elevated {
        return skip(
            "keep-awake-request",
            "needs-admin",
            "powercfg /requests needs an elevated prompt",
            false,
            true,
        );
    }
    run("keep-awake-request")
}

fn session_gate(id: &'static str, machine: &Machine) -> Option<Planned> {
    match machine.session {
        SessionKind::Console => None,
        SessionKind::Remote => Some(skip(id, "not-applicable", "remote session", true, false)),
        SessionKind::Unknown => Some(skip(id, "not-applicable", "session unknown", true, false)),
    }
}

fn physical(
    id: &'static str,
    flags: &Flags,
    machine: &Machine,
    prompts: Vec<&'static str>,
) -> Planned {
    if on_runner(flags) {
        return skip(id, "ci", "physical step", true, false);
    }
    if let Some(skipped) = session_gate(id, machine) {
        return skipped;
    }
    let blocks = id == "session-lock";
    row(id, Action::Prompt, "", "", prompts, true, false, blocks)
}

fn hotplug_prompts(machine: &Machine) -> Vec<&'static str> {
    if machine.displays == 0 {
        return vec![PROMPT_PLUG, PROMPT_UNPLUG];
    }
    if machine.displays >= 2 {
        return if machine.internal_display == Some(true) {
            vec![PROMPT_UNPLUG_EXTERNAL, PROMPT_REPLUG_EXTERNAL]
        } else {
            vec![PROMPT_UNPLUG, PROMPT_REPLUG]
        };
    }
    match machine.internal_display {
        Some(false) => vec![PROMPT_UNPLUG_EXTERNAL, PROMPT_REPLUG_EXTERNAL],
        Some(true) => vec![PROMPT_PLUG, PROMPT_UNPLUG],
        None => vec![PROMPT_EITHER],
    }
}

fn hotplug(machine: &Machine, flags: &Flags) -> Planned {
    physical("display-hotplug", flags, machine, hotplug_prompts(machine))
}

fn lid(machine: &Machine, flags: &Flags) -> Planned {
    if on_runner(flags) {
        return skip("lid-cycle", "ci", "physical step", true, false);
    }
    if let Some(skipped) = session_gate("lid-cycle", machine) {
        return skipped;
    }
    match machine.lid.as_str() {
        "absent" => skip("lid-cycle", "not-applicable", "no lid", true, false),
        "present" => physical("lid-cycle", flags, machine, vec![PROMPT_LID]),
        _ => skip("lid-cycle", "not-applicable", "lid unknown", true, false),
    }
}

fn power_key(_machine: &Machine, _flags: &Flags) -> Planned {
    // The chassis button is an ACPI control, not a HID key, and RegisterPowerSettingNotification
    // has no press GUID for it. With the action set to Do nothing, Windows writes no
    // language-independent event. https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerpowersettingnotification
    skip(
        "power-key",
        "not-applicable",
        "no language-independent power-button signal while the action is Do nothing",
        true,
        false,
    )
}

fn lock_prompt(hello: HelloKind) -> &'static str {
    match hello {
        HelloKind::Biometric => PROMPT_LOCK_HELLO,
        HelloKind::Pin => PROMPT_LOCK_PIN,
        HelloKind::Password => PROMPT_LOCK_PASSWORD,
        HelloKind::Unknown => PROMPT_LOCK_ANY,
    }
}

fn session_lock(machine: &Machine, flags: &Flags) -> Planned {
    physical(
        "session-lock",
        flags,
        machine,
        vec![lock_prompt(machine.hello)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{HelloKind, decode_facts};
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
            "ac_line": 1,
            "internal_display": true
        }))
        .unwrap()
    }

    fn flags(ci: bool, admin: bool, elevated: bool) -> Flags {
        Flags {
            ci,
            admin,
            elevated,
            github_actions: ci,
            school: false,
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
        assert_eq!(row(&rows, "session-lock").prompts, vec![PROMPT_LOCK_HELLO]);
        assert_eq!(row(&rows, "power-key").action, Action::Skip);
        assert_eq!(row(&rows, "power-key").reason, "not-applicable");
        assert!(rows.iter().all(|row| !is_service_probe(row.id)));
    }

    #[test]
    fn ci_outside_github_actions_does_not_change_the_plan() {
        let local = Flags {
            ci: true,
            admin: false,
            elevated: true,
            github_actions: false,
            school: false,
        };
        let rows = plan(&machine(), &local);
        assert_eq!(row(&rows, "cad-hklm-readback").action, Action::Skip);
        assert_eq!(row(&rows, "cad-hklm-readback").reason, "needs-admin");
        assert_eq!(row(&rows, "display-hotplug").action, Action::Prompt);
        assert_eq!(row(&rows, "fail-closed-blocked").reason, "needs-admin");
    }

    #[test]
    fn ci_skips_physical_rows_and_runs_hklm_only_when_already_elevated() {
        let skipped = plan(&machine(), &flags(true, false, false));
        assert_eq!(row(&skipped, "display-hotplug").reason, "ci");
        assert_eq!(row(&skipped, "lid-cycle").reason, "ci");
        assert_eq!(row(&skipped, "power-key").reason, "not-applicable");
        assert_eq!(
            row(&skipped, "power-key").detail,
            "no language-independent power-button signal while the action is Do nothing"
        );
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
        desktop.internal_display = Some(false);
        let rows = plan(&desktop, &flags(false, false, false));
        assert_eq!(row(&rows, "lid-cycle").reason, "not-applicable");
        assert_eq!(row(&rows, "lid-cycle").detail, "no lid");
        assert_eq!(row(&rows, "power-key").reason, "not-applicable");
        assert_eq!(
            row(&rows, "power-key").detail,
            "no language-independent power-button signal while the action is Do nothing"
        );
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
        assert_eq!(row(&rows, "keep-awake-request").reason, "needs-admin");
    }

    #[test]
    fn one_external_display_unplugs_instead_of_asking_for_a_new_monitor() {
        let mut external = machine();
        external.displays = 1;
        external.internal_display = Some(false);
        let rows = plan(&external, &flags(false, false, false));
        assert_eq!(
            row(&rows, "display-hotplug").prompts,
            vec![PROMPT_UNPLUG_EXTERNAL, PROMPT_REPLUG_EXTERNAL]
        );
        let mut panel = machine();
        panel.displays = 1;
        panel.internal_display = Some(true);
        let rows = plan(&panel, &flags(false, false, false));
        assert_eq!(
            row(&rows, "display-hotplug").prompts,
            vec![PROMPT_PLUG, PROMPT_UNPLUG]
        );
    }

    #[test]
    fn school_does_not_change_which_rows_run() {
        let bench = flags(false, false, false);
        let mut school = bench.clone();
        school.school = true;
        let bench_rows = plan(&machine(), &bench);
        let school_rows = plan(&machine(), &school);
        assert_eq!(profile_name(&bench), "bench");
        assert_eq!(profile_name(&school), "school");
        assert_eq!(bench_rows.len(), school_rows.len());
        for (left, right) in bench_rows.iter().zip(school_rows.iter()) {
            assert_eq!(left.id, right.id);
            assert_eq!(left.action, right.action);
            assert_eq!(left.reason, right.reason);
        }
    }

    fn precise(rows: &[Planned]) {
        for planned in rows {
            if planned.action == Action::Skip {
                assert!(!planned.reason.is_empty(), "{}", planned.id);
                assert!(!planned.detail.is_empty(), "{}", planned.id);
            }
            if planned.action == Action::Prompt {
                assert!(!planned.prompts.is_empty(), "{}", planned.id);
            }
        }
    }

    #[test]
    fn every_config_gets_a_step_or_a_precise_skip() {
        let bench = flags(false, false, false);
        let mut external = machine();
        external.displays = 1;
        external.internal_display = Some(false);
        external.lid = "present".to_string();
        external.power.known = true;
        external.power.s3 = true;
        external.power.modern_standby = false;
        external.power.hibernate = false;
        external.hello = HelloKind::Biometric;
        external.ac = "on-battery".to_string();
        let rows = plan(&external, &bench);
        precise(&rows);
        assert_eq!(
            row(&rows, "display-hotplug").prompts,
            vec![PROMPT_UNPLUG_EXTERNAL, PROMPT_REPLUG_EXTERNAL]
        );
        assert_eq!(row(&rows, "lid-cycle").prompts, vec![PROMPT_LID]);
        assert_eq!(row(&rows, "session-lock").prompts, vec![PROMPT_LOCK_HELLO]);
        assert_eq!(
            row(&rows, "keep-awake-request").detail,
            "powercfg /requests needs an elevated prompt"
        );
        assert_eq!(row(&rows, "power-key").reason, "not-applicable");
        assert_eq!(row(&rows, "overlay-topmost").action, Action::Run);

        let mut internal = machine();
        internal.displays = 1;
        internal.internal_display = Some(true);
        internal.hello = HelloKind::Pin;
        internal.ac = "plugged-in".to_string();
        internal.power.known = true;
        internal.power.s3 = false;
        internal.power.modern_standby = true;
        let rows = plan(&internal, &bench);
        precise(&rows);
        assert_eq!(
            row(&rows, "display-hotplug").prompts,
            vec![PROMPT_PLUG, PROMPT_UNPLUG]
        );
        assert_eq!(row(&rows, "session-lock").prompts, vec![PROMPT_LOCK_PIN]);
        assert_eq!(row(&rows, "lid-cycle").action, Action::Prompt);

        let mut both = machine();
        both.displays = 2;
        both.internal_display = Some(true);
        both.power.known = true;
        both.power.s3 = false;
        both.power.modern_standby = false;
        both.power.hibernate = true;
        let rows = plan(&both, &bench);
        precise(&rows);
        assert_eq!(
            row(&rows, "display-hotplug").prompts,
            vec![PROMPT_UNPLUG_EXTERNAL, PROMPT_REPLUG_EXTERNAL]
        );
        assert_eq!(row(&rows, "lid-cycle").action, Action::Prompt);

        let mut many_external = machine();
        many_external.displays = 3;
        many_external.internal_display = Some(false);
        many_external.lid = "absent".to_string();
        let rows = plan(&many_external, &bench);
        precise(&rows);
        assert_eq!(
            row(&rows, "display-hotplug").prompts,
            vec![PROMPT_UNPLUG, PROMPT_REPLUG]
        );
        assert_eq!(row(&rows, "lid-cycle").detail, "no lid");

        let mut headless = machine();
        headless.displays = 0;
        headless.internal_display = None;
        headless.lid = "unknown".to_string();
        headless.hello = HelloKind::Unknown;
        headless.ac = "unknown".to_string();
        headless.power.known = false;
        let rows = plan(&headless, &bench);
        precise(&rows);
        assert_eq!(
            row(&rows, "display-hotplug").prompts,
            vec![PROMPT_PLUG, PROMPT_UNPLUG]
        );
        assert_eq!(row(&rows, "overlay-topmost").detail, "no display");
        assert_eq!(row(&rows, "overlay-capture").detail, "no display");
        assert_eq!(row(&rows, "display-enum").detail, "no display");
        assert_eq!(row(&rows, "lid-cycle").detail, "lid unknown");
        assert_eq!(row(&rows, "session-lock").prompts, vec![PROMPT_LOCK_ANY]);

        let mut unknown_panel = machine();
        unknown_panel.displays = 1;
        unknown_panel.internal_display = None;
        let rows = plan(&unknown_panel, &bench);
        precise(&rows);
        assert_eq!(row(&rows, "display-hotplug").prompts, vec![PROMPT_EITHER]);

        let mut desktop = machine();
        desktop.lid = "absent".to_string();
        desktop.displays = 1;
        desktop.internal_display = Some(false);
        desktop.hello = HelloKind::Password;
        desktop.power.known = true;
        desktop.power.s3 = false;
        desktop.power.modern_standby = false;
        desktop.power.hibernate = false;
        let rows = plan(&desktop, &bench);
        precise(&rows);
        assert_eq!(row(&rows, "lid-cycle").detail, "no lid");
        assert_eq!(
            row(&rows, "session-lock").prompts,
            vec![PROMPT_LOCK_PASSWORD]
        );
        assert_eq!(
            row(&rows, "display-hotplug").prompts,
            vec![PROMPT_UNPLUG_EXTERNAL, PROMPT_REPLUG_EXTERNAL]
        );

        let mut remote = machine();
        remote.session = SessionKind::Remote;
        remote.displays = 2;
        let rows = plan(&remote, &bench);
        precise(&rows);
        assert_eq!(row(&rows, "display-hotplug").detail, "remote session");
        assert_eq!(row(&rows, "lid-cycle").detail, "remote session");
        assert_eq!(row(&rows, "session-lock").detail, "remote session");
        assert_eq!(row(&rows, "cad-hkcu-readback").action, Action::Run);

        let mut unknown_session = machine();
        unknown_session.session = SessionKind::Unknown;
        let rows = plan(&unknown_session, &bench);
        precise(&rows);
        assert_eq!(row(&rows, "display-hotplug").detail, "session unknown");
        assert_eq!(row(&rows, "lid-cycle").detail, "session unknown");
        assert_eq!(row(&rows, "session-lock").detail, "session unknown");

        let elevated = flags(false, true, true);
        let rows = plan(&external, &elevated);
        precise(&rows);
        assert_eq!(row(&rows, "keep-awake-request").action, Action::Run);
        assert_eq!(row(&rows, "cad-hklm-readback").action, Action::Run);
        assert_eq!(row(&rows, "fail-closed-blocked").action, Action::Run);

        let mut on_ac = external.clone();
        on_ac.ac = "plugged-in".to_string();
        on_ac.power.s3 = true;
        on_ac.power.modern_standby = false;
        on_ac.power.hibernate = true;
        let mut on_dc = on_ac.clone();
        on_dc.ac = "on-battery".to_string();
        let mut modern = on_ac.clone();
        modern.power.s3 = false;
        modern.power.modern_standby = true;
        modern.power.hibernate = false;
        let ac_rows = plan(&on_ac, &bench);
        let dc_rows = plan(&on_dc, &bench);
        let modern_rows = plan(&modern, &bench);
        precise(&ac_rows);
        precise(&dc_rows);
        precise(&modern_rows);
        for (left, right) in ac_rows.iter().zip(dc_rows.iter()) {
            assert_eq!(left.action, right.action);
            assert_eq!(left.prompts, right.prompts);
            assert_eq!(left.reason, right.reason);
        }
        assert_eq!(
            row(&ac_rows, "lid-cycle").action,
            row(&modern_rows, "lid-cycle").action
        );
        assert_eq!(row(&ac_rows, "lid-cycle").prompts, vec![PROMPT_LID]);
        assert_eq!(row(&modern_rows, "power-key").reason, "not-applicable");
    }
}
