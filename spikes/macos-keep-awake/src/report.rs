//! Kit report. The log is rendered from this, and the exit code follows it.

use std::io::{self, Write};

use serde::{Deserialize, Serialize};

/// `println!` panics on EPIPE after Terminal closes. This does not.
pub fn write_line(mut out: impl Write, line: &str) {
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

pub fn write_stdout(line: &str) {
    write_line(io::stdout(), line);
}

#[cfg(test)]
use crate::args::{APP_NAP_SAMPLE_MS, DEFAULT_INTERVAL_MS, REASON_PREFIX, RUNNER_SAMPLE_MS};
#[cfg(test)]
use crate::drift::{APP_NAP_GAP_THRESHOLD_MS, APP_NAP_THRESHOLD_WHY};
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
    pub uncovered_expected_ticks: u64,
    pub app_nap_ms: u64,
    pub app_nap_count: Option<u64>,
    pub app_nap_occlusion_visible: Option<bool>,
    pub app_nap_ran: bool,
    pub max_gap_ms: Option<u64>,
    pub p95_gap_ms: Option<u64>,
    pub mean_gap_ms: Option<u64>,
    pub max_wake_late_ms: Option<u64>,
    pub app_nap_threshold_ms: u64,
    pub app_nap_verdict: String,
    pub app_nap_reason: String,
    /// `None` means slowing is not claimed (baseline below expected, or counts missing).
    pub slowed: Option<bool>,
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
    pub detection_method: String,
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
    pub no_input: bool,
    pub probe_crash: Option<String>,
}

pub fn laptop_lid_row(outcome: &str) -> Result<String, String> {
    match outcome {
        "slept" | "awake" | "skip" | "not-detected" | "not-run-on-runner" | "needs-terminal"
        | "no-lid" | "lid-unknown" | "on-battery" => Ok(outcome.to_string()),
        other => Err(format!("unknown lid outcome {other}")),
    }
}

pub fn lid_external_row(outcome: &str) -> Result<String, String> {
    match outcome {
        "slept"
        | "awake"
        | "no-external-display"
        | "skip"
        | "not-detected"
        | "not-run-on-runner"
        | "needs-terminal"
        | "no-lid"
        | "lid-unknown"
        | "on-battery"
        | "closed" => Ok(outcome.to_string()),
        other => Err(format!("unknown lid-external outcome {other}")),
    }
}

pub fn power_key_row(outcome: &str) -> Result<String, String> {
    match outcome {
        "slept" | "restart-panel" | "nothing" | "skip" | "not-detected" | "not-run-on-runner"
        | "needs-terminal" | "no-touch-id" => Ok(outcome.to_string()),
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
        detection_method: "not-run-on-runner".to_string(),
    }
}

pub fn hardware_row(
    status: String,
    outcome: Option<String>,
    gap: Option<u64>,
    assertion_held: bool,
    note: &str,
    detection_method: &str,
) -> HardwareRow {
    HardwareRow {
        status,
        outcome,
        heartbeat_gap_ms: gap,
        assertion_held,
        note: note.to_string(),
        detection_method: detection_method.to_string(),
    }
}

pub fn expected_ticks(sample_ms: u64, interval_ms: u64) -> u64 {
    // Each tick: pump(0.01) then sleep(interval_ms). The pump wait is capped at 10 ms.
    let step = interval_ms.saturating_add(10).max(1);
    sample_ms / step
}

pub fn pmset_shows(excerpt: &str, needle: &str) -> bool {
    excerpt.contains(needle)
}

