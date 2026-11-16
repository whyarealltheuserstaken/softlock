//! Proof report. The job log is rendered from this, and the exit code follows it.

use serde::{Deserialize, Serialize};

use crate::verdict::{ABSENT_PIXELS, CONTROL_PIXELS, PRESENT_PIXELS};

pub const METHODS: [&str; 3] = ["gdi", "dxgi", "wgc"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookReport {
    pub installed: bool,
    pub key_seen: bool,
    pub key_injected: bool,
    pub key_vk: Option<u32>,
    pub click_seen: bool,
    pub click_injected: bool,
    pub click_message: Option<u32>,
    pub inserted: u32,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShotReport {
    pub method: String,
    pub exclusion: bool,
    pub marker_pixels: u32,
    pub control_pixels: u32,
    pub marker_region_pixels: u32,
    pub marker_region_control_pixels: u32,
    pub adjacent_control_pixels: u32,
    pub verdict: String,
    pub note: Option<String>,
    pub error: Option<String>,
    pub bmp: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: u32,
    pub state: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteractiveReport {
    pub runner_image: String,
    pub session_id: u32,
    pub pid: u32,
    pub window_station: String,
    pub desktop: String,
    pub username: String,
    pub os_product: String,
    pub os_build: String,
    pub installation_type: String,
    pub screen_w: i32,
    pub screen_h: i32,
    pub hook: HookReport,
    pub topmost: bool,
    pub topmost_hit: bool,
    pub adjacent_hit: bool,
    pub topmost_above: String,
    pub shots: Vec<ShotReport>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceReport {
    pub session_id: u32,
    pub pid: u32,
    pub username: String,
    pub console_session_id: u32,
    pub sessions: Vec<SessionInfo>,
    pub token_source: String,
    pub child_pid: Option<u32>,
    pub child_exit_code: Option<u32>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofReport {
    pub runner_image: String,
    pub service_registered: bool,
    pub service_started: bool,
    pub service_stopped: bool,
    pub service_deleted: bool,
    pub service: Option<ServiceReport>,
    pub interactive: Option<InteractiveReport>,
}

pub fn failure_reasons(report: &ProofReport) -> Vec<String> {
    let mut reasons = Vec::new();
    if !report.service_registered {
        reasons.push("service was not registered".to_string());
    }
    if !report.service_started {
        reasons.push("service did not start".to_string());
    }
    if !report.service_stopped {
        reasons.push("service did not stop".to_string());
    }
    if !report.service_deleted {
        reasons.push("service was not deleted".to_string());
    }
    let Some(service) = report.service.as_ref() else {
        reasons.push("service report is missing".to_string());
        return reasons;
    };
    if service.session_id != 0 {
        reasons.push(format!(
            "service session is {}, expected 0",
            service.session_id
        ));
    }
    if let Some(error) = service.error.as_ref() {
        reasons.push(format!("service launch: {error}"));
    }
    let Some(interactive) = report.interactive.as_ref() else {
        reasons.push("interactive report is missing".to_string());
        return reasons;
    };
    if let Some(error) = interactive.error.as_ref() {
        reasons.push(format!("interactive proof: {error}"));
    }
    if interactive.session_id == 0 {
        reasons.push("launched process stayed in session 0".to_string());
    }
    if !interactive.window_station.eq_ignore_ascii_case("WinSta0") {
        reasons.push(format!("window station is {}", interactive.window_station));
    }
    if !interactive.desktop.eq_ignore_ascii_case("Default") {
        reasons.push(format!("desktop is {}", interactive.desktop));
    }
    if !interactive.hook.installed {
        reasons.push("hook was not installed".to_string());
    }
    if !(interactive.hook.key_seen && interactive.hook.key_injected) {
        reasons.push("hook did not see the injected key".to_string());
    }
    if !(interactive.hook.click_seen && interactive.hook.click_injected) {
        reasons.push("hook did not see the injected click".to_string());
    }
    if let Some(error) = interactive.hook.error.as_ref() {
        reasons.push(format!("hook: {error}"));
    }
    if !interactive.topmost {
        reasons.push("marker window was not topmost".to_string());
    }
    if !interactive.topmost_hit {
        reasons.push(
            "WindowFromPoint at the marker centre did not return the marker window".to_string(),
        );
    }
    if !interactive.adjacent_hit {
        reasons.push(
            "WindowFromPoint beside the marker did not return the control window".to_string(),
        );
    }
    for method in METHODS {
        reasons.extend(shot_reason(interactive, method, false));
        reasons.extend(shot_reason(interactive, method, true));
    }
    reasons
}

pub fn render(report: &ProofReport) -> String {
    let mut lines = Vec::new();
    lines.push(format!("runner-image={}", report.runner_image));
    if let Some(interactive) = report.interactive.as_ref() {
        lines.push(format!("os-product={}", interactive.os_product));
        lines.push(format!("os-build={}", interactive.os_build));
        lines.push(format!(
            "installation-type={}",
            interactive.installation_type
        ));
        lines.push(format!(
            "screen={}x{}",
            interactive.screen_w, interactive.screen_h
        ));
        lines.push(format!("child-session={}", interactive.session_id));
        lines.push(format!(
            "child-window-station={}",
            interactive.window_station
        ));
        lines.push(format!("child-desktop={}", interactive.desktop));
        lines.push(format!("child-username={}", interactive.username));
        lines.push(format!("hook-installed={}", interactive.hook.installed));
        lines.push(format!("hook-key-seen={}", interactive.hook.key_seen));
        lines.push(format!(
            "hook-key-injected={}",
            interactive.hook.key_injected
        ));
        lines.push(format!(
            "hook-key-vk={}",
            display_option(interactive.hook.key_vk)
        ));
        lines.push(format!("hook-click-seen={}", interactive.hook.click_seen));
        lines.push(format!(
            "hook-click-injected={}",
            interactive.hook.click_injected
        ));
        lines.push(format!(
            "hook-click-message={}",
            display_option(interactive.hook.click_message)
        ));
        lines.push(format!("hook-inserted={}", interactive.hook.inserted));
        if let Some(error) = interactive.hook.error.as_ref() {
            lines.push(format!("hook-error={error}"));
        }
        lines.push(format!("topmost={}", interactive.topmost));
        lines.push(format!("topmost-hit={}", interactive.topmost_hit));
        lines.push(format!("adjacent-hit={}", interactive.adjacent_hit));
        lines.push(format!("topmost-above={}", interactive.topmost_above));
        for method in METHODS {
            for exclusion in [false, true] {
                lines.push(shot_line(interactive, method, exclusion));
            }
        }
        if let Some(error) = interactive.error.as_ref() {
            lines.push(format!("interactive-error={error}"));
        }
    }
    if let Some(service) = report.service.as_ref() {
        lines.push(format!("service-session={}", service.session_id));
        lines.push(format!("service-username={}", service.username));
        lines.push(format!("console-session={}", service.console_session_id));
        lines.push(format!("token-source={}", service.token_source));
        lines.push(format!("child-pid={}", display_option(service.child_pid)));
        lines.push(format!(
            "child-exit-code={}",
            display_option(service.child_exit_code)
        ));
        for session in &service.sessions {
            lines.push(format!(
                "session={} state={} name={}",
                session.id, session.state, session.name
            ));
        }
        if let Some(error) = service.error.as_ref() {
            lines.push(format!("service-error={error}"));
        }
    }
    lines.push(format!("service-registered={}", report.service_registered));
    lines.push(format!("service-started={}", report.service_started));
    lines.push(format!("service-stopped={}", report.service_stopped));
    lines.push(format!("service-deleted={}", report.service_deleted));
    let reasons = failure_reasons(report);
    if reasons.is_empty() {
        lines.push("result=pass".to_string());
    } else {
        lines.push("result=fail".to_string());
        for reason in reasons {
            lines.push(format!("failure={reason}"));
        }
    }
    lines.join("\n")
}

fn shot_reason(interactive: &InteractiveReport, method: &str, exclusion: bool) -> Option<String> {
    let shot = find_shot(interactive, method, exclusion);
    let phase = if exclusion { "on" } else { "off" };
    let Some(shot) = shot else {
        return Some(format!(
            "{method} capture with exclusion {phase} is missing"
        ));
    };
    if shot.error.is_some() || shot.verdict == "unusable" {
        let detail = shot.error.clone().unwrap_or_else(|| shot.verdict.clone());
        return Some(format!(
            "{method} capture with exclusion {phase} is unusable: {detail}"
        ));
    }
    if !exclusion && shot.verdict != "present" {
        return Some(format!(
            "{method} marker with exclusion off is {}",
            shot.verdict
        ));
    }
    if exclusion && shot.verdict != "present" && shot.verdict != "absent" {
        return Some(format!(
            "{method} marker with exclusion on is {}",
            shot.verdict
        ));
    }
    if shot.adjacent_control_pixels < CONTROL_PIXELS {
        return Some(format!(
            "{method} adjacent control region with exclusion {phase} has {} control pixels",
            shot.adjacent_control_pixels
        ));
    }
    if !exclusion && shot.marker_region_pixels < PRESENT_PIXELS {
        return Some(format!(
            "{method} marker region with exclusion off has {} marker pixels",
            shot.marker_region_pixels
        ));
    }
    None
}

fn shot_line(interactive: &InteractiveReport, method: &str, exclusion: bool) -> String {
    let phase = if exclusion { "on" } else { "off" };
    let Some(shot) = find_shot(interactive, method, exclusion) else {
        return format!("{method}-exclusion-{phase}=missing");
    };
    if let Some(error) = shot.error.as_ref() {
        return format!(
            "{method}-exclusion-{phase}=marker-{} control-{} error={error}",
            word(shot.marker_pixels, PRESENT_PIXELS, ABSENT_PIXELS),
            if shot.control_pixels >= CONTROL_PIXELS {
                "present"
            } else {
                "absent"
            }
        );
    }
    format!(
        "{method}-exclusion-{phase}=marker-{} control-{} marker-pixels={} control-pixels={} marker-region={} adjacent-control={} under-marker-control={}",
        word(shot.marker_pixels, PRESENT_PIXELS, ABSENT_PIXELS),
        if shot.control_pixels >= CONTROL_PIXELS {
            "present"
        } else {
            "absent"
        },
        shot.marker_pixels,
        shot.control_pixels,
        shot.marker_region_pixels,
        shot.adjacent_control_pixels,
        shot.marker_region_control_pixels
    )
}

fn find_shot<'a>(
    interactive: &'a InteractiveReport,
    method: &str,
    exclusion: bool,
) -> Option<&'a ShotReport> {
    interactive
        .shots
        .iter()
        .find(|shot| shot.method == method && shot.exclusion == exclusion)
}

fn word(pixels: u32, present: u32, absent: u32) -> &'static str {
    if pixels >= present {
        "present"
    } else if pixels < absent {
        "absent"
    } else {
        "partial"
    }
}

fn display_option(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string())
}

#[cfg(test)]
pub fn passing_fixture() -> ProofReport {
    let shots = METHODS
        .into_iter()
        .flat_map(|method| {
            [
                shot(method, false, 20_000, 80_000, "present"),
                shot(method, true, 0, 80_000, "absent"),
            ]
        })
        .collect();
    ProofReport {
        runner_image: "windows-2022".to_string(),
        service_registered: true,
        service_started: true,
        service_stopped: true,
        service_deleted: true,
        service: Some(ServiceReport {
            session_id: 0,
            pid: 10,
            username: "SYSTEM".to_string(),
            console_session_id: 1,
            sessions: vec![SessionInfo {
                id: 1,
                state: "active".to_string(),
                name: "Console".to_string(),
            }],
            token_source: "console-user".to_string(),
            child_pid: Some(11),
            child_exit_code: Some(0),
            error: None,
        }),
        interactive: Some(InteractiveReport {
            runner_image: "windows-2022".to_string(),
            session_id: 1,
            pid: 11,
            window_station: "WinSta0".to_string(),
            desktop: "Default".to_string(),
            username: "runneradmin".to_string(),
            os_product: "Windows Server 2022 Datacenter".to_string(),
            os_build: "20348".to_string(),
            installation_type: "Server".to_string(),
            screen_w: 1024,
            screen_h: 768,
            hook: HookReport {
                installed: true,
                key_seen: true,
                key_injected: true,
                key_vk: Some(124),
                click_seen: true,
                click_injected: true,
                click_message: Some(513),
                inserted: 5,
                error: None,
            },
            topmost: true,
            topmost_hit: true,
            adjacent_hit: true,
            topmost_above: "none".to_string(),
            shots,
            error: None,
        }),
    }
}

#[cfg(test)]
fn shot(method: &str, exclusion: bool, marker: u32, control: u32, verdict: &str) -> ShotReport {
    ShotReport {
        method: method.to_string(),
        exclusion,
        marker_pixels: marker,
        control_pixels: control,
        verdict: verdict.to_string(),
        marker_region_pixels: marker,
        marker_region_control_pixels: if exclusion { control } else { 0 },
        adjacent_control_pixels: control,
        note: None,
        error: None,
        bmp: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_proof_passes_and_reports_exclusion_both_ways() {
        let report = passing_fixture();
        assert!(failure_reasons(&report).is_empty());
        let text = render(&report);
        assert!(text.contains("hook-key-seen=true"));
        assert!(text.contains("hook-key-injected=true"));
        assert!(text.contains("hook-click-seen=true"));
        assert!(text.contains("hook-click-injected=true"));
        assert!(text.contains("gdi-exclusion-off=marker-present control-present"));
        assert!(text.contains("gdi-exclusion-on=marker-absent control-present"));
        assert!(text.contains("dxgi-exclusion-on=marker-absent control-present"));
        assert!(text.contains("wgc-exclusion-off=marker-present control-present"));
        assert!(text.contains("topmost-hit=true"));
        assert!(text.contains("adjacent-hit=true"));
        assert!(text.contains("adjacent-control="));
        assert!(text.contains("service-session=0"));
        assert!(text.contains("child-session=1"));
        assert!(text.contains("result=pass"));
    }

    #[test]
    fn exclusion_that_does_not_hide_the_marker_still_completes() {
        let mut report = passing_fixture();
        let interactive = report.interactive.as_mut().expect("interactive");
        for shot in &mut interactive.shots {
            if shot.exclusion {
                shot.marker_pixels = 20_000;
                shot.verdict = "present".to_string();
            }
        }
        assert!(failure_reasons(&report).is_empty());
        let text = render(&report);
        assert!(text.contains("gdi-exclusion-on=marker-present control-present"));
        assert!(text.contains("result=pass"));
    }

    #[test]
    fn missing_hook_click_session_zero_and_blank_capture_fail() {
        let mut report = passing_fixture();
        report.service.as_mut().expect("service").session_id = 2;
        let interactive = report.interactive.as_mut().expect("interactive");
        interactive.session_id = 0;
        interactive.hook.click_seen = false;
        interactive.shots.retain(|shot| shot.method != "dxgi");
        let gdi = interactive
            .shots
            .iter_mut()
            .find(|shot| shot.method == "gdi" && !shot.exclusion)
            .expect("gdi");
        gdi.verdict = "unusable".to_string();
        gdi.control_pixels = 0;
        gdi.error = Some("blank".to_string());
        let reasons = failure_reasons(&report);
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("injected click"))
        );
        assert!(reasons.iter().any(|reason| reason.contains("session 0")));
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("service session"))
        );
        assert!(reasons.iter().any(|reason| reason.contains("dxgi")));
        assert!(reasons.iter().any(|reason| reason.contains("gdi")));
        assert!(render(&report).contains("result=fail"));
    }

    #[test]
    fn global_control_pixels_do_not_excuse_an_empty_adjacent_region() {
        let mut report = passing_fixture();
        let interactive = report.interactive.as_mut().expect("interactive");
        for shot in &mut interactive.shots {
            shot.adjacent_control_pixels = 0;
        }
        let reasons = failure_reasons(&report);
        assert!(reasons.iter().any(|reason| reason.contains("adjacent")));
        assert!(render(&report).contains("result=fail"));
    }
}
