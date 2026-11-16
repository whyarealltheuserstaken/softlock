//! Proof report. The job log is rendered from this, and the exit code follows it.

use serde::{Deserialize, Serialize};

use crate::classify::{
    self, POST_SOURCES, dead_callback_reported_in_time, draft_name, expected_state_id,
    fail_closed_end, healthy_callback_within_one_heartbeat, state_name,
};
use crate::labels::{
    AGENT_LABEL, CAPTURE_APIS, DAEMON_LABEL, REQUIRED_TAPS, SCREEN_CAPTURE_ENTRY, SHARING_TYPES,
};
use crate::launchctl::PrintState;
use crate::text::one_line;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionReport {
    pub product_name: String,
    pub product_version: String,
    pub build_version: String,
    pub uname_m: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowReport {
    pub ordered_front: bool,
    pub key_window: bool,
    pub key_received: bool,
    pub post_location: String,
    pub stage: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TapReport {
    pub location: String,
    pub privilege: String,
    /// `listen-only` for the three required taps. `default` is the one active tap.
    pub option: String,
    pub created: bool,
    pub callback_fired: bool,
    pub key_deleted: Option<bool>,
    pub listen_saw: Option<bool>,
    /// Listen-only tap saw keyCode 0 while no Default tap was installed.
    pub control_saw: Option<bool>,
    pub tcc: String,
    pub accessibility_trusted: Option<bool>,
    pub listen_preflight: Option<bool>,
    pub prompt_owner: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureReport {
    pub api: String,
    pub opened: bool,
    pub width: i32,
    pub height: i32,
    pub nonblack_pixels: Option<i32>,
    pub marker_pixels: Option<i32>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobMarker {
    pub pid: u32,
    pub uid: u32,
    pub graphic_session: bool,
    pub window_ordered_front: bool,
    pub window_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct JobObservation {
    pub kind: String,
    pub session_limit: String,
    pub keep_alive: bool,
    pub bootstrap_ok: bool,
    pub bootstrap_error: Option<String>,
    pub print_state: PrintState,
    pub pid_from_print: Option<u32>,
    pub absent_after_bootout: bool,
    pub stderr_tail: String,
    pub marker: Option<JobMarker>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobReport {
    pub kind: String,
    pub label: String,
    pub keep_alive: bool,
    pub session_limit: String,
    pub loaded: bool,
    pub running: bool,
    pub unloaded: bool,
    pub pid: Option<u32>,
    pub uid: Option<u32>,
    pub graphic_session: Option<bool>,
    pub window_ordered_front: Option<bool>,
    pub window_error: Option<String>,
    pub error: Option<String>,
}

/// The screen-saver window m04 puts under both sharing types.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharingWindowReport {
    pub ordered_front: bool,
    pub level: String,
    pub collection: String,
    pub ignores_mouse_events: bool,
    /// True only if the probe called `CGShieldingWindowLevel`. m04 must not.
    pub shielding_called: bool,
    pub sharing_default: String,
    pub sharing_none: String,
    /// Symbol the display-image capture actually resolved on this OS.
    pub display_image_api: String,
    /// ScreenCaptureKit entry point the harness called, with its filter.
    pub screen_capture_entry: String,
    pub error: Option<String>,
}

/// One capture of the marker window at one sharing type.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharingSample {
    pub sharing: String,
    pub api: String,
    pub opened: bool,
    pub width: i32,
    pub height: i32,
    pub marker_pixels: Option<i32>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharingReport {
    pub window: SharingWindowReport,
    pub samples: Vec<SharingSample>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostedObservation {
    pub source: String,
    pub poster: String,
    pub poster_pid: i64,
    pub state_id: i64,
    pub source_pid: i64,
    pub fields_read: bool,
    pub tap_saw: bool,
    pub window_received: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlowObservation {
    pub disable_event: String,
    pub sleep_ms: i64,
    pub disable_wait_ms: i64,
    pub key_reached_during: bool,
    pub key_reached_after: bool,
    pub reenabled: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MarkerObservation {
    pub dead_ms: i64,
    pub dead_misses: u32,
    pub healthy_ms: i64,
    pub healthy_seen: bool,
    pub reenabled: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClassifyReport {
    pub posts: Vec<PostedObservation>,
    pub slow: SlowObservation,
    pub marker: MarkerObservation,
    /// Always `Locked` when a measurement is a guard failure. Never `Unlocked`.
    pub end_state: String,
    /// `created` when every m03 tap was created, `no-tap` when `CGEventTapCreate` returned NULL.
    pub tap_status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofReport {
    pub runner_image: String,
    pub version: VersionReport,
    pub window: Option<WindowReport>,
    pub taps: Vec<TapReport>,
    pub captures: Vec<CaptureReport>,
    pub jobs: Vec<JobReport>,
    pub classify: Option<ClassifyReport>,
    /// m04 marker counts. Absent means the measurement did not run.
    #[serde(default)]
    pub sharing: Option<SharingReport>,
    /// Set when our own probe process crashes or times out. A capability miss stays None.
    pub probe_crash: Option<String>,
}

pub struct TapBits {
    pub created: bool,
    pub callback_fired: bool,
    pub accessibility_trusted: Option<bool>,
    pub listen_preflight: Option<bool>,
    pub prompt: Option<&'static str>,
}

pub enum WindowPlan {
    Prove,
    Skip { error: String },
}

pub fn prompt_token(code: i32) -> Option<&'static str> {
    match code {
        1 => Some("accessibility-prompt"),
        2 => Some("input-monitoring-prompt"),
        3 => Some("screen-recording-prompt"),
        4 => Some("tcc-prompt"),
        _ => None,
    }
}

pub fn job_window_plan(graphic: bool, session_status: i32) -> WindowPlan {
    if session_status != 0 {
        return WindowPlan::Skip {
            error: format!("SessionGetInfo failed ({session_status}); NSWindow was not created"),
        };
    }
    if !graphic {
        return WindowPlan::Skip {
            error: "no graphic session; NSWindow was not created".to_string(),
        };
    }
    WindowPlan::Prove
}

pub fn tap_from_bits(location: &str, privilege: &str, bits: TapBits) -> TapReport {
    let trusted = flag(bits.accessibility_trusted);
    let listen = flag(bits.listen_preflight);
    let (tcc, error) = if bits.created {
        ("created".to_string(), None)
    } else if let Some(token) = bits.prompt {
        (
            "prompt".to_string(),
            Some(format!(
                "CGEventTapCreate returned NULL; TCC prompt the job cannot click ({token}); AXIsProcessTrusted={trusted}; CGPreflightListenEventAccess={listen}"
            )),
        )
    } else {
        (
            "null".to_string(),
            Some(format!(
                "CGEventTapCreate returned NULL; TCC AXIsProcessTrusted={trusted}; CGPreflightListenEventAccess={listen}"
            )),
        )
    };
    TapReport {
        location: location.to_string(),
        privilege: privilege.to_string(),
        option: "listen-only".to_string(),
        created: bits.created,
        callback_fired: bits.callback_fired,
        key_deleted: None,
        listen_saw: None,
        control_saw: None,
        tcc,
        accessibility_trusted: bits.accessibility_trusted,
        listen_preflight: bits.listen_preflight,
        prompt_owner: bits.prompt.map(str::to_string),
        error,
    }
}

pub fn active_tap(
    bits: TapBits,
    key_deleted: Option<bool>,
    listen_saw: Option<bool>,
    control_saw: Option<bool>,
    detail: &str,
) -> TapReport {
    let mut tap = tap_from_bits("kCGSessionEventTap", "user", bits);
    tap.option = "default".to_string();
    tap.key_deleted = key_deleted;
    tap.listen_saw = listen_saw;
    tap.control_saw = control_saw;
    if tap.created {
        let swallowed = key_deleted == Some(true) && listen_saw == Some(false);
        tap.callback_fired = key_deleted == Some(true);
        if !swallowed {
            let why = if detail.is_empty() {
                "kCGEventTapOptionDefault did not swallow keyCode 0".to_string()
            } else {
                one_line(detail)
            };
            tap.error = Some(why);
        }
    }
    tap
}

pub fn window_process_miss(code: i32, stderr: &str) -> WindowReport {
    WindowReport {
        ordered_front: false,
        key_window: false,
        key_received: false,
        post_location: String::new(),
        stage: last_stage(stderr),
        error: Some(format!(
            "NSWindow proof process exited {code}: {}",
            one_line(stderr)
        )),
    }
}

fn last_stage(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("stage="))
        .unwrap_or("none")
        .to_string()
}

pub fn tap_process_miss(location: &str, privilege: &str, code: i32, stderr: &str) -> TapReport {
    TapReport {
        location: location.to_string(),
        privilege: privilege.to_string(),
        option: "listen-only".to_string(),
        created: false,
        callback_fired: false,
        key_deleted: None,
        listen_saw: None,
        control_saw: None,
        tcc: "null".to_string(),
        accessibility_trusted: None,
        listen_preflight: None,
        prompt_owner: None,
        error: Some(format!(
            "CGEventTapCreate process exited {code}; TCC result was not written: {}",
            one_line(stderr)
        )),
    }
}

pub fn tap_sudo_failed(location: &str, detail: &str) -> TapReport {
    TapReport {
        location: location.to_string(),
        privilege: "root".to_string(),
        option: "listen-only".to_string(),
        created: false,
        callback_fired: false,
        key_deleted: None,
        listen_saw: None,
        control_saw: None,
        tcc: "null".to_string(),
        accessibility_trusted: None,
        listen_preflight: None,
        prompt_owner: None,
        error: Some(format!(
            "sudo -n failed before CGEventTapCreate: {}",
            one_line(detail)
        )),
    }
}

pub fn capture_opened(
    api: &str,
    width: i32,
    height: i32,
    nonblack: i32,
    marker: i32,
) -> CaptureReport {
    CaptureReport {
        api: api.to_string(),
        opened: true,
        width,
        height,
        nonblack_pixels: Some(nonblack),
        marker_pixels: Some(marker),
        error: None,
    }
}

pub fn capture_unavailable(api: &str, detail: &str) -> CaptureReport {
    CaptureReport {
        api: api.to_string(),
        opened: false,
        width: 0,
        height: 0,
        nonblack_pixels: None,
        marker_pixels: None,
        error: Some(format!("{api}: {}", one_line(detail))),
    }
}

pub fn capture_detail(base: &str, prompt: Option<&str>) -> String {
    match prompt {
        Some(token) if !token.is_empty() => {
            format!("{base}; prompt the job cannot click ({token})")
        }
        _ => base.to_string(),
    }
}

pub fn job_from_observation(obs: &JobObservation) -> JobReport {
    let label = match obs.kind.as_str() {
        "agent" => AGENT_LABEL.to_string(),
        "daemon" => DAEMON_LABEL.to_string(),
        _ => String::new(),
    };
    let loaded =
        obs.bootstrap_ok && (obs.print_state != PrintState::Absent || obs.marker.is_some());
    let error = if !obs.bootstrap_ok {
        let detail = obs.bootstrap_error.as_deref().unwrap_or("no detail");
        Some(format!("launchctl bootstrap failed: {}", one_line(detail)))
    } else if obs.marker.is_none() {
        let tail = one_line(&obs.stderr_tail);
        if tail.is_empty() {
            Some("job did not write a marker".to_string())
        } else {
            Some(format!("job did not write a marker: {tail}"))
        }
    } else {
        None
    };
    JobReport {
        kind: obs.kind.clone(),
        label,
        keep_alive: obs.keep_alive,
        session_limit: obs.session_limit.clone(),
        loaded,
        running: obs.print_state == PrintState::Running,
        unloaded: obs.absent_after_bootout,
        pid: obs
            .marker
            .as_ref()
            .map(|marker| marker.pid)
            .or(obs.pid_from_print),
        uid: obs.marker.as_ref().map(|marker| marker.uid),
        graphic_session: obs.marker.as_ref().map(|marker| marker.graphic_session),
        window_ordered_front: obs
            .marker
            .as_ref()
            .map(|marker| marker.window_ordered_front),
        window_error: obs
            .marker
            .as_ref()
            .and_then(|marker| marker.window_error.clone()),
        error,
    }
}

pub fn failure_reasons(report: &ProofReport) -> Vec<String> {
    let mut reasons = Vec::new();
    reasons.extend(version_failures(&report.version));
    match report.window.as_ref() {
        None => reasons.push("window proof is missing".to_string()),
        Some(window) => {
            if let Some(reason) = window_failure(window) {
                reasons.push(reason);
            }
        }
    }
    for (location, privilege) in REQUIRED_TAPS {
        match find_tap(report, location, privilege) {
            None => reasons.push(format!("{location} {privilege} tap is missing")),
            Some(tap) => {
                if let Some(reason) = tap_failure(tap) {
                    reasons.push(reason);
                }
            }
        }
    }
    for api in CAPTURE_APIS {
        match find_capture(report, api) {
            None => reasons.push(format!("{api} capture is missing")),
            Some(capture) => {
                if let Some(reason) = capture_failure(capture) {
                    reasons.push(reason);
                }
            }
        }
    }
    for kind in ["agent", "daemon"] {
        match find_job(report, kind) {
            None => reasons.push(format!("{kind} job is missing")),
            Some(job) => reasons.extend(job_failures(job)),
        }
    }
    match find_active(report) {
        None => reasons.push("active kCGEventTapOptionDefault tap is missing".to_string()),
        Some(tap) => {
            if let Some(reason) = active_failure(tap) {
                reasons.push(reason);
            }
        }
    }
    if let Some(crash) = report.probe_crash.as_deref() {
        reasons.push(format!("probe crashed: {crash}"));
    }
    match report.classify.as_ref() {
        None => reasons.push("m03 classify proof is missing".to_string()),
        Some(classify) => reasons.extend(classify_failures(classify)),
    }
    reasons.extend(sharing_failures(report.sharing.as_ref()));
    reasons
}

/// A path m14 may claim: the marker is in the default capture and absent when sharing is `.none`.
/// Zero on both is a miss (the window was not in the image), not a claim.
pub fn m14_may_claim(sample_default: &SharingSample, sample_none: &SharingSample) -> bool {
    sample_default.sharing == "default"
        && sample_none.sharing == "none"
        && sample_default.api == sample_none.api
        && sample_default.opened
        && sample_none.opened
        && sample_default.marker_pixels.is_some_and(|count| count > 0)
        && sample_none.marker_pixels == Some(0)
}

pub fn sharing_sample(sharing: &str, capture: &CaptureReport) -> SharingSample {
    SharingSample {
        sharing: sharing.to_string(),
        api: capture.api.clone(),
        opened: capture.opened,
        width: capture.width,
        height: capture.height,
        marker_pixels: capture.marker_pixels,
        error: capture.error.clone(),
    }
}

fn sharing_failures(sharing: Option<&SharingReport>) -> Vec<String> {
    let Some(sharing) = sharing else {
        return vec!["sharing measurement is missing".to_string()];
    };
    let mut reasons = Vec::new();
    let window = &sharing.window;
    if window.shielding_called {
        reasons.push("CGShieldingWindowLevel was called".to_string());
    }
    if window.level != "screenSaver" {
        reasons.push(format!(
            "sharing window level is {}",
            display_text(&window.level)
        ));
    }
    if !window.collection.contains("canJoinAllSpaces")
        || !window.collection.contains("fullScreenAuxiliary")
    {
        reasons.push(format!(
            "sharing window collection is {}",
            display_text(&window.collection)
        ));
    }
    if !window.ignores_mouse_events {
        reasons.push("sharing window ignoresMouseEvents was not set".to_string());
    }
    if window.sharing_default != "NSWindowSharingReadOnly" {
        reasons.push(format!(
            "sharing type default read back {}",
            display_text(&window.sharing_default)
        ));
    }
    if window.sharing_none != "NSWindowSharingNone" {
        reasons.push(format!(
            "sharing type none read back {}",
            display_text(&window.sharing_none)
        ));
    }
    if window.display_image_api.is_empty() {
        reasons.push("display-image API was not named".to_string());
    }
    if window.screen_capture_entry != SCREEN_CAPTURE_ENTRY {
        reasons.push("ScreenCaptureKit entry was not named".to_string());
    }
    if probe_crashed(window.error.as_deref()) {
        reasons.push("sharing window probe crashed".to_string());
    } else if !window.ordered_front {
        reasons.push("sharing window was not ordered front".to_string());
    }
    for sharing_type in SHARING_TYPES {
        for api in CAPTURE_APIS {
            match sharing
                .samples
                .iter()
                .find(|sample| sample.sharing == sharing_type && sample.api == api)
            {
                None => reasons.push(format!("sharing {sharing_type} {api} is missing")),
                Some(sample) => {
                    if let Some(reason) = sharing_sample_failure(sample) {
                        reasons.push(reason);
                    }
                }
            }
        }
    }
    reasons
}

fn sharing_sample_failure(sample: &SharingSample) -> Option<String> {
    let who = format!("sharing {} {}", sample.sharing, sample.api);
    if probe_crashed(sample.error.as_deref()) {
        return Some(format!("{who} probe crashed"));
    }
    if sample.opened {
        if sample.marker_pixels.is_none() {
            return Some(format!("{who} opened without a marker count"));
        }
        return None;
    }
    match sample.error.as_deref() {
        Some(error) if !error.is_empty() => None,
        _ => Some(format!(
            "{who} recorded neither a marker count nor an error"
        )),
    }
}

pub fn render(report: &ProofReport) -> String {
    let mut lines = Vec::new();
    lines.push("sw_vers".to_string());
    lines.push(format!("ProductName:\t{}", report.version.product_name));
    lines.push(format!(
        "ProductVersion:\t{}",
        report.version.product_version
    ));
    lines.push(format!("BuildVersion:\t{}", report.version.build_version));
    lines.push("uname -m".to_string());
    lines.push(report.version.uname_m.clone());
    lines.push(format!("runner-image={}", report.runner_image));
    if let Some(window) = report.window.as_ref() {
        lines.push(format!("window-ordered-front={}", window.ordered_front));
        lines.push(format!("window-key={}", window.key_window));
        lines.push(format!("window-key-received={}", window.key_received));
        lines.push(format!(
            "window-post-location={}",
            display_text(&window.post_location)
        ));
        lines.push(format!("window-stage={}", display_text(&window.stage)));
        lines.push(format!(
            "window-error={}",
            window.error.as_deref().unwrap_or("none")
        ));
    } else {
        lines.push("window-ordered-front=missing".to_string());
    }
    for (location, privilege) in REQUIRED_TAPS {
        lines.push(tap_line(report, location, privilege));
    }
    lines.push(active_line(report));
    for api in CAPTURE_APIS {
        lines.push(capture_line(report, api));
    }
    lines.extend(sharing_lines(report.sharing.as_ref()));
    for kind in ["agent", "daemon"] {
        lines.extend(job_lines(report, kind));
    }
    if let Some(crash) = report.probe_crash.as_deref() {
        lines.push(format!("probe-crash={crash}"));
    }
    lines.extend(classify_lines(report));
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

fn classify_failures(classify: &ClassifyReport) -> Vec<String> {
    let mut reasons = Vec::new();
    if classify.end_state != fail_closed_end() {
        reasons.push(format!(
            "m03 failure ended {}; fail-closed ends Locked",
            display_text(&classify.end_state)
        ));
    }
    if classify.tap_status == "no-tap" {
        reasons.push("m03=no-tap".to_string());
    }
    for source in POST_SOURCES {
        match classify
            .posts
            .iter()
            .find(|post| post.source == source && post.poster == "second-process")
        {
            None => reasons.push(format!(
                "m03 post {source} from a second process is missing"
            )),
            Some(post) => {
                if probe_crashed(post.error.as_deref()) {
                    reasons.push(format!("m03 post {source} crashed in the probe"));
                } else if !post.fields_read {
                    reasons.push(format!(
                        "m03 post {source} did not record a state id and source pid"
                    ));
                }
            }
        }
    }
    for post in &classify.posts {
        if !post.tap_saw {
            reasons.push(format!(
                "m03 post {} {} tap-saw=false",
                post.source, post.poster
            ));
        }
    }
    if classify.slow.reenabled || classify.marker.reenabled {
        reasons.push("m03 re-enabled the tap and that is not success".to_string());
    }
    if probe_crashed(classify.slow.error.as_deref()) {
        reasons.push("m03 slow callback crashed in the probe".to_string());
    } else if classify.tap_status != "no-tap" && classify.slow.sleep_ms <= 0 {
        reasons.push("m03 slow callback did not run".to_string());
    }
    let known = matches!(
        classify.slow.disable_event.as_str(),
        "kCGEventTapDisabledByTimeout" | "kCGEventTapDisabledByUserInput" | "none"
    );
    if !known {
        reasons.push(format!(
            "m03 disable event {} is not a timeout, user input, or none",
            classify.slow.disable_event
        ));
    }
    if probe_crashed(classify.marker.error.as_deref()) {
        reasons.push("m03 marker check crashed in the probe".to_string());
    } else if classify.tap_status != "no-tap" && !marker_bounds_met(&classify.marker) {
        reasons.push(format!(
            "m03 marker dead-ms={} healthy-ms={} is outside the heartbeat bound",
            classify.marker.dead_ms, classify.marker.healthy_ms
        ));
    }
    reasons
}

fn marker_bounds_met(marker: &MarkerObservation) -> bool {
    marker.dead_ms >= 0
        && marker.healthy_ms >= 0
        && dead_callback_reported_in_time(marker.dead_misses, marker.dead_ms as u64)
        && marker.healthy_seen
        && healthy_callback_within_one_heartbeat(marker.healthy_ms as u64)
}

fn classify_lines(report: &ProofReport) -> Vec<String> {
    let Some(classify) = report.classify.as_ref() else {
        return vec!["m03=missing".to_string()];
    };
    let mut lines = Vec::new();
    for post in &classify.posts {
        let class = classify::classify(post.state_id, post.source_pid);
        let matches = expected_state_id(&post.source) == Some(post.state_id);
        lines.push(format!(
            "post source={} poster={} poster-pid={} state-id={} state-name={} state-matches={} source-pid={} tap-saw={} window-received={} draft={} error={}",
            post.source,
            post.poster,
            post.poster_pid,
            post.state_id,
            state_name(post.state_id),
            matches,
            post.source_pid,
            post.tap_saw,
            post.window_received,
            draft_name(class),
            post.error.as_deref().unwrap_or("none"),
        ));
    }
    lines.push(format!(
        "slow disable-event={} sleep-ms={} disable-wait-ms={} key-reached-during={} key-reached-after={} reenabled={} error={}",
        classify.slow.disable_event,
        classify.slow.sleep_ms,
        classify.slow.disable_wait_ms,
        classify.slow.key_reached_during,
        classify.slow.key_reached_after,
        classify.slow.reenabled,
        classify.slow.error.as_deref().unwrap_or("none"),
    ));
    let dead = classify.marker.dead_ms >= 0
        && dead_callback_reported_in_time(
            classify.marker.dead_misses,
            classify.marker.dead_ms as u64,
        );
    let healthy = classify.marker.healthy_seen
        && classify.marker.healthy_ms >= 0
        && healthy_callback_within_one_heartbeat(classify.marker.healthy_ms as u64);
    lines.push(format!(
        "marker dead-ms={} dead-misses={} healthy-ms={} healthy-seen={} dead-within-1s={} healthy-within-heartbeat={} reenabled={} error={}",
        classify.marker.dead_ms,
        classify.marker.dead_misses,
        classify.marker.healthy_ms,
        classify.marker.healthy_seen,
        dead,
        healthy,
        classify.marker.reenabled,
        classify.marker.error.as_deref().unwrap_or("none"),
    ));
    lines.push(format!("fail-closed={}", classify.end_state));
    if classify.tap_status == "no-tap" {
        lines.push("m03=no-tap".to_string());
    }
    lines
}

fn version_failures(version: &VersionReport) -> Vec<String> {
    let mut reasons = Vec::new();
    if version.product_name.is_empty()
        || version.product_version.is_empty()
        || version.build_version.is_empty()
    {
        reasons.push("sw_vers is missing a field".to_string());
    }
    if version.uname_m != "arm64" {
        reasons.push(format!(
            "uname -m is {}, expected arm64",
            display_text(&version.uname_m)
        ));
    }
    reasons
}

pub fn window_proof_unfinished(error: Option<&str>) -> bool {
    error.is_some_and(|text| text.contains("proof started"))
}

fn probe_crashed(error: Option<&str>) -> bool {
    error.is_some_and(|text| text.contains("process exited") || text.contains("timed out"))
}

fn window_failure(window: &WindowReport) -> Option<String> {
    if probe_crashed(window.error.as_deref()) {
        return Some("window proof crashed in the probe".to_string());
    }
    if window.key_received {
        return None;
    }
    match window.error.as_deref() {
        Some(error) if api_named(error) => None,
        Some(_) => Some("window miss does not name the API".to_string()),
        None => Some("window did not receive a key and recorded no API error".to_string()),
    }
}

fn api_named(error: &str) -> bool {
    [
        "CGEventPost",
        "CGEventCreateKeyboardEvent",
        "CGEventSourceCreate",
        "NSWindow",
        "NSApplication",
    ]
    .iter()
    .any(|needle| error.contains(needle))
}

fn tap_failure(tap: &TapReport) -> Option<String> {
    let who = format!("{} {}", tap.location, tap.privilege);
    if probe_crashed(tap.error.as_deref()) {
        return Some(format!("{who} probe crashed"));
    }
    match tap.tcc.as_str() {
        "created" if tap.created => None,
        "created" => Some(format!("{who} tcc=created but the tap was NULL")),
        "null" if tap.created => Some(format!("{who} tcc=null but the tap was created")),
        "null" => {
            let error = tap.error.as_deref().unwrap_or("");
            let sudo = tap.privilege == "root" && error.contains("sudo");
            let named = error.contains("CGEventTapCreate")
                && (error.contains("TCC")
                    || error.contains("AXIsProcessTrusted")
                    || error.contains("CGPreflightListenEventAccess"));
            if sudo || named {
                None
            } else {
                Some(format!(
                    "{who} NULL tap does not name CGEventTapCreate and the TCC result"
                ))
            }
        }
        "prompt" => {
            let named = tap
                .prompt_owner
                .as_deref()
                .is_some_and(|owner| !owner.is_empty())
                || tap
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("prompt"));
            if named {
                None
            } else {
                Some(format!("{who} prompt was not named"))
            }
        }
        other => Some(format!("{who} tcc={other} is not created, null, or prompt")),
    }
}

fn capture_failure(capture: &CaptureReport) -> Option<String> {
    if probe_crashed(capture.error.as_deref()) {
        return Some(format!("{} probe crashed", capture.api));
    }
    if capture.opened {
        if capture.width <= 0 || capture.height <= 0 {
            return Some(format!("{} opened without a size", capture.api));
        }
        if capture.nonblack_pixels.is_none() || capture.marker_pixels.is_none() {
            return Some(format!("{} opened without a pixel sample", capture.api));
        }
        return None;
    }
    match capture.error.as_deref() {
        Some(error) if !error.is_empty() => None,
        _ => Some(format!(
            "{} capture recorded neither an image nor an error",
            capture.api
        )),
    }
}

fn job_failures(job: &JobReport) -> Vec<String> {
    let mut reasons = Vec::new();
    let expected = match job.kind.as_str() {
        "agent" => AGENT_LABEL,
        "daemon" => DAEMON_LABEL,
        other => {
            reasons.push(format!("unknown job {other}"));
            return reasons;
        }
    };
    if job.label != expected {
        reasons.push(format!("{} label is {}", job.kind, job.label));
    }
    if !job.keep_alive {
        reasons.push(format!("{} KeepAlive was not requested", job.kind));
    }
    if !job.loaded {
        let error = job.error.as_deref().unwrap_or("");
        if !(error.contains("launchctl") || error.contains("plist") || error.contains("sudo")) {
            reasons.push(format!(
                "{} load miss does not name launchctl, plist, or sudo",
                job.kind
            ));
        }
    }
    if job.loaded && job.graphic_session.is_none() {
        reasons.push(format!("{} loaded but wrote no marker", job.kind));
    }
    if window_proof_unfinished(job.window_error.as_deref())
        || probe_crashed(job.window_error.as_deref())
    {
        reasons.push(format!("{} window proof crashed", job.kind));
    }
    if !job.unloaded {
        reasons.push(format!("{} was not unloaded", job.kind));
    }
    reasons
}

fn tap_line(report: &ProofReport, location: &str, privilege: &str) -> String {
    let Some(tap) = find_tap(report, location, privilege) else {
        return format!("tap-{location}-{privilege}=missing");
    };
    format!(
        "tap-{location}-{privilege}={} callback={} tcc={} accessibility={} listen={} prompt={} error={}",
        tap.tcc,
        tap.callback_fired,
        tap.tcc,
        flag(tap.accessibility_trusted),
        flag(tap.listen_preflight),
        tap.prompt_owner.as_deref().unwrap_or("none"),
        tap.error.as_deref().unwrap_or("none")
    )
}

fn active_failure(tap: &TapReport) -> Option<String> {
    if probe_crashed(tap.error.as_deref()) {
        return Some("active tap probe crashed".to_string());
    }
    if tap.option != "default" {
        return Some("active tap is not kCGEventTapOptionDefault".to_string());
    }
    if !tap.created {
        return tap_failure(tap);
    }
    if tap.control_saw != Some(true) {
        return Some("listen-only control did not see keyCode 0".to_string());
    }
    let swallowed = tap.key_deleted == Some(true) && tap.listen_saw == Some(false);
    if swallowed {
        return None;
    }
    let named = tap
        .error
        .as_deref()
        .is_some_and(|error| error.contains("kCGEventTapOptionDefault"));
    if named {
        None
    } else {
        Some("active tap did not prove the posted key was swallowed".to_string())
    }
}

fn sharing_lines(sharing: Option<&SharingReport>) -> Vec<String> {
    let Some(sharing) = sharing else {
        return vec!["sharing-window=missing".to_string()];
    };
    let window = &sharing.window;
    let mut lines = vec![
        format!(
            "sharing-window=level={} collection={} ignoresMouseEvents={} shielding={} ordered-front={} error={}",
            display_text(&window.level),
            display_text(&window.collection),
            window.ignores_mouse_events,
            window.shielding_called,
            window.ordered_front,
            window.error.as_deref().unwrap_or("none")
        ),
        format!(
            "sharing-readback-default={}",
            display_text(&window.sharing_default)
        ),
        format!(
            "sharing-readback-none={}",
            display_text(&window.sharing_none)
        ),
        format!(
            "display-image-api={}",
            display_text(&window.display_image_api)
        ),
    ];
    for sharing_type in SHARING_TYPES {
        for api in CAPTURE_APIS {
            lines.push(sharing_sample_line(sharing, sharing_type, api));
        }
    }
    for api in CAPTURE_APIS {
        let claim = match (
            find_sharing(sharing, "default", api),
            find_sharing(sharing, "none", api),
        ) {
            (Some(sample_default), Some(sample_none))
                if m14_may_claim(sample_default, sample_none) =>
            {
                "claim"
            }
            _ => "do-not-claim",
        };
        lines.push(format!("sharing-m14-{api}={claim}"));
    }
    lines
}

fn sharing_sample_line(sharing: &SharingReport, sharing_type: &str, api: &str) -> String {
    let Some(sample) = find_sharing(sharing, sharing_type, api) else {
        return format!("sharing-{sharing_type}-{api}=missing");
    };
    let entry = if api == "ScreenCaptureKit" {
        format!(" entry={}", sharing.window.screen_capture_entry)
    } else {
        String::new()
    };
    if sample.opened {
        let count = sample
            .marker_pixels
            .map(|count| count.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        format!(
            "sharing-{sharing_type}-{api}=marker pixels={count} width={} height={height}{entry}",
            sample.width,
            height = sample.height
        )
    } else {
        format!(
            "sharing-{sharing_type}-{api}=unavailable error={}{entry}",
            sample.error.as_deref().unwrap_or("none")
        )
    }
}

fn find_sharing<'a>(
    sharing: &'a SharingReport,
    sharing_type: &str,
    api: &str,
) -> Option<&'a SharingSample> {
    sharing
        .samples
        .iter()
        .find(|sample| sample.sharing == sharing_type && sample.api == api)
}

fn capture_line(report: &ProofReport, api: &str) -> String {
    let Some(capture) = find_capture(report, api) else {
        return format!("capture-{api}=missing");
    };
    if capture.opened {
        format!(
            "capture-{api}=opened width={} height={} nonblack={} marker={}",
            capture.width,
            capture.height,
            pixel_flag(capture.nonblack_pixels),
            pixel_flag(capture.marker_pixels)
        )
    } else {
        format!(
            "capture-{api}=unavailable error={}",
            capture.error.as_deref().unwrap_or("none")
        )
    }
}

fn pixel_flag(count: Option<i32>) -> String {
    match count {
        Some(count) if count > 0 => format!("true pixels={count}"),
        Some(count) => format!("false pixels={count}"),
        None => "unknown".to_string(),
    }
}

fn active_line(report: &ProofReport) -> String {
    let Some(tap) = find_active(report) else {
        return "tap-active-kCGSessionEventTap-user=missing".to_string();
    };
    format!(
        "tap-active-kCGSessionEventTap-user={} control={} deleted={} listen-saw={} accessibility={} listen={} prompt={} error={}",
        if tap.created {
            "created"
        } else {
            tap.tcc.as_str()
        },
        flag(tap.control_saw),
        flag(tap.key_deleted),
        flag(tap.listen_saw),
        flag(tap.accessibility_trusted),
        flag(tap.listen_preflight),
        tap.prompt_owner.as_deref().unwrap_or("none"),
        tap.error.as_deref().unwrap_or("none")
    )
}

fn job_lines(report: &ProofReport, kind: &str) -> Vec<String> {
    let Some(job) = find_job(report, kind) else {
        return vec![format!("job-{kind}=missing")];
    };
    vec![
        format!("job-{kind}-label={}", job.label),
        format!("job-{kind}-loaded={}", job.loaded),
        format!("job-{kind}-running={}", job.running),
        format!("job-{kind}-unloaded={}", job.unloaded),
        format!("job-{kind}-keep-alive={}", job.keep_alive),
        format!(
            "job-{kind}-session-limit={}",
            display_text(&job.session_limit)
        ),
        format!("job-{kind}-pid={}", display_option(job.pid)),
        format!("job-{kind}-uid={}", display_option(job.uid)),
        format!("job-{kind}-graphic-session={}", flag(job.graphic_session)),
        format!(
            "job-{kind}-window-ordered-front={}",
            match job.window_ordered_front {
                Some(value) => value.to_string(),
                None => "unknown".to_string(),
            }
        ),
        format!(
            "job-{kind}-window-error={}",
            job.window_error.as_deref().unwrap_or("none")
        ),
        format!(
            "job-{kind}-error={}",
            job.error.as_deref().unwrap_or("none")
        ),
    ]
}

fn find_tap<'a>(report: &'a ProofReport, location: &str, privilege: &str) -> Option<&'a TapReport> {
    report.taps.iter().find(|tap| {
        tap.location == location && tap.privilege == privilege && tap.option != "default"
    })
}

fn find_active(report: &ProofReport) -> Option<&TapReport> {
    report.taps.iter().find(|tap| tap.option == "default")
}

fn find_capture<'a>(report: &'a ProofReport, api: &str) -> Option<&'a CaptureReport> {
    report.captures.iter().find(|capture| capture.api == api)
}

fn find_job<'a>(report: &'a ProofReport, kind: &str) -> Option<&'a JobReport> {
    report.jobs.iter().find(|job| job.kind == kind)
}

fn flag(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    }
}

