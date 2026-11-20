//! Kit report. The log is rendered from this, and the exit code follows it.

use std::io::{self, Write};

use serde::{Deserialize, Serialize};

use crate::claim::{SHARING_APIS, m14_token, window_removed};
use crate::scrub::contains_serial;
use crate::text::one_line;

/// `println!` panics on EPIPE after Terminal closes. This does not.
pub fn write_line(mut out: impl Write, line: &str) {
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

pub fn write_stdout(line: &str) {
    write_line(io::stdout(), line);
}

pub const KIT_ID: &str = "m06-overlay";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionReport {
    pub product_name: String,
    pub product_version: String,
    pub build_version: String,
    pub uname_m: String,
    pub hw_model: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureSample {
    pub sharing: String,
    pub api: String,
    pub opened: bool,
    pub width: i32,
    pub height: i32,
    pub marker_pixels: i32,
    pub control_pixels: i32,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathClaim {
    pub api: String,
    pub default_marker: i32,
    pub none_marker: i32,
    pub baseline_marker: i32,
    pub control_in_none: i32,
    pub removed: bool,
    pub m14: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OverlayWindow {
    pub ordered_front: bool,
    pub level: String,
    pub collection: String,
    pub ignores_mouse_events: bool,
    pub is_key: bool,
    pub is_main: bool,
    pub shielding_called: bool,
    pub sharing_default: String,
    pub sharing_none: String,
    pub window_area: i32,
    pub display_count: i32,
    pub covered_count: i32,
    pub visible_after_none: bool,
    pub display_image_api: String,
    pub screen_capture_entry: String,
    pub scstream_entry: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClickFocus {
    pub ignores_mouse_events: bool,
    pub overlay_became_key: bool,
    pub target_stayed_key: bool,
    pub posted_click_count: i32,
    pub posted_key_count: i32,
    pub accessibility: bool,
    pub measured_click: String,
    pub measured_focus: String,
    pub typed_click: String,
    pub typed_focus: String,
    pub click_outcome: String,
    pub focus_outcome: String,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PresentationRow {
    pub name: String,
    pub option_bit: String,
    pub applied_while_inactive: bool,
    pub app_was_active: bool,
    pub outcome: String,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperatorRow {
    pub status: String,
    pub outcome: Option<String>,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KitReport {
    pub kit: String,
    pub mode: String,
    pub version: VersionReport,
    pub overlay: OverlayWindow,
    pub samples: Vec<CaptureSample>,
    pub claims: Vec<PathClaim>,
    pub click_focus: ClickFocus,
    pub presentation: Vec<PresentationRow>,
    pub spaces: OperatorRow,
    pub fullscreen: OperatorRow,
    pub hotplug: OperatorRow,
    pub screens_before: i32,
    pub screens_after: i32,
    pub covered_before: i32,
    pub covered_after: i32,
    pub max_screens: i32,
    pub screen_recording: bool,
    pub accessibility: bool,
    pub timed_out: bool,
    pub timeout_ms: u64,
    pub dismissible: bool,
    pub overlay_torn_down: bool,
    pub serial_collected: bool,
    pub locked_session: bool,
    pub interrupted: bool,
    pub no_input: bool,
    pub probe_crash: Option<String>,
}

pub fn runner_operator_row(subject: &str) -> OperatorRow {
    OperatorRow {
        status: "not-run-on-runner".to_string(),
        outcome: None,
        note: format!(
            "{subject} needs a person at the Mac. The GitHub runner cannot switch Spaces, open a full-screen app, or plug in a display."
        ),
    }
}

pub fn operator_row(status: &str, outcome: Option<&str>, note: &str) -> OperatorRow {
    OperatorRow {
        status: status.to_string(),
        outcome: outcome.map(str::to_string),
        note: note.to_string(),
    }
}

pub fn claims_from_samples(samples: &[CaptureSample], window_area: i32) -> Vec<PathClaim> {
    SHARING_APIS
        .iter()
        .map(|api| {
            let default = find_sample(samples, "default", api);
            let none = find_sample(samples, "none", api);
            let baseline = find_sample(samples, "baseline", api);
            let default_marker = default.map(|s| s.marker_pixels).unwrap_or(-1);
            let none_marker = none.map(|s| s.marker_pixels).unwrap_or(-1);
            let baseline_marker = baseline.map(|s| s.marker_pixels).unwrap_or(-1);
            let control_in_none = none.map(|s| s.control_pixels).unwrap_or(-1);
            let removed = window_removed(default_marker, none_marker, baseline_marker, window_area);
            PathClaim {
                api: (*api).to_string(),
                default_marker,
                none_marker,
                baseline_marker,
                control_in_none,
                removed,
                m14: m14_token(removed).to_string(),
            }
        })
        .collect()
}

fn find_sample<'a>(
    samples: &'a [CaptureSample],
    sharing: &str,
    api: &str,
) -> Option<&'a CaptureSample> {
    samples
        .iter()
        .find(|sample| sample.sharing == sharing && sample.api == api)
}

pub fn displays_covered(overlay: &OverlayWindow) -> bool {
    overlay.display_count > 0
        && overlay.covered_count == overlay.display_count
        && overlay.ordered_front
}

pub fn cover_matches_screens(covered: i32, screens: i32) -> bool {
    screens > 0 && covered == screens
}

/// Operator `no-external-display` is only valid when a second screen was never attached.
pub fn reject_no_external_display(max_screens: i32) -> bool {
    max_screens > 1
}

pub fn score_hotplug(
    typed: &str,
    screens_before: i32,
    screens_after: i32,
    covered_after: i32,
    max_screens: i32,
) -> OperatorRow {
    if typed == "no-external-display" && reject_no_external_display(max_screens) {
        return operator_row(
            "fail",
            Some("no-external-display"),
            "no-external-display is invalid because more than one screen was attached",
        );
    }
    if typed == "skip" || typed == "no-input" || typed == "interrupted" {
        return operator_row("recorded", Some(typed), "operator");
    }
    if !cover_matches_screens(covered_after, screens_after) {
        return operator_row(
            "fail",
            Some("miss"),
            &format!("covered {covered_after} of {screens_after} screens after sync"),
        );
    }
    if typed == "no-external-display" {
        return operator_row(
            "recorded",
            Some("no-external-display"),
            &format!(
                "screens-before={screens_before} screens-after={screens_after} max-screens={max_screens}"
            ),
        );
    }
    let outcome = if screens_after > screens_before {
        "covered"
    } else {
        typed
    };
    operator_row(
        "recorded",
        Some(outcome),
        &format!(
            "screens-before={screens_before} screens-after={screens_after} covered-after={covered_after}"
        ),
    )
}

pub fn measured_and_typed_disagree(measured: &str, typed: &str) -> bool {
    if typed.is_empty()
        || typed == "skip"
        || typed == "no-input"
        || typed == "interrupted"
        || measured.is_empty()
        || measured == "not-applicable"
        || measured == "not-run-on-runner"
    {
        return false;
    }
    measured != typed
}

pub fn row_is_failure(outcome: Option<&str>) -> bool {
    matches!(
        outcome,
        Some("miss" | "covered-by-space" | "covered-by-fullscreen" | "blocked" | "moved")
    )
}

#[cfg(test)]
pub fn row_is_success(outcome: Option<&str>) -> bool {
    matches!(
        outcome,
        Some("stayed-up" | "covered" | "fell-through" | "held" | "swallowed")
    )
}

pub const PRESENTATION_RECORDED_NOTE: &str =
    "recorded; Decision 20 remote pass and D6 capture claim stay out of this kit";

pub fn hardware_only_list() -> Vec<&'static str> {
    vec![
        "Space switch",
        "full-screen app",
        "physical click under the overlay",
        "Command-Tab",
        "Force Quit",
        "power key",
        "external display hot-plug",
    ]
}

pub fn instructions(kit: bool) -> &'static str {
    if kit {
        "Magenta covering windows appear on every attached display. Escape or Control-C tears them down and still writes the result.\n\
         Capture runs first. Then switch Spaces, open one full-screen app, click the window under the overlay, try Command-Tab, Force Quit, and the power key, and optionally plug in a display.\n\
         Type skip to skip a row. Type no-external-display if you have no second panel. Esc needs Accessibility; Ctrl-C and the timeout always tear the overlay down. Send result.json and result.txt."
    } else {
        "Runner slice: covering overlay on every attached display, marker counts for default and sharing none, posted click/focus when Accessibility is granted.\n\
         Spaces, full-screen, chords, the power key, and hot-plug stay not-run-on-runner. Control-C, SIGHUP, SIGTERM, panic, and the timeout tear the overlay down. Escape needs Accessibility."
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
    if report.overlay.shielding_called {
        reasons.push("CGShieldingWindowLevel was called".to_string());
    }
    if report.overlay.level != "screenSaver" {
        reasons.push(format!(
            "overlay level is {}",
            one_line(&report.overlay.level)
        ));
    }
    if !report.overlay.collection.contains("canJoinAllSpaces")
        || !report.overlay.collection.contains("fullScreenAuxiliary")
    {
        reasons.push(format!(
            "overlay collection is {}",
            one_line(&report.overlay.collection)
        ));
    }
    if !report.overlay.ignores_mouse_events {
        reasons.push("ignoresMouseEvents was not set".to_string());
    }
    if report.overlay.is_key || report.overlay.is_main {
        reasons.push("the overlay became key or main".to_string());
    }
    if !report.dismissible {
        reasons.push("the overlay is not dismissible".to_string());
    }
    if !report.overlay_torn_down {
        reasons.push("the overlay was not torn down".to_string());
    }
    if let Some(crash) = &report.probe_crash {
        reasons.push(format!("probe crash: {crash}"));
    }
    if contains_serial(&report.version.hw_model) {
        reasons.push("serial text reached the report".to_string());
    }
    if report.timed_out {
        reasons.push("the timeout watchdog tore the overlay down".to_string());
    }
    if report.screen_recording {
        if report.claims.len() != SHARING_APIS.len() {
            reasons.push("marker counts are missing for a capture path".to_string());
        }
        for claim in &report.claims {
            if claim.default_marker < 0 || claim.none_marker < 0 || claim.baseline_marker < 0 {
                reasons.push(format!("{} marker count is missing", claim.api));
            }
        }
    }
    if report
        .claims
        .iter()
        .any(|claim| claim.m14 != "do-not-claim")
    {
        for claim in &report.claims {
            if claim.m14 != "do-not-claim" {
                reasons.push(format!("{} m14 token is {}", claim.api, claim.m14));
            }
        }
    }
    if !displays_covered(&report.overlay) {
        reasons.push(format!(
            "covered {} of {} displays",
            report.overlay.covered_count, report.overlay.display_count
        ));
    }
    if report.screens_after > 0
        && !cover_matches_screens(report.covered_after, report.screens_after)
    {
        reasons.push(format!(
            "after sync covered {} of {} screens",
            report.covered_after, report.screens_after
        ));
    }
    if measured_and_typed_disagree(
        &report.click_focus.measured_click,
        &report.click_focus.typed_click,
    ) {
        reasons.push("measured click disagrees with typed click".to_string());
    }
    if measured_and_typed_disagree(
        &report.click_focus.measured_focus,
        &report.click_focus.typed_focus,
    ) {
        reasons.push("measured focus disagrees with typed focus".to_string());
    }
    push_row_failure(&mut reasons, "spaces", &report.spaces);
    push_row_failure(&mut reasons, "fullscreen", &report.fullscreen);
    push_row_failure(&mut reasons, "hotplug", &report.hotplug);
    if row_is_failure(Some(&report.click_focus.click_outcome))
        && !report.click_focus.typed_click.is_empty()
    {
        reasons.push(format!("click={}", report.click_focus.click_outcome));
    }
    if row_is_failure(Some(&report.click_focus.focus_outcome))
        && !report.click_focus.typed_focus.is_empty()
    {
        reasons.push(format!("focus={}", report.click_focus.focus_outcome));
    }
    reasons
}

fn push_row_failure(reasons: &mut Vec<String>, label: &str, row: &OperatorRow) {
    if row.status == "fail" || row_is_failure(row.outcome.as_deref()) {
        reasons.push(format!(
            "{label}={}",
            row.outcome.as_deref().unwrap_or(&row.status)
        ));
    }
}

pub fn incomplete_reasons(report: &KitReport) -> Vec<String> {
    let mut reasons = Vec::new();
    if !report.screen_recording {
        reasons.push("CGPreflightScreenCaptureAccess is false".to_string());
    }
    if report.mode != "kit" {
        return reasons;
    }
    for (label, row) in [
        ("spaces", &report.spaces),
        ("fullscreen", &report.fullscreen),
        ("hotplug", &report.hotplug),
    ] {
        if !row_filled(row) {
            reasons.push(format!("{label} row was skipped"));
        }
    }
    for row in &report.presentation {
        if !presentation_filled(row) {
            reasons.push(format!("{} row was skipped", row.name));
        }
    }
    if report.click_focus.click_outcome.is_empty() || report.click_focus.focus_outcome.is_empty() {
        reasons.push("click or focus row is missing".to_string());
    }
    reasons
}

fn row_filled(row: &OperatorRow) -> bool {
    matches!(
        row.outcome.as_deref(),
        Some(
            "stayed-up"
                | "covered-by-space"
                | "covered-by-fullscreen"
                | "miss"
                | "covered"
                | "no-external-display"
                | "fell-through"
                | "blocked"
                | "held"
                | "moved"
        )
    )
}

fn presentation_filled(row: &PresentationRow) -> bool {
    matches!(
        row.outcome.as_str(),
        "swallowed" | "presentation-option-worked" | "desktop-reachable"
    )
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
    out.push_str(&format!(
        "overlay-level={}\n",
        one_line(&report.overlay.level)
    ));
    out.push_str(&format!(
        "overlay-collection={}\n",
        one_line(&report.overlay.collection)
    ));
    out.push_str(&format!(
        "ignoresMouseEvents={}\n",
        report.overlay.ignores_mouse_events
    ));
    out.push_str(&format!("overlay-key={}\n", report.overlay.is_key));
    out.push_str(&format!("overlay-main={}\n", report.overlay.is_main));
    out.push_str(&format!(
        "shielding-called={}\n",
        report.overlay.shielding_called
    ));
    out.push_str(&format!(
        "sharing-default={}\n",
        one_line(&report.overlay.sharing_default)
    ));
    out.push_str(&format!(
        "sharing-none={}\n",
        one_line(&report.overlay.sharing_none)
    ));
    out.push_str(&format!("display-count={}\n", report.overlay.display_count));
    out.push_str(&format!("covered-count={}\n", report.overlay.covered_count));
    out.push_str(&format!("screens-before={}\n", report.screens_before));
    out.push_str(&format!("screens-after={}\n", report.screens_after));
    out.push_str(&format!("covered-before={}\n", report.covered_before));
    out.push_str(&format!("covered-after={}\n", report.covered_after));
    out.push_str(&format!("max-screens={}\n", report.max_screens));
    out.push_str(&format!("screen-recording={}\n", report.screen_recording));
    out.push_str(&format!("accessibility={}\n", report.accessibility));
    out.push_str(&format!("timed-out={}\n", report.timed_out));
    out.push_str(&format!("timeout-ms={}\n", report.timeout_ms));
    out.push_str(&format!(
        "displays-covered={}\n",
        displays_covered(&report.overlay)
    ));
    out.push_str(&format!("window-area={}\n", report.overlay.window_area));
    out.push_str(&format!(
        "display-image-api={}\n",
        one_line(&report.overlay.display_image_api)
    ));
    out.push_str(&format!(
        "screen-capture-entry={}\n",
        one_line(&report.overlay.screen_capture_entry)
    ));
    out.push_str(&format!(
        "scstream-entry={}\n",
        one_line(&report.overlay.scstream_entry)
    ));
    for claim in &report.claims {
        out.push_str(&format!(
            "marker-{}-default={}\n",
            claim.api, claim.default_marker
        ));
        out.push_str(&format!(
            "marker-{}-none={}\n",
            claim.api, claim.none_marker
        ));
        out.push_str(&format!(
            "marker-{}-baseline={}\n",
            claim.api, claim.baseline_marker
        ));
        out.push_str(&format!(
            "control-{}-none={}\n",
            claim.api, claim.control_in_none
        ));
        out.push_str(&format!("removed-{}={}\n", claim.api, claim.removed));
        out.push_str(&format!(
            "sharing-m14-{}={} removed={}\n",
            claim.api, claim.m14, claim.removed
        ));
    }
    out.push_str(&format!(
        "click={}\n",
        one_line(&report.click_focus.click_outcome)
    ));
    out.push_str(&format!(
        "focus={}\n",
        one_line(&report.click_focus.focus_outcome)
    ));
    out.push_str(&format!(
        "measured-click={}\n",
        one_line(&report.click_focus.measured_click)
    ));
    out.push_str(&format!(
        "measured-focus={}\n",
        one_line(&report.click_focus.measured_focus)
    ));
    out.push_str(&format!(
        "typed-click={}\n",
        one_line(&report.click_focus.typed_click)
    ));
    out.push_str(&format!(
        "typed-focus={}\n",
        one_line(&report.click_focus.typed_focus)
    ));
    out.push_str(&format!(
        "overlay-became-key={}\n",
        report.click_focus.overlay_became_key
    ));
    out.push_str(&format!(
        "target-stayed-key={}\n",
        report.click_focus.target_stayed_key
    ));
    out.push_str(&format!(
        "posted-click-count={}\n",
        report.click_focus.posted_click_count
    ));
    for row in &report.presentation {
        out.push_str(&format!("{}={}\n", row.name, one_line(&row.outcome)));
        out.push_str(&format!(
            "{}-applied-while-inactive={}\n",
            row.name, row.applied_while_inactive
        ));
        out.push_str(&format!(
            "{}-app-was-active={}\n",
            row.name, row.app_was_active
        ));
        if row.outcome == "desktop-reachable" {
            out.push_str(&format!(
                "{}-detail={}\n",
                row.name, PRESENTATION_RECORDED_NOTE
            ));
        }
    }
    out.push_str(&format!("spaces={}\n", row_token(&report.spaces)));
    out.push_str(&format!("fullscreen={}\n", row_token(&report.fullscreen)));
    out.push_str(&format!("hotplug={}\n", row_token(&report.hotplug)));
    out.push_str(&format!("dismissible={}\n", report.dismissible));
    out.push_str(&format!("overlay-torn-down={}\n", report.overlay_torn_down));
    out.push_str("blocks=m14\n");
    for item in hardware_only_list() {
        out.push_str(&format!("hardware-only={item}\n"));
    }
    out.push_str(&format!("serial-collected={}\n", report.serial_collected));
    out.push_str(&format!("locked-session={}\n", report.locked_session));
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

fn row_token(row: &OperatorRow) -> String {
    if row.status == "fail" {
        format!("fail:{}", row.outcome.as_deref().unwrap_or("fail"))
    } else {
        row.outcome.clone().unwrap_or_else(|| row.status.clone())
    }
}

#[cfg(test)]
fn passing_runner_fixture() -> KitReport {
    let samples = sample_set(76834, 34, 34, 77615);
    let overlay = OverlayWindow {
        ordered_front: true,
        level: "screenSaver".to_string(),
        collection: "canJoinAllSpaces,fullScreenAuxiliary".to_string(),
        ignores_mouse_events: true,
        is_key: false,
        is_main: false,
        shielding_called: false,
        sharing_default: "NSWindowSharingReadOnly".to_string(),
        sharing_none: "NSWindowSharingNone".to_string(),
        window_area: 76800,
        display_count: 1,
        covered_count: 1,
        visible_after_none: true,
        display_image_api: "CGWindowListCreateImage,CGDisplayCreateImage".to_string(),
        screen_capture_entry: crate::claim::SCREEN_CAPTURE_ENTRY.to_string(),
        scstream_entry: crate::claim::SCSTREAM_ENTRY.to_string(),
        error: None,
    };
    let claims = claims_from_samples(&samples, overlay.window_area);
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
        overlay,
        samples,
        claims,
        click_focus: ClickFocus {
            ignores_mouse_events: true,
            overlay_became_key: false,
            target_stayed_key: true,
            posted_click_count: 0,
            posted_key_count: 0,
            accessibility: false,
            measured_click: "not-run-on-runner".to_string(),
            measured_focus: "not-run-on-runner".to_string(),
            typed_click: String::new(),
            typed_focus: String::new(),
            click_outcome: "not-run-on-runner".to_string(),
            focus_outcome: "not-run-on-runner".to_string(),
            note: "runner has no Accessibility grant".to_string(),
        },
        presentation: vec![
            PresentationRow {
                name: "command-tab".to_string(),
                option_bit: "disableProcessSwitching".to_string(),
                applied_while_inactive: false,
                app_was_active: false,
                outcome: "not-run-on-runner".to_string(),
                note: String::new(),
            },
            PresentationRow {
                name: "force-quit".to_string(),
                option_bit: "disableForceQuit".to_string(),
                applied_while_inactive: false,
                app_was_active: false,
                outcome: "not-run-on-runner".to_string(),
                note: String::new(),
            },
            PresentationRow {
                name: "power-key".to_string(),
                option_bit: "disableSessionTermination".to_string(),
                applied_while_inactive: false,
                app_was_active: false,
                outcome: "not-run-on-runner".to_string(),
                note: String::new(),
            },
        ],
        spaces: runner_operator_row("a Space switch"),
        fullscreen: runner_operator_row("a full-screen app"),
        hotplug: runner_operator_row("an external display"),
        screens_before: 1,
        screens_after: 1,
        covered_before: 1,
        covered_after: 1,
        max_screens: 1,
        screen_recording: true,
        accessibility: false,
        timed_out: false,
        timeout_ms: crate::wait::DEFAULT_TIMEOUT_MS,
        dismissible: true,
        overlay_torn_down: true,
        serial_collected: false,
        locked_session: false,
        interrupted: false,
        no_input: false,
        probe_crash: None,
    }
}

#[cfg(test)]
fn sample_set(default: i32, none: i32, baseline: i32, control: i32) -> Vec<CaptureSample> {
    let mut samples = Vec::new();
    for api in SHARING_APIS {
        for (sharing, marker, control_pixels) in [
            ("default", default, 0),
            ("none", none, control),
            ("baseline", baseline, 0),
        ] {
            samples.push(CaptureSample {
                sharing: sharing.to_string(),
                api: api.to_string(),
                opened: true,
                width: 1024,
                height: 768,
                marker_pixels: marker,
                control_pixels,
                error: None,
            });
        }
    }
    samples
}

#[cfg(test)]
fn passing_kit_fixture() -> KitReport {
    let mut report = passing_runner_fixture();
    report.mode = "kit".to_string();
    report.version.uname_m = "x86_64".to_string();
    report.version.hw_model = "MacBookPro16,2".to_string();
    report.click_focus.measured_click = "fell-through".to_string();
    report.click_focus.measured_focus = "held".to_string();
    report.click_focus.typed_click = "fell-through".to_string();
    report.click_focus.typed_focus = "held".to_string();
    report.click_focus.click_outcome = "fell-through".to_string();
    report.click_focus.focus_outcome = "held".to_string();
    report.click_focus.posted_click_count = 1;
    report.click_focus.accessibility = true;
    report.accessibility = true;
    report.spaces = operator_row("recorded", Some("stayed-up"), "operator");
    report.fullscreen = operator_row("recorded", Some("stayed-up"), "operator");
    report.hotplug = operator_row(
        "recorded",
        Some("no-external-display"),
        "no second panel attached",
    );
    for row in &mut report.presentation {
        row.outcome = "desktop-reachable".to_string();
        row.applied_while_inactive = false;
        row.app_was_active = false;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_capture_path_records_marker_counts_for_default_and_none_sharing() {
        let report = passing_runner_fixture();
        assert_eq!(report.claims.len(), 5);
        let text = render(&report);
        for api in SHARING_APIS {
            assert!(text.contains(&format!("marker-{api}-default=76834")));
            assert!(text.contains(&format!("marker-{api}-none=34")));
            assert!(text.contains(&format!("marker-{api}-baseline=34")));
            assert!(text.contains(&format!("sharing-m14-{api}=do-not-claim removed=true")));
        }
        assert!(failure_reasons(&report).is_empty());
        assert!(text.contains("result=pass"));
    }

    #[test]
    fn every_attached_display_is_covered_and_a_hotplug_is_covered_or_a_precise_skip() {
        let report = passing_runner_fixture();
        assert!(displays_covered(&report.overlay));
        assert_eq!(report.overlay.display_count, report.overlay.covered_count);
        assert_eq!(row_token(&report.hotplug), "not-run-on-runner");
        let kit = passing_kit_fixture();
        assert_eq!(kit.hotplug.outcome.as_deref(), Some("no-external-display"));
        let mut miss = passing_runner_fixture();
        miss.overlay.covered_count = 0;
        assert!(!displays_covered(&miss.overlay));
        assert!(
            failure_reasons(&miss)
                .iter()
                .any(|reason| reason.contains("covered"))
        );
    }

    #[test]
    fn a_space_switch_and_a_full_screen_app_keep_the_overlay_or_name_the_cover() {
        let runner = passing_runner_fixture();
        assert_eq!(row_token(&runner.spaces), "not-run-on-runner");
        assert_eq!(row_token(&runner.fullscreen), "not-run-on-runner");
        let kit = passing_kit_fixture();
        assert_eq!(kit.spaces.outcome.as_deref(), Some("stayed-up"));
        assert_eq!(kit.fullscreen.outcome.as_deref(), Some("stayed-up"));
        let mut named = passing_kit_fixture();
        named.fullscreen = operator_row(
            "fail",
            Some("covered-by-fullscreen"),
            "Safari full-screen covered the overlay",
        );
        assert!(
            failure_reasons(&named)
                .iter()
                .any(|reason| reason.contains("covered-by-fullscreen"))
        );
        assert!(render(&named).contains("result=fail"));
        assert!(render(&named).contains("fullscreen=fail:covered-by-fullscreen"));
    }

    #[test]
    fn a_click_falls_through_and_keyboard_focus_does_not_move() {
        let kit = passing_kit_fixture();
        assert_eq!(kit.click_focus.click_outcome, "fell-through");
        assert_eq!(kit.click_focus.focus_outcome, "held");
        assert!(!kit.click_focus.overlay_became_key);
        assert!(kit.click_focus.target_stayed_key);
        assert!(kit.overlay.ignores_mouse_events);
        let text = render(&kit);
        assert!(text.contains("click=fell-through"));
        assert!(text.contains("focus=held"));
        assert!(text.contains("ignoresMouseEvents=true"));
        assert!(text.contains("overlay-key=false"));
    }

    #[test]
    fn command_tab_force_quit_and_power_key_each_have_a_row() {
        let kit = passing_kit_fixture();
        let names: Vec<_> = kit
            .presentation
            .iter()
            .map(|row| row.name.as_str())
            .collect();
        assert_eq!(names, ["command-tab", "force-quit", "power-key"]);
        for row in &kit.presentation {
            assert!(presentation_filled(row));
            assert!(!row.app_was_active);
        }
        let text = render(&kit);
        assert!(text.contains("command-tab=desktop-reachable"));
        assert!(text.contains("force-quit=desktop-reachable"));
        assert!(text.contains("power-key=desktop-reachable"));
        assert!(text.contains("command-tab-app-was-active=false"));
        let runner = passing_runner_fixture();
        assert!(
            runner
                .presentation
                .iter()
                .all(|row| row.outcome == "not-run-on-runner")
        );
    }

    #[test]
    fn the_finding_tells_m14_which_capture_paths_it_may_claim() {
        let report = passing_kit_fixture();
        assert!(
            report
                .claims
                .iter()
                .all(|claim| claim.m14 == "do-not-claim" && claim.removed)
        );
        let text = render(&report);
        assert!(text.contains("blocks=m14"));
        assert!(text.contains("sharing-m14-SCStream=do-not-claim removed=true"));
    }

    #[test]
    fn a_passing_runner_report_prints_result_pass_and_tears_the_overlay_down() {
        let report = passing_runner_fixture();
        let text = render(&report);
        assert!(text.contains("result=pass"));
        assert!(text.contains("dismissible=true"));
        assert!(text.contains("overlay-torn-down=true"));
        assert!(text.contains("shielding-called=false"));
        assert!(text.contains("serial-collected=false"));
        assert!(text.contains("locked-session=false"));
        assert!(!contains_serial(&text));
        assert!(text.contains("Escape") || text.contains("Control-C"));
        let json = serde_json::to_string(&report).expect("json");
        assert!(json.contains(KIT_ID));
        assert!(!contains_serial(&json));
    }

    #[test]
    fn intel_and_arm64_unames_are_allowed() {
        let mut report = passing_runner_fixture();
        report.version.uname_m = "x86_64".to_string();
        assert!(failure_reasons(&report).is_empty());
        report.version.uname_m = "arm64".to_string();
        assert!(failure_reasons(&report).is_empty());
    }

    #[test]
    fn an_undismissed_overlay_fails_the_job() {
        let mut report = passing_runner_fixture();
        report.overlay_torn_down = false;
        report.dismissible = false;
        let reasons = failure_reasons(&report);
        assert!(reasons.iter().any(|reason| reason.contains("torn down")));
        assert!(reasons.iter().any(|reason| reason.contains("dismissible")));
        assert!(render(&report).contains("result=fail"));
    }

    #[test]
    fn closed_stdin_is_no_input_not_interrupted() {
        let mut report = passing_kit_fixture();
        report.no_input = true;
        report.spaces = operator_row("no-input", Some("no-input"), "stdin closed");
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
    fn stayed_up_covered_fell_through_held_and_swallowed_are_passes() {
        for outcome in ["stayed-up", "covered", "fell-through", "held", "swallowed"] {
            assert!(row_is_success(Some(outcome)), "{outcome}");
            assert!(!row_is_failure(Some(outcome)), "{outcome}");
        }
        let kit = passing_kit_fixture();
        assert!(failure_reasons(&kit).is_empty());
        assert!(render(&kit).contains("result=pass"));
    }

    #[test]
    fn miss_covered_by_blocked_and_moved_are_fails() {
        for outcome in [
            "miss",
            "covered-by-space",
            "covered-by-fullscreen",
            "blocked",
            "moved",
        ] {
            assert!(row_is_failure(Some(outcome)), "{outcome}");
            assert!(!row_is_success(Some(outcome)), "{outcome}");
        }
        let mut miss = passing_kit_fixture();
        miss.spaces = operator_row("fail", Some("miss"), "operator");
        assert!(failure_reasons(&miss).iter().any(|r| r.contains("miss")));

        let mut space = passing_kit_fixture();
        space.spaces = operator_row("fail", Some("covered-by-space"), "operator");
        assert!(
            failure_reasons(&space)
                .iter()
                .any(|r| r.contains("covered-by-space"))
        );

        let mut full = passing_kit_fixture();
        full.fullscreen = operator_row("fail", Some("covered-by-fullscreen"), "operator");
        assert!(
            failure_reasons(&full)
                .iter()
                .any(|r| r.contains("covered-by-fullscreen"))
        );

        let mut blocked = passing_kit_fixture();
        blocked.click_focus.typed_click = "blocked".to_string();
        blocked.click_focus.measured_click = "blocked".to_string();
        blocked.click_focus.click_outcome = "blocked".to_string();
        assert!(
            failure_reasons(&blocked)
                .iter()
                .any(|r| r.contains("click=blocked"))
        );

        let mut moved = passing_kit_fixture();
        moved.click_focus.typed_focus = "moved".to_string();
        moved.click_focus.measured_focus = "moved".to_string();
        moved.click_focus.focus_outcome = "moved".to_string();
        assert!(
            failure_reasons(&moved)
                .iter()
                .any(|r| r.contains("focus=moved"))
        );
        assert!(render(&miss).contains("result=fail"));
    }

    #[test]
    fn presentation_records_swallowed_as_pass_and_desktop_reachable_with_d20_d6() {
        let kit = passing_kit_fixture();
        assert!(failure_reasons(&kit).is_empty());
        let text = render(&kit);
        assert!(text.contains("command-tab=desktop-reachable"));
        assert!(text.contains("Decision 20"));
        assert!(text.contains("D6"));
        let mut swallowed = passing_kit_fixture();
        for row in &mut swallowed.presentation {
            row.outcome = "swallowed".to_string();
        }
        assert!(failure_reasons(&swallowed).is_empty());
        assert!(render(&swallowed).contains("result=pass"));
        assert!(render(&swallowed).contains("command-tab=swallowed"));
    }

    #[test]
    fn cover_count_must_match_screen_count_after_sync() {
        assert!(cover_matches_screens(2, 2));
        assert!(!cover_matches_screens(1, 2));
        let mut report = passing_kit_fixture();
        report.screens_after = 2;
        report.covered_after = 1;
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("after sync"))
        );
        assert_eq!(score_hotplug("covered", 1, 2, 1, 2).status, "fail");
        assert_eq!(
            score_hotplug("covered", 1, 2, 2, 2).outcome.as_deref(),
            Some("covered")
        );
    }

    #[test]
    fn no_external_display_is_rejected_when_a_second_screen_was_attached() {
        assert!(reject_no_external_display(2));
        assert!(!reject_no_external_display(1));
        let row = score_hotplug("no-external-display", 2, 2, 2, 2);
        assert_eq!(row.status, "fail");
        let ok = score_hotplug("no-external-display", 1, 1, 1, 1);
        assert_eq!(ok.status, "recorded");
        let mut report = passing_kit_fixture();
        report.max_screens = 2;
        report.hotplug = score_hotplug("no-external-display", 1, 2, 2, 2);
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("no-external-display"))
        );
    }

    #[test]
    fn screen_recording_preflight_never_passes_capture() {
        let mut report = passing_runner_fixture();
        report.screen_recording = false;
        assert!(
            incomplete_reasons(&report)
                .iter()
                .any(|reason| reason.contains("CGPreflightScreenCaptureAccess"))
        );
        assert!(!render(&report).contains("result=pass"));
        assert!(render(&report).contains("result=incomplete"));
    }

    #[test]
    fn measured_and_typed_click_and_focus_must_agree() {
        assert!(measured_and_typed_disagree("fell-through", "blocked"));
        assert!(!measured_and_typed_disagree("fell-through", "fell-through"));
        assert!(!measured_and_typed_disagree("fell-through", ""));
        let mut report = passing_kit_fixture();
        report.click_focus.measured_click = "fell-through".to_string();
        report.click_focus.typed_click = "blocked".to_string();
        report.click_focus.click_outcome = "blocked".to_string();
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("measured click"))
        );
        report.click_focus.typed_click = "fell-through".to_string();
        report.click_focus.click_outcome = "fell-through".to_string();
        report.click_focus.measured_focus = "held".to_string();
        report.click_focus.typed_focus = "moved".to_string();
        report.click_focus.focus_outcome = "moved".to_string();
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("measured focus"))
        );
    }

    #[test]
    fn accessibility_and_display_count_are_on_the_report() {
        let report = passing_kit_fixture();
        let text = render(&report);
        assert!(text.contains("accessibility=true"));
        assert!(text.contains("display-count=1"));
        assert!(text.contains("max-screens=1"));
        assert!(text.contains("timeout-ms="));
    }
}
