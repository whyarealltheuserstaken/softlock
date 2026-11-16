//! Kit report. The log is rendered from this, and the exit code follows it.

use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::args::{APP_NAP_SAMPLE_MS, DEFAULT_INTERVAL_MS, REASON_PREFIX, RUNNER_SAMPLE_MS};
use crate::scrub::{contains_serial, filter_pmset, scrub};
use crate::text::one_line;

pub const KIT_ID: &str = "m07-keep-awake";
pub const SYSTEM_ASSERTION: &str = "PreventUserIdleSystemSleep";
pub const DISPLAY_ASSERTION: &str = "PreventUserIdleDisplaySleep";
pub const NO_IDLE: &str = "NoIdleSleepAssertion";
pub const NO_DISPLAY: &str = "NoDisplaySleepAssertion";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionReport {
    pub product_name: String,
    pub product_version: String,
    pub build_version: String,
    pub uname_m: String,
    pub hw_model: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssertionNames {
    pub prevent_user_idle_system_sleep: bool,
    pub prevent_user_idle_display_sleep: bool,
    pub no_idle_sleep: bool,
    pub no_display_sleep: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssertionReport {
    pub names_compiled: AssertionNames,
    pub reason: String,
    pub created: bool,
    pub released: bool,
    pub pmset_while_held: String,
    pub pmset_after_release: String,
    pub pmset_shows_reason: bool,
    pub pmset_shows_system: bool,
    pub pmset_shows_display: bool,
    pub pmset_after_shows_reason: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaffeinateReport {
    pub present: bool,
    pub ran: bool,
    pub pmset_showed_caffeinate: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimerReport {
    pub interval_ms: u64,
    pub duration_ms: u64,
    pub overlay_down_count: Option<u64>,
    pub overlay_up_count: Option<u64>,
    pub overlay_ordered_front: bool,
    pub overlay_opaque: bool,
    pub overlay_display_count: i32,
    pub occlusion_visible_down: Option<bool>,
    pub occlusion_visible_up: Option<bool>,
    pub expected_ticks: u64,
    pub app_nap_ms: u64,
    pub app_nap_count: Option<u64>,
    pub app_nap_occlusion_visible: Option<bool>,
    pub app_nap_ran: bool,
    pub slowed: bool,
    pub sentence: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardwareRow {
    pub status: String,
    pub outcome: Option<String>,
    pub heartbeat_gap_ms: Option<u64>,
    pub assertion_held: bool,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KitReport {
    pub kit: String,
    pub mode: String,
    pub version: VersionReport,
    pub display_count: i32,
    pub assertions: AssertionReport,
    pub caffeinate: CaffeinateReport,
    pub timer: TimerReport,
    pub lid_on_power: HardwareRow,
    pub lid_external: HardwareRow,
    pub power_key: HardwareRow,
    pub keep_awake_claim: String,
    pub hardware_only: Vec<String>,
    pub serial_collected: bool,
    pub locked_session: bool,
    pub reversible: bool,
    pub interrupted: bool,
    pub probe_crash: Option<String>,
}

pub fn laptop_lid_row(outcome: &str) -> Result<String, String> {
    match outcome {
        "slept" | "awake" | "skip" | "not-run-on-runner" | "needs-terminal" => {
            Ok(outcome.to_string())
        }
        "no-lid" | "no lid" | "nolid" => {
            Err("this Mac is a laptop, so \"no lid\" is not the row".to_string())
        }
        other => Err(format!("unknown lid outcome {other}")),
    }
}

pub fn lid_external_row(outcome: &str) -> Result<String, String> {
    match outcome {
        "slept"
        | "awake"
        | "no-external-display"
        | "skip"
        | "not-run-on-runner"
        | "needs-terminal" => Ok(outcome.to_string()),
        "no-lid" | "no lid" => {
            Err("this Mac is a laptop, so \"no lid\" is not the row".to_string())
        }
        other => Err(format!("unknown lid-external outcome {other}")),
    }
}

pub fn power_key_row(outcome: &str) -> Result<String, String> {
    match outcome {
        "slept" | "restart-panel" | "nothing" | "skip" | "not-run-on-runner" | "needs-terminal" => {
            Ok(outcome.to_string())
        }
        other => Err(format!("unknown power-key outcome {other}")),
    }
}

pub fn runner_hardware_row(subject: &str, assertion_held: bool) -> HardwareRow {
    HardwareRow {
        status: "not-run-on-runner".to_string(),
        outcome: None,
        heartbeat_gap_ms: None,
        assertion_held,
        note: format!(
            "{subject} needs Henry's Intel Mac. The GitHub runner has no lid and no power key."
        ),
    }
}

pub fn hardware_row(
    status: String,
    outcome: Option<String>,
    gap: Option<u64>,
    assertion_held: bool,
    note: &str,
) -> HardwareRow {
    HardwareRow {
        status,
        outcome,
        heartbeat_gap_ms: gap,
        assertion_held,
        note: note.to_string(),
    }
}

pub fn expected_ticks(sample_ms: u64, interval_ms: u64) -> u64 {
    let step = interval_ms.saturating_add(10).max(1);
    sample_ms / step
}

pub fn pmset_shows(excerpt: &str, needle: &str) -> bool {
    excerpt.contains(needle)
}

pub fn slowed_sentence(down: u64, up: u64) -> (bool, String) {
    if down == 0 {
        return (
            false,
            "No overlay-down count was recorded, so slowing is not claimed.".to_string(),
        );
    }
    let slowed = up * 100 < down * 80;
    let sentence = if slowed {
        format!("The covered process slowed ({up} ticks with the overlay up, {down} with it down).")
    } else {
        format!(
            "The covered process did not slow in this sample ({up} ticks with the overlay up, {down} with it down)."
        )
    };
    (slowed, sentence)
}

pub fn keep_awake_claim(assertions_held: bool, lid_on_power: &HardwareRow) -> String {
    if !assertions_held {
        return "keep-awake must not be claimed; the idle-sleep assertions were not held"
            .to_string();
    }
    match lid_on_power.outcome.as_deref() {
        Some("awake") => {
            "idle-sleep assertions held, and this lid-closed-on-power run stayed awake; phase 2 still does not offer lid-close takeover (D7). m16 may claim keep-awake for idle sleep only after the sheet is filled."
                .to_string()
        }
        Some("slept") => {
            "idle-sleep assertions held; lid close slept. m16 may claim keep-awake for idle sleep only. It must not claim agents kept running with the lid shut."
                .to_string()
        }
        _ => {
            "idle-sleep assertions can be created. m16 may claim keep-awake for idle sleep only after Henry's lid row is filled. It must not claim a lid-close takeover."
                .to_string()
        }
    }
}

pub fn hardware_only_list() -> Vec<String> {
    vec![
        "lid-closed-on-power".to_string(),
        "lid-closed-with-external-display".to_string(),
        "power-key".to_string(),
        "App Nap of a covered process on a real idle Intel Mac".to_string(),
    ]
}

pub fn instructions(kit: bool) -> &'static str {
    if kit {
        "step=1 Stay at the Mac. This kit does not lock it and does not swallow the keyboard.\n\
         step=2 It holds both IOPMAssertionCreateWithName idle-system and idle-display assertions through the lid and power-key steps (QA1340). They are released afterwards.\n\
         step=3 A timer process ticks. The same sample runs with the overlay down, then covering that window. The overlay is fully opaque, ignores the mouse, and does not become key. Escape ends it early only if Terminal has Accessibility permission; otherwise it closes by itself after about 60 s.\n\
         step=4 Hands-off App Nap: the overlay covers the timer for 300 s (--sample-ms 300000). Do not touch the Mac. Check the App Nap column in Activity Monitor for the timer process.\n\
         step=5 Lid closed on power, no external display if you can unplug it. Stay next to the machine. After about 20 seconds, open the lid. Type slept, awake, or skip. \"no lid\" is not a row on this laptop. Both assertions stay held.\n\
         step=6 If you use an external display, plug it in, wait until it shows the desktop, close the lid, wait, open the lid. Type slept, awake, no-external-display, or skip. Both assertions stay held.\n\
         step=7 Tap the power key once. Do not hold it. Type slept, restart-panel, nothing, or skip. Both assertions are still held for this row (assertion-held=true). They are released after this step. Nothing is left armed. Paste the filtered JSON onto the sheet."
    } else {
        "step=1 Runner proof only. No lid, no power key, no real idle App Nap.\n\
         step=2 Create idle-system and idle-display assertions with IOPMAssertionCreateWithName and keep them held.\n\
         step=3 Confirm the probe reason in a filtered pmset excerpt (summary counts plus this pid/reason only).\n\
         step=4 If caffeinate exists, run it as caffeinate -i -t 5 -w <kit pid>.\n\
         step=5 Timer counts with the overlay down and covering that window, same duration. Overlay is fully opaque. Record timer occlusion.\n\
         step=6 Hardware rows stay not-run-on-runner while both assertions are still held (assertion-held=true).\n\
         step=7 Release both assertions after those rows."
    }
}

pub fn failure_reasons(report: &KitReport) -> Vec<String> {
    let mut reasons = Vec::new();
    if report.serial_collected {
        reasons.push("a hardware serial number was collected".to_string());
    }
    if report.locked_session {
        reasons.push("the kit must not lock the session".to_string());
    }
    if report.interrupted {
        reasons.push("the kit was interrupted".to_string());
    }
    if report.mode == "runner" && report.version.uname_m != "arm64" {
        reasons.push(format!(
            "runner uname -m is {}, expected arm64",
            report.version.uname_m
        ));
    }
    if !report
        .assertions
        .names_compiled
        .prevent_user_idle_system_sleep
    {
        reasons.push("PreventUserIdleSystemSleep did not compile".to_string());
    }
    if !report
        .assertions
        .names_compiled
        .prevent_user_idle_display_sleep
    {
        reasons.push("PreventUserIdleDisplaySleep did not compile".to_string());
    }
    if !report.assertions.created {
        reasons.push("IOPMAssertionCreateWithName did not create both idle assertions".to_string());
    }
    if !report.assertions.pmset_shows_reason {
        reasons.push("pmset -g assertions did not show the probe reason string".to_string());
    }
    if !report.assertions.released {
        reasons.push("the idle assertions were not released".to_string());
    }
    if report.assertions.pmset_after_shows_reason {
        reasons.push("pmset still showed the probe reason after release".to_string());
    }
    if !report.reversible {
        reasons.push("the kit did not reverse its power assertions".to_string());
    }
    if report.timer.overlay_down_count.is_none() || report.timer.overlay_up_count.is_none() {
        reasons.push("timer counts for overlay down and overlay up are missing".to_string());
    }
    if !report.timer.overlay_ordered_front {
        reasons.push("overlay did not order front over the timer window".to_string());
    }
    if let Some(crash) = &report.probe_crash {
        reasons.push(format!("probe crash: {crash}"));
    }
    if contains_serial(&report.assertions.pmset_while_held)
        || contains_serial(&report.assertions.pmset_after_release)
        || contains_serial(&report.version.hw_model)
    {
        reasons.push("serial text reached the report".to_string());
    }
    if pmset_leaks_other_owners(&report.assertions.pmset_while_held)
        || pmset_leaks_other_owners(&report.assertions.pmset_after_release)
    {
        reasons.push("pmset excerpt named another process, path, or device".to_string());
    }
    if report.mode == "kit"
        && (report.lid_on_power.status == "recorded" || report.power_key.status == "recorded")
        && (!report.lid_on_power.assertion_held || !report.power_key.assertion_held)
    {
        reasons
            .push("idle assertions were not held through the lid and power-key rows".to_string());
    }
    if report.lid_on_power.status == "no-lid"
        || report.lid_on_power.outcome.as_deref() == Some("no-lid")
    {
        reasons.push("this Mac is a laptop, so \"no lid\" is not the row".to_string());
    }
    reasons
}

pub fn render(report: &KitReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("kit={}\n", report.kit));
    out.push_str(&format!("mode={}\n", report.mode));
    out.push_str(instructions(report.mode == "kit"));
    out.push('\n');
    out.push_str(&format!(
        "sw_vers-product={}\n",
        one_line(&report.version.product_name)
    ));
    out.push_str(&format!(
        "sw_vers-version={}\n",
        one_line(&report.version.product_version)
    ));
    out.push_str(&format!(
        "sw_vers-build={}\n",
        one_line(&report.version.build_version)
    ));
    out.push_str(&format!("uname-m={}\n", one_line(&report.version.uname_m)));
    out.push_str(&format!(
        "hw-model={}\n",
        one_line(&report.version.hw_model)
    ));
    out.push_str(&format!("display-count={}\n", report.display_count));
    out.push_str(&format!(
        "assertion-compiled-{SYSTEM_ASSERTION}={}\n",
        report
            .assertions
            .names_compiled
            .prevent_user_idle_system_sleep
    ));
    out.push_str(&format!(
        "assertion-compiled-{DISPLAY_ASSERTION}={}\n",
        report
            .assertions
            .names_compiled
            .prevent_user_idle_display_sleep
    ));
    out.push_str(&format!(
        "assertion-compiled-{NO_IDLE}={}\n",
        report.assertions.names_compiled.no_idle_sleep
    ));
    out.push_str(&format!(
        "assertion-compiled-{NO_DISPLAY}={}\n",
        report.assertions.names_compiled.no_display_sleep
    ));
    out.push_str(&format!(
        "assertion-reason={}\n",
        one_line(&report.assertions.reason)
    ));
    out.push_str(&format!(
        "assertion-created={}\n",
        report.assertions.created
    ));
    out.push_str(&format!(
        "assertion-released={}\n",
        report.assertions.released
    ));
    out.push_str(&format!(
        "lid-on-power-assertion-held={}\n",
        report.lid_on_power.assertion_held
    ));
    out.push_str(&format!(
        "lid-external-assertion-held={}\n",
        report.lid_external.assertion_held
    ));
    out.push_str(&format!(
        "power-key-assertion-held={}\n",
        report.power_key.assertion_held
    ));
    out.push_str(&format!(
        "pmset-shows-reason={}\n",
        report.assertions.pmset_shows_reason
    ));
    out.push_str(&format!(
        "pmset-shows-{SYSTEM_ASSERTION}={}\n",
        report.assertions.pmset_shows_system
    ));
    out.push_str(&format!(
        "pmset-shows-{DISPLAY_ASSERTION}={}\n",
        report.assertions.pmset_shows_display
    ));
    out.push_str(&format!(
        "pmset-after-shows-reason={}\n",
        report.assertions.pmset_after_shows_reason
    ));
    if let Some(error) = &report.assertions.error {
        out.push_str(&format!("assertion-error={}\n", one_line(error)));
    }
    out.push_str("pmset-while-held-begin\n");
    out.push_str(&scrub(&filter_pmset(
        &report.assertions.pmset_while_held,
        pid_from_reason(&report.assertions.reason),
        &report.assertions.reason,
    )));
    out.push('\n');
    out.push_str("pmset-while-held-end\n");
    out.push_str(&format!(
        "caffeinate-present={}\n",
        report.caffeinate.present
    ));
    out.push_str(&format!("caffeinate-ran={}\n", report.caffeinate.ran));
    out.push_str(&format!(
        "caffeinate-pmset={}\n",
        report.caffeinate.pmset_showed_caffeinate
    ));
    out.push_str(&format!(
        "timer-overlay-down={}\n",
        fmt_count(report.timer.overlay_down_count)
    ));
    out.push_str(&format!(
        "timer-overlay-up={}\n",
        fmt_count(report.timer.overlay_up_count)
    ));
    out.push_str(&format!("timer-slowed={}\n", report.timer.slowed));
    out.push_str(&format!(
        "timer-sentence={}\n",
        one_line(&report.timer.sentence)
    ));
    out.push_str(&format!(
        "overlay-ordered-front={}\n",
        report.timer.overlay_ordered_front
    ));
    out.push_str(&format!("overlay-opaque={}\n", report.timer.overlay_opaque));
    out.push_str(&format!(
        "timer-expected-ticks={}\n",
        report.timer.expected_ticks
    ));
    out.push_str(&format!(
        "timer-occlusion-visible-down={}\n",
        fmt_flag(report.timer.occlusion_visible_down)
    ));
    out.push_str(&format!(
        "timer-occlusion-visible-up={}\n",
        fmt_flag(report.timer.occlusion_visible_up)
    ));
    out.push_str(&format!("app-nap-ran={}\n", report.timer.app_nap_ran));
    out.push_str(&format!("app-nap-ms={}\n", report.timer.app_nap_ms));
    out.push_str(&format!(
        "app-nap-count={}\n",
        fmt_count(report.timer.app_nap_count)
    ));
    out.push_str(&format!(
        "lid-on-power={}\n",
        row_token(&report.lid_on_power)
    ));
    out.push_str(&format!(
        "lid-external={}\n",
        row_token(&report.lid_external)
    ));
    out.push_str(&format!("power-key={}\n", row_token(&report.power_key)));
    out.push_str(&format!(
        "keep-awake-claim={}\n",
        one_line(&report.keep_awake_claim)
    ));
    out.push_str("blocks=m16\n");
    for item in &report.hardware_only {
        out.push_str(&format!("hardware-only={}\n", one_line(item)));
    }
    out.push_str(&format!("serial-collected={}\n", report.serial_collected));
    out.push_str(&format!("locked-session={}\n", report.locked_session));
    out.push_str(&format!("reversible={}\n", report.reversible));
    out.push_str(&format!("interrupted={}\n", report.interrupted));
    if let Some(crash) = &report.probe_crash {
        out.push_str(&format!("probe-crash={}\n", one_line(crash)));
    }
    let reasons = failure_reasons(report);
    if reasons.is_empty() {
        out.push_str("result=pass\n");
    } else {
        out.push_str("result=fail\n");
        for reason in reasons {
            out.push_str(&format!("fail={}\n", one_line(&reason)));
        }
    }
    out
}

fn fmt_count(count: Option<u64>) -> String {
    match count {
        Some(value) => value.to_string(),
        None => "missing".to_string(),
    }
}

fn fmt_flag(value: Option<bool>) -> String {
    match value {
        Some(true) => "true".to_string(),
        Some(false) => "false".to_string(),
        None => "missing".to_string(),
    }
}

fn pid_from_reason(reason: &str) -> u32 {
    reason
        .rsplit('-')
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn pmset_leaks_other_owners(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "windowserver",
        "addonsd",
        "virtual usb",
        "listed by owning",
        "kernel assertions",
        "com.apple.",
        "/users/",
        ".app/",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn row_token(row: &HardwareRow) -> String {
    row.outcome.clone().unwrap_or_else(|| row.status.clone())
}

#[cfg(test)]
fn passing_runner_fixture() -> KitReport {
    let down = 160;
    let up = 158;
    let (slowed, sentence) = slowed_sentence(down, up);
    let assertions = AssertionReport {
        names_compiled: AssertionNames {
            prevent_user_idle_system_sleep: true,
            prevent_user_idle_display_sleep: true,
            no_idle_sleep: true,
            no_display_sleep: true,
        },
        reason: format!("{REASON_PREFIX}-1"),
        created: true,
        released: true,
        pmset_while_held: format!(
            "{SYSTEM_ASSERTION} 1\n{DISPLAY_ASSERTION} 1\npid 1({REASON_PREFIX}-1): {SYSTEM_ASSERTION} named: \"{REASON_PREFIX}-1\""
        ),
        pmset_after_release: "No assertions.\n".to_string(),
        pmset_shows_reason: true,
        pmset_shows_system: true,
        pmset_shows_display: true,
        pmset_after_shows_reason: false,
        error: None,
    };
    let lid = runner_hardware_row("lid close", true);
    KitReport {
        kit: KIT_ID.to_string(),
        mode: "runner".to_string(),
        version: VersionReport {
            product_name: "macOS".to_string(),
            product_version: "26.7.2".to_string(),
            build_version: "25G332".to_string(),
            uname_m: "arm64".to_string(),
            hw_model: "VirtualMac".to_string(),
        },
        display_count: 1,
        keep_awake_claim: keep_awake_claim(true, &lid),
        assertions,
        caffeinate: CaffeinateReport {
            present: true,
            ran: true,
            pmset_showed_caffeinate: true,
            error: None,
        },
        timer: TimerReport {
            interval_ms: DEFAULT_INTERVAL_MS,
            duration_ms: RUNNER_SAMPLE_MS,
            overlay_down_count: Some(down),
            overlay_up_count: Some(up),
            overlay_ordered_front: true,
            overlay_opaque: true,
            overlay_display_count: 1,
            occlusion_visible_down: Some(true),
            occlusion_visible_up: Some(false),
            expected_ticks: expected_ticks(RUNNER_SAMPLE_MS, DEFAULT_INTERVAL_MS),
            app_nap_ms: APP_NAP_SAMPLE_MS,
            app_nap_count: None,
            app_nap_occlusion_visible: None,
            app_nap_ran: false,
            slowed,
            sentence,
            error: None,
        },
        lid_on_power: lid.clone(),
        lid_external: runner_hardware_row("lid close with an external display", true),
        power_key: runner_hardware_row("power key", true),
        hardware_only: hardware_only_list(),
        serial_collected: false,
        locked_session: false,
        reversible: true,
        interrupted: false,
        probe_crash: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_laptop_lid_row_refuses_no_lid() {
        assert!(laptop_lid_row("no lid").unwrap_err().contains("laptop"));
        assert_eq!(laptop_lid_row("slept").unwrap(), "slept");
        assert_eq!(laptop_lid_row("awake").unwrap(), "awake");
        assert!(lid_external_row("no-lid").is_err());
        assert_eq!(
            lid_external_row("no-external-display").unwrap(),
            "no-external-display"
        );
        assert_eq!(power_key_row("nothing").unwrap(), "nothing");
        assert!(power_key_row("hold").is_err());
        assert!(pmset_shows(
            "PreventUserIdleSystemSleep pid 1",
            SYSTEM_ASSERTION
        ));
    }

    #[test]
    fn covered_process_slowing_uses_an_eighty_percent_bar() {
        let (slowed, sentence) = slowed_sentence(100, 90);
        assert!(!slowed);
        assert!(sentence.contains("did not slow"));
        let (slowed, sentence) = slowed_sentence(100, 50);
        assert!(slowed);
        assert!(sentence.contains("slowed"));
    }

    #[test]
    fn keep_awake_names_m16_and_idle_sleep_only() {
        let lid = runner_hardware_row("lid", true);
        let claim = keep_awake_claim(true, &lid);
        assert!(claim.contains("m16"));
        assert!(claim.contains("idle sleep"));
        assert!(claim.contains("lid-close takeover") || claim.contains("lid shut"));
        let slept = hardware_row(
            "recorded".to_string(),
            Some("slept".to_string()),
            Some(8000),
            true,
            "lid",
        );
        let claim = keep_awake_claim(true, &slept);
        assert!(claim.contains("must not claim agents kept running with the lid shut"));
    }

    #[test]
    fn a_passing_runner_report_prints_the_reason_and_result_pass() {
        let report = passing_runner_fixture();
        assert!(failure_reasons(&report).is_empty());
        let text = render(&report);
        assert!(text.contains(&format!("assertion-reason={REASON_PREFIX}-1")));
        assert!(text.contains("pmset-shows-reason=true"));
        assert!(text.contains("result=pass"));
        assert!(text.contains("blocks=m16"));
        assert!(text.contains("hardware-only=lid-closed-on-power"));
        assert!(text.contains("lid-on-power=not-run-on-runner"));
        assert!(!text.contains("no lid"));
        assert!(!contains_serial(&text));
        assert!(text.contains("step=1"));
        assert!(text.contains("step=7"));
        assert!(text.contains("IOPMAssertionCreateWithName"));
        assert!(text.contains("overlay-opaque=true"));
        assert_eq!(
            report.timer.expected_ticks,
            expected_ticks(RUNNER_SAMPLE_MS, DEFAULT_INTERVAL_MS)
        );
        assert_eq!(report.timer.expected_ticks, 133);
        assert!(text.contains("lid-on-power-assertion-held=true"));
        assert!(!text.contains("WindowServer"));
        let json = serde_json::to_string(&report).expect("json");
        assert!(json.contains(KIT_ID));
        assert!(!contains_serial(&json));
    }

    #[test]
    fn unreleased_assertions_fail_the_job() {
        let mut report = passing_runner_fixture();
        report.assertions.released = false;
        report.reversible = false;
        report.assertions.pmset_after_shows_reason = true;
        let reasons = failure_reasons(&report);
        assert!(reasons.iter().any(|reason| reason.contains("released")));
        assert!(render(&report).contains("result=fail"));
    }

    #[test]
    fn a_serial_in_pmset_fails_the_job() {
        let mut report = passing_runner_fixture();
        report
            .assertions
            .pmset_while_held
            .push_str(&format!("\n{} {}: X\n", "Serial", "Number"));
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("serial"))
        );
    }

    #[test]
    fn intel_uname_is_allowed_on_the_kit_not_the_runner() {
        let mut report = passing_runner_fixture();
        report.version.uname_m = "x86_64".to_string();
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("arm64"))
        );
        report.mode = "kit".to_string();
        assert!(failure_reasons(&report).is_empty());
    }

    #[test]
    fn a_foreign_pmset_line_fails_the_job() {
        let mut report = passing_runner_fixture();
        report
            .assertions
            .pmset_while_held
            .push_str("\npid 161(WindowServer): UserIsActive\n");
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("another process"))
        );
    }
}