fn display_text(text: &str) -> &str {
    if text.is_empty() { "none" } else { text }
}

fn display_option(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string())
}

#[cfg(test)]
fn passing_fixture() -> ProofReport {
    ProofReport {
        runner_image: "macos-latest".to_string(),
        version: VersionReport {
            product_name: "macOS".to_string(),
            product_version: "15.5".to_string(),
            build_version: "24F74".to_string(),
            uname_m: "arm64".to_string(),
        },
        window: Some(WindowReport {
            ordered_front: true,
            key_window: true,
            key_received: true,
            post_location: "kCGSessionEventTap".to_string(),
            stage: "key-received-done".to_string(),
            error: None,
        }),
        taps: vec![
            tap_from_bits(
                "kCGSessionEventTap",
                "user",
                TapBits {
                    created: false,
                    callback_fired: false,
                    accessibility_trusted: Some(false),
                    listen_preflight: Some(false),
                    prompt: None,
                },
            ),
            tap_from_bits(
                "kCGHIDEventTap",
                "user",
                TapBits {
                    created: false,
                    callback_fired: false,
                    accessibility_trusted: Some(false),
                    listen_preflight: Some(false),
                    prompt: None,
                },
            ),
            tap_from_bits(
                "kCGHIDEventTap",
                "root",
                TapBits {
                    created: true,
                    callback_fired: true,
                    accessibility_trusted: Some(true),
                    listen_preflight: Some(true),
                    prompt: None,
                },
            ),
            active_tap(
                TapBits {
                    created: true,
                    callback_fired: true,
                    accessibility_trusted: Some(true),
                    listen_preflight: Some(true),
                    prompt: None,
                },
                Some(true),
                Some(false),
                Some(true),
                "",
            ),
        ],
        captures: CAPTURE_APIS
            .into_iter()
            .map(|api| capture_opened(api, 1024, 768, 100, 40))
            .collect(),
        jobs: vec![agent_observation(true), daemon_observation()],
        classify: Some(passing_classify()),
        sharing: Some(measured_sharing(80, 80)),
        probe_crash: None,
    }
}