/// Compare tick rates. The uncovered sample may be shorter than the covered one.
pub fn slowed_sentence(
    down: u64,
    down_ms: u64,
    up: u64,
    up_ms: u64,
    interval_ms: u64,
) -> (Option<bool>, String) {
    if down == 0 || down_ms == 0 || up_ms == 0 {
        return (
            None,
            "No overlay-down count was recorded, so slowing is not claimed.".to_string(),
        );
    }
    let expected = expected_ticks(down_ms, interval_ms);
    if expected > 0 && down * 100 < expected * 80 {
        return (
            None,
            format!(
                "not claimed: baseline below expected ({down} uncovered ticks in {down_ms} ms, expected {expected})"
            ),
        );
    }
    let down_rate = down.saturating_mul(1000) / down_ms;
    let up_rate = up.saturating_mul(1000) / up_ms;
    let slowed = up_rate.saturating_mul(100) < down_rate.saturating_mul(80);
    let sentence = if slowed {
        format!(
            "The covered process slowed ({up_rate} ticks/s with the overlay up, {down_rate} ticks/s with it down)."
        )
    } else {
        format!(
            "The covered process did not slow in this sample ({up_rate} ticks/s with the overlay up, {down_rate} ticks/s with it down)."
        )
    };
    (Some(slowed), sentence)
}

pub fn fmt_slowed(slowed: Option<bool>) -> &'static str {
    match slowed {
        Some(true) => "true",
        Some(false) => "false",
        None => "not-claimed",
    }
}

pub fn hardware_row_filled(row: &HardwareRow) -> bool {
    row.status == "recorded"
        && matches!(
            row.outcome.as_deref(),
            Some(
                "slept"
                    | "awake"
                    | "no-external-display"
                    | "restart-panel"
                    | "nothing"
                    | "no-lid"
                    | "lid-unknown"
                    | "on-battery"
                    | "no-touch-id"
                    | "closed"
            )
        )
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
        "App Nap of a covered process on a real idle Intel Mac (timer-tick drift)".to_string(),
    ]
}