#[cfg(test)]
fn passing_classify() -> ClassifyReport {
    let posts = [
        ("kCGEventSourceStatePrivate", -1),
        ("kCGEventSourceStateCombinedSessionState", 0),
        ("kCGEventSourceStateHIDSystemState", 1),
    ]
    .into_iter()
    .map(|(source, state_id)| PostedObservation {
        source: source.to_string(),
        poster: "second-process".to_string(),
        poster_pid: 4242,
        state_id,
        source_pid: 4242,
        fields_read: true,
        tap_saw: true,
        window_received: true,
        error: None,
    })
    .collect();
    ClassifyReport {
        posts,
        slow: SlowObservation {
            disable_event: "none".to_string(),
            sleep_ms: 2000,
            disable_wait_ms: 2000,
            key_reached_during: false,
            key_reached_after: true,
            reenabled: false,
            error: None,
        },
        marker: MarkerObservation {
            dead_ms: 750,
            dead_misses: 3,
            healthy_ms: 20,
            healthy_seen: true,
            reenabled: false,
            error: None,
        },
        end_state: fail_closed_end().to_string(),
        tap_status: "created".to_string(),
    }
}

#[cfg(test)]
fn measured_sharing(default_marker: i32, none_marker: i32) -> SharingReport {
    let mut samples = Vec::new();
    for sharing in SHARING_TYPES {
        for api in CAPTURE_APIS {
            let marker = if sharing == "none" {
                none_marker
            } else {
                default_marker
            };
            samples.push(SharingSample {
                sharing: sharing.to_string(),
                api: api.to_string(),
                opened: true,
                width: 1024,
                height: 768,
                marker_pixels: Some(marker),
                error: None,
            });
        }
    }
    SharingReport {
        window: SharingWindowReport {
            ordered_front: true,
            level: "screenSaver".to_string(),
            collection: "canJoinAllSpaces,fullScreenAuxiliary".to_string(),
            ignores_mouse_events: true,
            shielding_called: false,
            sharing_default: "NSWindowSharingReadOnly".to_string(),
            sharing_none: "NSWindowSharingNone".to_string(),
            display_image_api: "CGWindowListCreateImage".to_string(),
            screen_capture_entry: SCREEN_CAPTURE_ENTRY.to_string(),
            error: None,
        },
        samples,
    }
}

#[cfg(test)]
fn agent_observation(marker: bool) -> JobReport {
    job_from_observation(&JobObservation {
        kind: "agent".to_string(),
        session_limit: "Aqua".to_string(),
        keep_alive: true,
        bootstrap_ok: true,
        bootstrap_error: None,
        print_state: PrintState::Running,
        pid_from_print: Some(42),
        absent_after_bootout: true,
        stderr_tail: String::new(),
        marker: marker.then_some(JobMarker {
            pid: 42,
            uid: 501,
            graphic_session: true,
            window_ordered_front: true,
            window_error: None,
        }),
    })
}

#[cfg(test)]
fn daemon_observation() -> JobReport {
    job_from_observation(&JobObservation {
        kind: "daemon".to_string(),
        session_limit: "none".to_string(),
        keep_alive: true,
        bootstrap_ok: true,
        bootstrap_error: None,
        print_state: PrintState::Running,
        pid_from_print: Some(1),
        absent_after_bootout: true,
        stderr_tail: String::new(),
        marker: Some(JobMarker {
            pid: 1,
            uid: 0,
            graphic_session: false,
            window_ordered_front: false,
            window_error: Some("no graphic session; NSWindow was not created".to_string()),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_proof_passes_and_prints_sw_vers_and_uname() {
        let report = passing_fixture();
        assert!(failure_reasons(&report).is_empty());
        let text = render(&report);
        assert!(text.contains("sw_vers\n"));
        assert!(text.contains("ProductName:\tmacOS"));
        assert!(text.contains("ProductVersion:\t15.5"));
        assert!(text.contains("BuildVersion:\t24F74"));
        assert!(text.contains("uname -m\narm64"));
        assert!(text.contains(
            "tap-kCGSessionEventTap-user=null callback=false tcc=null accessibility=false listen=false prompt=none error=CGEventTapCreate returned NULL; TCC AXIsProcessTrusted=false; CGPreflightListenEventAccess=false"
        ));
        assert!(text.contains(
            "tap-kCGHIDEventTap-root=created callback=true tcc=created accessibility=true listen=true prompt=none error=none"
        ));
        assert!(text.contains("capture-CGWindowListCreateImage=opened width=1024 height=768"));
        assert!(text.contains("capture-ScreenCaptureKit=opened width=1024 height=768"));
        assert!(text.contains("window-ordered-front=true"));
        assert!(text.contains("window-key-received=true"));
        assert!(text.contains("job-agent-unloaded=true"));
        assert!(text.contains("job-daemon-unloaded=true"));
        assert!(text.contains("job-agent-keep-alive=true"));
        assert!(text.contains("job-daemon-keep-alive=true"));
        assert!(text.contains("job-agent-graphic-session=true"));
        assert!(text.contains("job-daemon-graphic-session=false"));
        assert!(
            text.contains("job-daemon-window-error=no graphic session; NSWindow was not created")
        );
        assert!(text.contains("state-id=-1"));
        assert!(text.contains("source-pid=4242"));
        assert!(text.contains("window-received=true"));
        assert!(text.contains("disable-event=none"));
        assert!(text.contains("dead-within-1s=true"));
        assert!(text.contains("healthy-within-heartbeat=true"));
        assert!(text.contains("fail-closed=Locked"));
        assert!(text.contains("result=pass"));
        assert!(text.contains("display-image-api=CGWindowListCreateImage"));
        assert!(text.contains("sharing-default-screencapture=marker pixels=80"));
        assert!(text.contains("sharing-none-screencapture=marker pixels=80"));
        assert!(text.contains(
            "sharing-none-ScreenCaptureKit=marker pixels=80 width=1024 height=768 entry=SCScreenshotManager excludingWindows empty"
        ));
        assert!(text.contains("sharing-m14-ScreenCaptureKit=do-not-claim"));
        assert!(text.contains("ignoresMouseEvents=true"));
        assert!(text.contains("shielding=false"));
        assert!(!text.contains("Serial"));
        let back: ProofReport =
            serde_json::from_str(&serde_json::to_string(&report).unwrap()).expect("json");
        assert_eq!(report, back);
    }

    #[test]
    fn a_missing_tap_emits_m03_no_tap_and_fails_the_run() {
        let mut classify = passing_classify();
        classify.tap_status = "no-tap".to_string();
        classify.slow.sleep_ms = 0;
        classify.marker.dead_ms = 0;
        classify.marker.healthy_seen = false;
        let reasons = classify_failures(&classify);
        assert!(reasons.iter().any(|reason| reason == "m03=no-tap"));
        assert!(
            reasons
                .iter()
                .all(|reason| !reason.contains("slow callback"))
        );
        assert!(reasons.iter().all(|reason| !reason.contains("marker")));
        let mut report = passing_fixture();
        report.classify = Some(classify);
        assert!(render(&report).contains("m03=no-tap"));
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason == "m03=no-tap")
        );
    }

    #[test]
    fn a_second_process_post_the_tap_did_not_see_fails() {
        let mut classify = passing_classify();
        classify.posts[0].tap_saw = false;
        let reasons = classify_failures(&classify);
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("tap-saw=false"))
        );
    }

    #[test]
    fn a_same_process_post_the_tap_did_not_see_fails() {
        let mut classify = passing_classify();
        classify.posts.push(PostedObservation {
            source: "kCGEventSourceStatePrivate".to_string(),
            poster: "same-process".to_string(),
            poster_pid: 7,
            state_id: -1,
            source_pid: 7,
            fields_read: true,
            tap_saw: false,
            window_received: false,
            error: None,
        });
        let reasons = classify_failures(&classify);
        assert!(
            reasons.iter().any(|reason| {
                reason.contains("same-process") && reason.contains("tap-saw=false")
            })
        );
    }

    #[test]
    fn a_reenabled_tap_or_an_unlocked_end_fails_closed() {
        let mut unlocked = passing_fixture();
        unlocked.classify.as_mut().unwrap().end_state = "Unlocked".to_string();
        assert!(
            failure_reasons(&unlocked)
                .iter()
                .any(|reason| reason.contains("Locked"))
        );
        let mut reenabled = passing_fixture();
        reenabled.classify.as_mut().unwrap().slow.reenabled = true;
        assert!(
            failure_reasons(&reenabled)
                .iter()
                .any(|reason| reason.contains("re-enabled"))
        );
        let mut early = passing_fixture();
        early.classify.as_mut().unwrap().marker.dead_misses = 1;
        early.classify.as_mut().unwrap().marker.dead_ms = 100;
        assert!(
            failure_reasons(&early)
                .iter()
                .any(|reason| reason.contains("heartbeat"))
        );
    }

    #[test]
    fn a_null_session_tap_with_a_tcc_denial_is_a_recorded_miss() {
        let mut report = passing_fixture();
        report.taps[0] = tap_from_bits(
            "kCGSessionEventTap",
            "user",
            TapBits {
                created: false,
                callback_fired: false,
                accessibility_trusted: Some(false),
                listen_preflight: None,
                prompt: None,
            },
        );
        assert!(failure_reasons(&report).is_empty());
        assert!(render(&report).contains("CGPreflightListenEventAccess=unknown"));
    }

    #[test]
    fn a_prompt_the_job_cannot_click_is_recorded() {
        let mut report = passing_fixture();
        report.taps[0] = tap_from_bits(
            "kCGSessionEventTap",
            "user",
            TapBits {
                created: false,
                callback_fired: false,
                accessibility_trusted: Some(false),
                listen_preflight: Some(false),
                prompt: prompt_token(1),
            },
        );
        assert_eq!(prompt_token(2), Some("input-monitoring-prompt"));
        assert_eq!(prompt_token(0), None);
        assert!(failure_reasons(&report).is_empty());
        let text = render(&report);
        assert!(text.contains("tcc=prompt"));
        assert!(text.contains("cannot click (accessibility-prompt)"));
    }

    #[test]
    fn a_window_miss_names_the_api_or_fails() {
        let mut report = passing_fixture();
        {
            let window = report.window.as_mut().expect("window");
            window.ordered_front = false;
            window.key_received = false;
            window.post_location.clear();
            window.error = Some(
                "CGEventPost did not deliver keyCode 0 (tried kCGSessionEventTap, kCGAnnotatedSessionEventTap, kCGHIDEventTap)"
                    .to_string(),
            );
        }
        assert!(failure_reasons(&report).is_empty());
        assert!(render(&report).contains("window-error=CGEventPost"));

        {
            let window = report.window.as_mut().expect("window");
            window.ordered_front = false;
            window.key_window = true;
            window.key_received = true;
            window.error = None;
        }
        assert!(failure_reasons(&report).is_empty());

        {
            let window = report.window.as_mut().expect("window");
            window.key_received = false;
            window.ordered_front = false;
            window.error = Some(
                "CGEventPost reaches the session event stream (taps saw keyCode 0); the window was not key (activation did not make it key), so the app did not receive it"
                    .to_string(),
            );
        }
        assert!(failure_reasons(&report).is_empty());

        report.window.as_mut().expect("window").error = Some("nope".to_string());
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("does not name the API"))
        );

        report.window.as_mut().expect("window").error = None;
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("no API error"))
        );

        report.window = Some(window_process_miss(139, "stage=order-front\n"));
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("crashed"))
        );
        assert!(render(&report).contains("window-stage=order-front"));
        report.taps[0] =
            tap_process_miss("kCGSessionEventTap", "user", 139, "stage=CGEventTapCreate");
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("probe crashed"))
        );
    }

    #[test]
    fn a_probe_crash_fails_and_an_active_miss_must_name_the_option() {
        let mut report = passing_fixture();
        report.probe_crash = Some("desktop probe process exited 139: stage=order-out".to_string());
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("probe crashed"))
        );
        assert!(render(&report).contains("result=fail"));

        report.probe_crash = None;
        {
            let active = report
                .taps
                .iter_mut()
                .find(|tap| tap.option == "default")
                .unwrap();
            active.key_deleted = Some(false);
            active.listen_saw = Some(true);
            active.error = Some(
                "kCGEventTapOptionDefault returned NULL for keyCode 0 but the listen-only tap still observed it"
                    .to_string(),
            );
        }
        assert!(failure_reasons(&report).is_empty());
        report
            .taps
            .iter_mut()
            .find(|tap| tap.option == "default")
            .unwrap()
            .control_saw = Some(false);
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("listen-only control"))
        );
        report
            .taps
            .iter_mut()
            .find(|tap| tap.option == "default")
            .unwrap()
            .control_saw = Some(true);

        assert!(render(&report).contains("deleted=false"));
        assert!(render(&report).contains("listen-saw=true"));

        report
            .taps
            .iter_mut()
            .find(|tap| tap.option == "default")
            .unwrap()
            .error = Some("nope".to_string());
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("swallowed"))
        );
    }

    #[test]
    fn an_opened_image_without_pixels_fails_and_a_black_image_is_recorded() {
        let mut report = passing_fixture();
        report.captures[0].nonblack_pixels = None;
        report.captures[0].marker_pixels = None;
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("pixel sample"))
        );

        report.captures[0] = capture_opened("CGWindowListCreateImage", 1024, 768, 0, 0);
        assert!(failure_reasons(&report).is_empty());
        let text = render(&report);
        assert!(text.contains("nonblack=false pixels=0"));
        assert!(text.contains("marker=false pixels=0"));
        assert!(text.contains("tap-active-kCGSessionEventTap-user=created"));
        assert!(text.contains("control=true"));
        assert!(text.contains("deleted=true"));
        assert!(text.contains("listen-saw=false"));
    }

    #[test]
    fn a_null_tap_without_the_tcc_result_fails() {
        let mut report = passing_fixture();
        report.taps[0].error = Some("nope".to_string());
        let reasons = failure_reasons(&report);
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("CGEventTapCreate"))
        );
        assert!(render(&report).contains("result=fail"));
    }

    #[test]
    fn a_root_sudo_miss_is_recorded_and_an_intel_uname_fails() {
        let mut report = passing_fixture();
        report.taps[2] = tap_sudo_failed("kCGHIDEventTap", "a password is required\n");
        assert!(failure_reasons(&report).is_empty());
        assert!(
            render(&report)
                .contains("sudo -n failed before CGEventTapCreate: a password is required")
        );

        report.version.uname_m = "x86_64".to_string();
        let reasons = failure_reasons(&report);
        assert!(reasons.iter().any(|reason| reason.contains("arm64")));
        assert!(render(&report).contains("result=fail"));
    }

    #[test]
    fn jobs_must_unload_and_a_loaded_job_must_write_a_marker() {
        let mut report = passing_fixture();
        report.jobs[0].unloaded = false;
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("agent was not unloaded"))
        );

        report.jobs[0] = agent_observation(false);
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("wrote no marker"))
        );

        report.jobs[0].loaded = false;
        report.jobs[0].error = Some("nope".to_string());
        report.jobs[0].graphic_session = None;
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("launchctl"))
        );
    }

    #[test]
    fn keep_alive_is_required_and_a_launchctl_load_miss_is_enough() {
        let mut report = passing_fixture();
        report.jobs[1].keep_alive = false;
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("KeepAlive"))
        );

        report = passing_fixture();
        report.jobs[1] = job_from_observation(&JobObservation {
            kind: "daemon".to_string(),
            session_limit: "none".to_string(),
            keep_alive: true,
            bootstrap_ok: false,
            bootstrap_error: Some("sudo: a password is required".to_string()),
            print_state: PrintState::Absent,
            pid_from_print: None,
            absent_after_bootout: true,
            stderr_tail: String::new(),
            marker: None,
        });
        assert!(failure_reasons(&report).is_empty());
        assert!(render(&report).contains("job-daemon-loaded=false"));
        assert!(render(&report).contains("launchctl bootstrap failed"));
    }

    #[test]
    fn capture_must_open_with_a_size_or_name_the_error() {
        let mut report = passing_fixture();
        report
            .captures
            .retain(|capture| capture.api != "screencapture");
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("screencapture capture is missing"))
        );

        report = passing_fixture();
        report.captures[0].width = 0;
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("without a size"))
        );

        report.captures[0] = capture_unavailable(
            "CGWindowListCreateImage",
            &capture_detail("returned NULL", Some("screen-recording-prompt")),
        );
        assert!(failure_reasons(&report).is_empty());
        assert!(
            render(&report).contains(
                "capture-CGWindowListCreateImage=unavailable error=CGWindowListCreateImage: returned NULL; prompt the job cannot click (screen-recording-prompt)"
            )
        );
    }

    #[test]
    fn a_daemon_without_a_graphic_session_does_not_create_a_window() {
        match job_window_plan(false, 0) {
            WindowPlan::Skip { error } => {
                assert!(error.contains("NSWindow"));
                assert!(error.contains("no graphic session"));
            }
            WindowPlan::Prove => panic!("a session without graphics must not call AppKit"),
        }
        match job_window_plan(true, -600) {
            WindowPlan::Skip { error } => assert!(error.contains("SessionGetInfo failed (-600)")),
            WindowPlan::Prove => panic!("a failed session query must not call AppKit"),
        }
        assert!(matches!(job_window_plan(true, 0), WindowPlan::Prove));
    }

    #[test]
    fn sharing_none_omits_the_marker_only_when_the_default_capture_shows_it() {
        let shown = measured_sharing(40, 0);
        let hidden = shown
            .samples
            .iter()
            .find(|sample| sample.sharing == "none" && sample.api == "screencapture")
            .unwrap();
        let visible = shown
            .samples
            .iter()
            .find(|sample| sample.sharing == "default" && sample.api == "screencapture")
            .unwrap();
        assert!(m14_may_claim(visible, hidden));

        let still_there = measured_sharing(40, 12);
        let none = still_there
            .samples
            .iter()
            .find(|sample| sample.sharing == "none" && sample.api == "ScreenCaptureKit")
            .unwrap();
        let default = still_there
            .samples
            .iter()
            .find(|sample| sample.sharing == "default" && sample.api == "ScreenCaptureKit")
            .unwrap();
        assert!(!m14_may_claim(default, none));

        let both_zero = measured_sharing(0, 0);
        let none = both_zero
            .samples
            .iter()
            .find(|sample| sample.sharing == "none" && sample.api == "CGDisplayCreateImage")
            .unwrap();
        let default = both_zero
            .samples
            .iter()
            .find(|sample| sample.sharing == "default" && sample.api == "CGDisplayCreateImage")
            .unwrap();
        assert!(!m14_may_claim(default, none));

        let mut report = passing_fixture();
        report.sharing = Some(shown);
        assert!(failure_reasons(&report).is_empty());
        let text = render(&report);
        assert!(text.contains("sharing-none-screencapture=marker pixels=0"));
        assert!(text.contains("sharing-m14-screencapture=claim"));
        assert!(text.contains("sharing-m14-CGWindowListCreateImage=claim"));
        assert!(text.contains("result=pass"));
    }

    #[test]
    fn a_nonzero_marker_under_sharing_none_is_recorded_and_not_claimed() {
        let mut report = passing_fixture();
        report.sharing = Some(measured_sharing(90, 90));
        assert!(failure_reasons(&report).is_empty());
        let text = render(&report);
        assert!(text.contains("sharing-none-CGWindowListCreateImage=marker pixels=90"));
        assert!(text.contains("sharing-m14-CGWindowListCreateImage=do-not-claim"));
        assert!(text.contains("sharing-m14-CGDisplayCreateImage=do-not-claim"));
        assert!(text.contains("sharing-m14-screencapture=do-not-claim"));
        assert!(text.contains("sharing-m14-ScreenCaptureKit=do-not-claim"));
    }

    #[test]
    fn screen_capture_kit_unavailable_is_named_and_not_a_claim() {
        let mut sharing = measured_sharing(10, 10);
        for sample in &mut sharing.samples {
            if sample.api == "ScreenCaptureKit" {
                sample.opened = false;
                sample.marker_pixels = None;
                sample.width = 0;
                sample.height = 0;
                sample.error =
                    Some("ScreenCaptureKit: NSError SCStreamErrorDomain -3801".to_string());
            }
        }
        let mut report = passing_fixture();
        report.sharing = Some(sharing);
        assert!(failure_reasons(&report).is_empty());
        let text = render(&report);
        assert!(text.contains(
            "sharing-default-ScreenCaptureKit=unavailable error=ScreenCaptureKit: NSError SCStreamErrorDomain -3801 entry=SCScreenshotManager excludingWindows empty"
        ));
        assert!(text.contains("sharing-m14-ScreenCaptureKit=do-not-claim"));
    }

    #[test]
    fn shielding_or_a_missing_marker_count_fails_the_sharing_measurement() {
        let mut report = passing_fixture();
        report.sharing = None;
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("sharing measurement is missing"))
        );

        let mut sharing = measured_sharing(10, 0);
        sharing.window.shielding_called = true;
        report.sharing = Some(sharing);
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("CGShieldingWindowLevel"))
        );

        let mut sharing = measured_sharing(10, 0);
        let sample = sharing
            .samples
            .iter_mut()
            .find(|sample| sample.sharing == "none" && sample.api == "screencapture")
            .unwrap();
        sample.marker_pixels = None;
        report.sharing = Some(sharing);
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("without a marker count"))
        );
    }

    #[test]
    fn an_empty_sw_vers_field_fails() {
        let mut report = passing_fixture();
        report.version.build_version.clear();
        assert!(
            failure_reasons(&report)
                .iter()
                .any(|reason| reason.contains("sw_vers"))
        );
    }
}