pub fn instructions(kit: bool) -> &'static str {
    if kit {
        "This run holds idle-sleep assertions, measures a covered timer (the screen goes black for about 60s), then asks for lid and Touch ID only.\n\
         App Nap and throttling are scored from timer-tick drift in that sample. The lid steps sleep and lock the Mac. Unlock, then press Enter. Type skip to skip it.\n\
         Control-C ends it and exits non-zero. Settings are not changed. Send result.json and result.txt."
    } else {
        "Runner slice: create idle assertions, filtered pmset, optional caffeinate, short overlay-down and overlay-up timers.\n\
         App Nap, lid, and power-key rows stay not-run-on-runner. Drift is recorded but not claimed as App Nap.\n\
         Assertions are released before exit."
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
    if report.mode == "runner"
        && report.version.uname_m != "arm64"
        && report.version.uname_m != "x86_64"
    {
        reasons.push(format!(
            "runner uname -m is {}, expected arm64 or x86_64",
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
    if report.timer.occlusion_visible_down != Some(true) {
        reasons.push("timer-occlusion-visible-down is not true".to_string());
    }
    if report.timer.occlusion_visible_up != Some(false) {
        reasons.push("timer-occlusion-visible-up is not false".to_string());
    }
    if !report.timer.overlay_opaque {
        reasons.push("overlay-opaque is not true".to_string());
    }
    if report.mode == "kit"
        && report.timer.app_nap_ran
        && report.timer.app_nap_occlusion_visible != Some(false)
    {
        reasons.push("app-nap occlusion is not false".to_string());
    }
    if report.mode == "kit"
        && report.timer.app_nap_ran
        && report.timer.app_nap_verdict != "napped"
        && report.timer.app_nap_verdict != "not-napped"
        && report.timer.app_nap_verdict != "not-claimed"
    {
        reasons.push("app-nap verdict is missing".to_string());
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
    reasons
}

pub fn incomplete_reasons(report: &KitReport) -> Vec<String> {
    if report.mode != "kit" {
        return Vec::new();
    }
    let mut reasons = Vec::new();
    if !report.timer.app_nap_ran {
        reasons.push("app-nap-ran=false".to_string());
    }
    if report.timer.app_nap_count.is_none() {
        reasons.push("App Nap count is missing".to_string());
    }
    if report.timer.app_nap_verdict.is_empty() {
        reasons.push("App Nap verdict is missing".to_string());
    }
    if !hardware_row_filled(&report.lid_on_power) {
        reasons.push("lid-on-power row was skipped".to_string());
    }
    if !hardware_row_filled(&report.lid_external) {
        reasons.push("lid-external row was skipped".to_string());
    }
    if !hardware_row_filled(&report.power_key) {
        reasons.push("power-key row was skipped".to_string());
    }
    reasons
}

pub fn exit_nonzero(report: &KitReport) -> bool {
    report.interrupted
        || !failure_reasons(report).is_empty()
        || !incomplete_reasons(report).is_empty()
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
    out.push_str(&format!(
        "timer-slowed={}\n",
        fmt_slowed(report.timer.slowed)
    ));
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
        "timer-uncovered-expected-ticks={}\n",
        report.timer.uncovered_expected_ticks
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
    if report.mode == "runner" {
        out.push_str("app-nap=not-run-on-runner\n");
    } else {
        out.push_str(&format!(
            "app-nap-count={}\n",
            fmt_count(report.timer.app_nap_count)
        ));
        out.push_str(&format!(
            "app-nap-occlusion-visible={}\n",
            fmt_flag(report.timer.app_nap_occlusion_visible)
        ));
        out.push_str(&format!(
            "app-nap-max-gap-ms={}\n",
            fmt_count(report.timer.max_gap_ms)
        ));
        out.push_str(&format!(
            "app-nap-p95-gap-ms={}\n",
            fmt_count(report.timer.p95_gap_ms)
        ));
        out.push_str(&format!(
            "app-nap-mean-gap-ms={}\n",
            fmt_count(report.timer.mean_gap_ms)
        ));
        out.push_str(&format!(
            "app-nap-max-wake-late-ms={}\n",
            fmt_count(report.timer.max_wake_late_ms)
        ));
        out.push_str(&format!(
            "app-nap-threshold-ms={}\n",
            report.timer.app_nap_threshold_ms
        ));
        out.push_str(&format!(
            "app-nap-verdict={}\n",
            one_line(&report.timer.app_nap_verdict)
        ));
        out.push_str(&format!(
            "app-nap-reason={}\n",
            one_line(&report.timer.app_nap_reason)
        ));
    }
    out.push_str(&format!(
        "lid-on-power={}\n",
        row_token(&report.lid_on_power)
    ));
    out.push_str(&format!(
        "lid-on-power-detection={}\n",
        one_line(&report.lid_on_power.detection_method)
    ));
    out.push_str(&format!(
        "lid-external={}\n",
        row_token(&report.lid_external)
    ));
    out.push_str(&format!(
        "lid-external-detection={}\n",
        one_line(&report.lid_external.detection_method)
    ));
    out.push_str(&format!("power-key={}\n", row_token(&report.power_key)));
    out.push_str(&format!(
        "power-key-detection={}\n",
        one_line(&report.power_key.detection_method)
    ));
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
    out.push_str(&format!("no-input={}\n", report.no_input));
    if let Some(crash) = &report.probe_crash {
        out.push_str(&format!("probe-crash={}\n", one_line(crash)));
    }
    let failures = failure_reasons(report);
    let incomplete = incomplete_reasons(report);
    if !failures.is_empty() {
        out.push_str("result=fail\n");
        for reason in failures {
            out.push_str(&format!("fail={}\n", one_line(&reason)));
        }
    } else if !incomplete.is_empty() {
        out.push_str("result=incomplete\n");
        for reason in incomplete {
            out.push_str(&format!("incomplete={}\n", one_line(&reason)));
        }
    } else {
        out.push_str("result=pass\n");
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
    let (slowed, sentence) = slowed_sentence(
        down,
        RUNNER_SAMPLE_MS,
        up,
        RUNNER_SAMPLE_MS,
        DEFAULT_INTERVAL_MS,
    );
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
            uncovered_expected_ticks: expected_ticks(RUNNER_SAMPLE_MS, DEFAULT_INTERVAL_MS),
            app_nap_ms: APP_NAP_SAMPLE_MS,
            app_nap_count: None,
            app_nap_occlusion_visible: None,
            app_nap_ran: false,
            max_gap_ms: Some(60),
            p95_gap_ms: Some(55),
            mean_gap_ms: Some(52),
            max_wake_late_ms: Some(4),
            app_nap_threshold_ms: APP_NAP_GAP_THRESHOLD_MS,
            app_nap_verdict: "not-run-on-runner".to_string(),
            app_nap_reason: format!(
                "sample {} ms is shorter than 50000 ms, so App Nap is not claimed",
                RUNNER_SAMPLE_MS
            ),
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
        no_input: false,
        probe_crash: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_laptop_lid_row_refuses_no_lid() {
        assert!(laptop_lid_row("no lid").is_err());
        assert_eq!(laptop_lid_row("no-lid").unwrap(), "no-lid");
        assert_eq!(laptop_lid_row("lid-unknown").unwrap(), "lid-unknown");
        assert_eq!(laptop_lid_row("slept").unwrap(), "slept");
        assert_eq!(laptop_lid_row("awake").unwrap(), "awake");
        assert_eq!(lid_external_row("no-lid").unwrap(), "no-lid");
        assert_eq!(lid_external_row("lid-unknown").unwrap(), "lid-unknown");
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

    fn recorded(outcome: &str) -> HardwareRow {
        hardware_row(
            "recorded".to_string(),
            Some(outcome.to_string()),
            None,
            true,
            "operator",
            "iokit-power-notification",
        )
    }

    fn passing_kit_fixture() -> KitReport {
        let mut report = passing_runner_fixture();
        report.mode = "kit".to_string();
        report.timer.app_nap_ran = true;
        report.timer.app_nap_count = Some(400);
        report.timer.app_nap_occlusion_visible = Some(false);
        report.timer.app_nap_ms = APP_NAP_SAMPLE_MS;
        report.timer.max_gap_ms = Some(1_400);
        report.timer.p95_gap_ms = Some(80);
        report.timer.mean_gap_ms = Some(70);
        report.timer.max_wake_late_ms = Some(1_200);
        report.timer.app_nap_threshold_ms = APP_NAP_GAP_THRESHOLD_MS;
        report.timer.app_nap_verdict = "napped".to_string();
        report.timer.app_nap_reason = format!(
            "napped: max-gap=1400ms threshold={APP_NAP_GAP_THRESHOLD_MS}ms because {APP_NAP_THRESHOLD_WHY}"
        );
        report.lid_on_power = recorded("slept");
        report.lid_external = recorded("no-external-display");
        report.power_key = recorded("nothing");
        report.keep_awake_claim = keep_awake_claim(true, &report.lid_on_power);
        report
    }

    #[test]
    fn covered_process_slowing_uses_an_eighty_percent_bar() {
        let (slowed, sentence) = slowed_sentence(120, 8_000, 110, 8_000, 50);
        assert_eq!(slowed, Some(false));
        assert!(sentence.contains("did not slow"));
        let (slowed, sentence) = slowed_sentence(120, 8_000, 50, 8_000, 50);
        assert_eq!(slowed, Some(true));
        assert!(sentence.contains("slowed"));
        let (slowed, sentence) = slowed_sentence(127, 8_000, 900, 60_000, 50);
        assert_eq!(slowed, Some(false));
        assert!(sentence.contains("ticks/s"));
        let (slowed, sentence) = slowed_sentence(50, 8_000, 50, 8_000, 50);
        assert_eq!(slowed, None);
        assert_eq!(fmt_slowed(slowed), "not-claimed");
        assert!(sentence.contains("not claimed: baseline below expected"));
        assert!(sentence.contains("50 uncovered ticks in 8000 ms, expected 133"));
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
            "pmset-log",
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
        assert!(text.contains("Runner slice:"));
        assert!(text.contains("not-run-on-runner"));
        assert!(text.contains("IOPMAssertionCreateWithName") || text.contains("idle assertions"));
        assert!(text.contains("overlay-opaque=true"));
        assert!(text.contains("timer-occlusion-visible-down=true"));
        assert!(text.contains("timer-occlusion-visible-up=false"));
        assert!(text.contains("app-nap=not-run-on-runner"));
        assert!(!text.contains("app-nap-count=missing"));
        assert_eq!(
            report.timer.expected_ticks,
            expected_ticks(RUNNER_SAMPLE_MS, DEFAULT_INTERVAL_MS)
        );
        assert_eq!(report.timer.expected_ticks, 133);
        assert_eq!(report.timer.uncovered_expected_ticks, 133);
        assert!(text.contains("timer-uncovered-expected-ticks=133"));
        assert_eq!(expected_ticks(8_000, 50), 133);
        assert_eq!(expected_ticks(60_000, 50), 1_000);
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
    fn intel_uname_is_allowed_on_the_runner() {
        let mut report = passing_runner_fixture();
        report.version.uname_m = "x86_64".to_string();
        assert!(failure_reasons(&report).is_empty());
        report.version.uname_m = "ppc".to_string();
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("arm64 or x86_64"))
        );
        report.version.uname_m = "x86_64".to_string();
        report.mode = "kit".to_string();
        assert!(failure_reasons(&report).is_empty());
        assert!(!incomplete_reasons(&report).is_empty());
    }

    #[test]
    fn pass_requires_printed_occlusion_and_opaque_flags() {
        let mut down = passing_runner_fixture();
        down.timer.occlusion_visible_down = Some(false);
        assert!(
            failure_reasons(&down)
                .iter()
                .any(|reason| reason.contains("timer-occlusion-visible-down"))
        );
        assert!(render(&down).contains("result=fail"));

        let mut missing_down = passing_runner_fixture();
        missing_down.timer.occlusion_visible_down = None;
        assert!(
            failure_reasons(&missing_down)
                .iter()
                .any(|reason| reason.contains("timer-occlusion-visible-down"))
        );

        let mut up = passing_runner_fixture();
        up.timer.occlusion_visible_up = Some(true);
        assert!(
            failure_reasons(&up)
                .iter()
                .any(|reason| reason.contains("timer-occlusion-visible-up"))
        );
        assert!(render(&up).contains("result=fail"));

        let mut missing_up = passing_runner_fixture();
        missing_up.timer.occlusion_visible_up = None;
        assert!(
            failure_reasons(&missing_up)
                .iter()
                .any(|reason| reason.contains("timer-occlusion-visible-up"))
        );

        let mut opaque = passing_runner_fixture();
        opaque.timer.overlay_opaque = false;
        assert!(
            failure_reasons(&opaque)
                .iter()
                .any(|reason| reason.contains("overlay-opaque"))
        );
        assert!(render(&opaque).contains("result=fail"));
    }

    #[test]
    fn kit_is_incomplete_when_app_nap_or_hardware_rows_are_missing() {
        let ok = passing_kit_fixture();
        assert!(failure_reasons(&ok).is_empty());
        assert!(incomplete_reasons(&ok).is_empty());
        assert!(render(&ok).contains("result=pass"));
        assert!(render(&ok).contains("app-nap-count=400"));
        assert!(render(&ok).contains("app-nap-occlusion-visible=false"));
        assert!(render(&ok).contains("app-nap-verdict=napped"));
        assert!(render(&ok).contains("app-nap-threshold-ms=1000"));
        assert!(render(&ok).contains("lid-on-power-detection=iokit-power-notification"));
        assert!(instructions(true).contains("Type skip"));
        assert!(!instructions(true).contains("Return when"));
        assert!(!render(&ok).contains("app-nap=not-run-on-runner"));

        let mut no_nap = passing_kit_fixture();
        no_nap.timer.app_nap_ran = false;
        let text = render(&no_nap);
        assert!(text.contains("result=incomplete"));
        assert!(
            incomplete_reasons(&no_nap)
                .iter()
                .any(|reason| reason.contains("app-nap-ran=false"))
        );

        let mut missing_count = passing_kit_fixture();
        missing_count.timer.app_nap_count = None;
        assert!(
            incomplete_reasons(&missing_count)
                .iter()
                .any(|reason| reason.contains("App Nap count is missing"))
        );
        assert!(render(&missing_count).contains("result=incomplete"));

        let mut lid = passing_kit_fixture();
        lid.lid_on_power = recorded("skip");
        assert!(
            incomplete_reasons(&lid)
                .iter()
                .any(|reason| reason.contains("lid-on-power"))
        );

        let mut external = passing_kit_fixture();
        external.lid_external = runner_hardware_row("lid close with an external display", true);
        assert!(
            incomplete_reasons(&external)
                .iter()
                .any(|reason| reason.contains("lid-external"))
        );

        let mut power = passing_kit_fixture();
        power.power_key = hardware_row(
            "no-input".to_string(),
            Some("no-input".to_string()),
            None,
            true,
            "stdin closed",
            "no-input",
        );
        assert!(
            incomplete_reasons(&power)
                .iter()
                .any(|reason| reason.contains("power-key"))
        );
        assert!(exit_nonzero(&power));
    }

    #[test]
    fn app_nap_occlusion_must_be_false() {
        let mut visible = passing_kit_fixture();
        visible.timer.app_nap_occlusion_visible = Some(true);
        assert!(
            failure_reasons(&visible)
                .iter()
                .any(|reason| reason.contains("app-nap occlusion"))
        );
        assert!(render(&visible).contains("result=fail"));

        let mut missing = passing_kit_fixture();
        missing.timer.app_nap_occlusion_visible = None;
        assert!(
            failure_reasons(&missing)
                .iter()
                .any(|reason| reason.contains("app-nap occlusion"))
        );

        let runner = passing_runner_fixture();
        assert!(
            !failure_reasons(&runner)
                .iter()
                .any(|reason| reason.contains("app-nap occlusion"))
        );
    }

    #[test]
    fn a_closed_stdout_does_not_panic() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "epipe"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "epipe"))
            }
        }
        write_line(Closed, "report after Terminal closed");
    }

    #[test]
    fn a_low_baseline_prints_timer_slowed_not_claimed() {
        let mut report = passing_runner_fixture();
        let (slowed, sentence) = slowed_sentence(73, 8_000, 72, 8_000, 50);
        report.timer.overlay_down_count = Some(73);
        report.timer.overlay_up_count = Some(72);
        report.timer.slowed = slowed;
        report.timer.sentence = sentence;
        let text = render(&report);
        assert!(text.contains("timer-slowed=not-claimed"));
        assert!(text.contains("timer-sentence=not claimed: baseline below expected"));
        assert!(!text.contains("timer-slowed=false"));
    }

    #[test]
    fn closed_stdin_is_no_input_not_interrupted() {
        let mut report = passing_kit_fixture();
        report.no_input = true;
        report.interrupted = false;
        report.lid_on_power = hardware_row(
            "no-input".to_string(),
            Some("no-input".to_string()),
            None,
            true,
            "stdin closed",
            "no-input",
        );
        assert!(
            !failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("interrupted"))
        );
        let text = render(&report);
        assert!(text.contains("no-input=true"));
        assert!(text.contains("interrupted=false"));
        assert!(text.contains("result=incomplete"));
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
