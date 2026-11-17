//! Batched physical rows. Skips are decided before the prompt.
//! A step that was attempted and not seen is a failure.

use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::detect::DetectedConfig;
use crate::display::{connected_names, parse_drm_dir};
use crate::power::{parse_lid_closed_property, parse_locked_hint, parse_prepare_for_sleep};
use crate::prompt::{PromptRead, interpret_read};
use crate::report::{TestRow, empty_measurements};
use crate::restore::{hold_sleep_inhibit, interrupted, release_sleep_inhibit};

const WAIT: Duration = Duration::from_secs(15);
const UNLOCK_WAIT: Duration = Duration::from_secs(90);

#[derive(Clone, Debug)]
pub struct PhysicalFacts {
    pub ci: bool,
    pub logind: bool,
    pub lid_known: bool,
    pub drm: bool,
    pub journal_readable: bool,
    pub session_id: Option<String>,
}

pub fn precheck(id: &str, facts: &PhysicalFacts) -> Option<&'static str> {
    if facts.ci {
        return Some("ci");
    }
    match id {
        "physical.lid" if !facts.logind || !facts.lid_known => Some("not-applicable"),
        "physical.display-hotplug" if !facts.drm => Some("not-applicable"),
        "physical.power-key" if !facts.logind || !facts.journal_readable => Some("not-applicable"),
        "physical.password-unlock" if !facts.logind || facts.session_id.is_none() => {
            Some("not-applicable")
        }
        _ => None,
    }
}

pub fn journalctl_since_args(since: &str) -> Vec<String> {
    vec![
        "-u".into(),
        "systemd-logind".into(),
        "--since".into(),
        since.into(),
        "-o".into(),
        "cat".into(),
        "--no-pager".into(),
        "--no-hostname".into(),
    ]
}

pub fn journal_has_power_key(text: &str) -> bool {
    text.lines().any(|line| {
        line.to_ascii_lowercase().contains("power key")
            || parse_prepare_for_sleep(line) == Some(true)
    })
}

pub fn format_utc(secs: u64) -> String {
    let days = secs / 86_400;
    let tod = secs % 86_400;
    let hour = tod / 3_600;
    let min = (tod % 3_600) / 60;
    let sec = tod % 60;
    let (year, month, day) = civil_from_days(days as i64);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02}")
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    (year as i32, month as u32, day as u32)
}

pub fn lock_session_args(session: &str) -> Vec<String> {
    vec!["lock-session".into(), session.into()]
}

pub fn locked_hint_args(session: &str) -> Vec<String> {
    vec![
        "show-session".into(),
        session.into(),
        "--property=LockedHint".into(),
    ]
}

pub fn run_batch(config: &DetectedConfig, ask: bool) -> Vec<TestRow> {
    let facts = PhysicalFacts {
        ci: !ask,
        logind: config.logind,
        lid_known: lid_property_known(),
        drm: drm_present(),
        journal_readable: journal_readable(),
        session_id: session_id(),
    };
    let mut rows = Vec::new();
    let mut stop = false;
    for (id, name, prompt, kind) in STEPS {
        if stop || interrupted() {
            rows.push(skipped(
                id,
                name,
                "interrupted",
                "interrupted before the prompt",
            ));
            stop = true;
            continue;
        }
        if let Some(reason) = precheck(id, &facts) {
            let detail = match reason {
                "ci" => "ci: no physical steps",
                _ => "not available on this machine",
            };
            rows.push(skipped(id, name, reason, detail));
            continue;
        }
        if matches!(kind, Step::Power | Step::Lid) && hold_sleep_inhibit().is_err() {
            rows.push(TestRow::fail(
                id,
                name,
                true,
                false,
                "inhibit was not taken",
                empty_measurements(),
            ));
            continue;
        }
        match read_prompt(prompt) {
            PromptRead::Stop => {
                rows.push(skipped(id, name, "interrupted", "interrupted"));
                release_sleep_inhibit();
                stop = true;
            }
            PromptRead::Skip(reason) => {
                rows.push(skipped(id, name, &reason, &reason));
                release_sleep_inhibit();
            }
            PromptRead::Go => {
                let started = Instant::now();
                let since = format_utc(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                );
                let result = match kind {
                    Step::Lid => detect_lid(),
                    Step::Hotplug => detect_hotplug(),
                    Step::Power => detect_power(&since),
                    Step::Unlock => detect_unlock(facts.session_id.as_deref().unwrap_or("")),
                };
                release_sleep_inhibit();
                let mut row = match result {
                    Ok(detail) => {
                        TestRow::pass(id, name, true, false, detail, empty_measurements())
                    }
                    Err(err) => TestRow::fail(id, name, true, false, err, empty_measurements()),
                };
                row.duration_ms = started.elapsed().as_millis() as u64;
                rows.push(row);
            }
        }
    }
    rows
}

#[derive(Clone, Copy)]
enum Step {
    Lid,
    Hotplug,
    Power,
    Unlock,
}

const STEPS: &[(&str, &str, &str, Step)] = &[
    (
        "physical.lid",
        "Lid close/open",
        "lid: close the lid, then open it",
        Step::Lid,
    ),
    (
        "physical.display-hotplug",
        "Plug/unplug a display",
        "display: plug or unplug a monitor",
        Step::Hotplug,
    ),
    (
        "physical.power-key",
        "Power key",
        "power: tap the power key",
        Step::Power,
    ),
    (
        "physical.password-unlock",
        "Password unlock after OS lock",
        "unlock: the session will lock; sign in with your password",
        Step::Unlock,
    ),
];

fn read_prompt(prompt: &str) -> PromptRead {
    if interrupted() {
        return PromptRead::Stop;
    }
    eprint!("{prompt}: ");
    let _ = io::stderr().flush();
    let mut line = String::new();
    let read = io::stdin().lock().read_line(&mut line);
    let kind = match &read {
        Ok(n) => Ok(*n),
        Err(err) => Err(err.kind()),
    };
    let decision = interpret_read(interrupted(), kind, &line);
    if decision == PromptRead::Stop {
        crate::restore::INTERRUPTED.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    decision
}

fn detect_lid() -> Result<String, String> {
    let start = lid_closed();
    let deadline = Instant::now() + WAIT;
    let mut saw_true = start == Some(true);
    let mut saw_false = start == Some(false);
    while Instant::now() < deadline {
        if interrupted() {
            return Err("interrupted".into());
        }
        match lid_closed() {
            Some(true) => saw_true = true,
            Some(false) => saw_false = true,
            None => {}
        }
        if saw_true && saw_false {
            return Ok("LidClosed toggled".into());
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err("no LidClosed change".into())
}

fn detect_hotplug() -> Result<String, String> {
    let before = connected_names(&parse_drm_dir("/sys/class/drm"));
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if interrupted() {
            return Err("interrupted".into());
        }
        let now = connected_names(&parse_drm_dir("/sys/class/drm"));
        if now != before {
            return Ok(format!("drm connected {} -> {}", before.len(), now.len()));
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err("no DRM connector change".into())
}

fn detect_power(since: &str) -> Result<String, String> {
    let pid = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
    let (tx, rx) = mpsc::channel();
    let watch = pid.clone();
    thread::spawn(move || {
        let mut child = match Command::new("busctl")
            .args([
                "monitor",
                "--match",
                "type='signal',interface='org.freedesktop.login1.Manager',member='PrepareForSleep'",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => {
                let _ = tx.send(String::new());
                return;
            }
        };
        watch.store(child.id() as i32, std::sync::atomic::Ordering::SeqCst);
        let mut text = String::new();
        if let Some(mut out) = child.stdout.take() {
            let _ = std::io::Read::read_to_string(&mut out, &mut text);
        }
        let _ = child.wait();
        let _ = tx.send(text);
    });
    let deadline = Instant::now() + WAIT;
    let mut found = None;
    while Instant::now() < deadline {
        if interrupted() {
            found = Some(Err("interrupted".into()));
            break;
        }
        if let Ok(out) = Command::new("journalctl")
            .args(journalctl_since_args(since))
            .output()
            && journal_has_power_key(&String::from_utf8_lossy(&out.stdout))
        {
            found = Some(Ok("journal since prompt".into()));
            break;
        }
        thread::sleep(Duration::from_millis(400));
    }
    let id = pid.load(std::sync::atomic::Ordering::SeqCst);
    if id > 0 {
        unsafe {
            libc::kill(id, libc::SIGTERM);
        }
    }
    if let Some(result) = found {
        return result;
    }
    if let Ok(text) = rx.recv_timeout(Duration::from_secs(1))
        && (text.contains("PrepareForSleep") || journal_has_power_key(&text))
    {
        return Ok("logind PrepareForSleep".into());
    }
    Err("no power-key signal since the prompt".into())
}

fn detect_unlock(session: &str) -> Result<String, String> {
    let args = lock_session_args(session);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let locked = Command::new("loginctl").args(&arg_refs).status();
    if !locked.is_ok_and(|status| status.success()) {
        return Err("loginctl lock-session failed".into());
    }
    let deadline = Instant::now() + UNLOCK_WAIT;
    let mut saw_yes = false;
    while Instant::now() < deadline {
        if interrupted() {
            return Err("interrupted".into());
        }
        match locked_hint(session) {
            Some(true) => saw_yes = true,
            Some(false) if saw_yes => return Ok("LockedHint yes then no".into()),
            _ => {}
        }
        thread::sleep(Duration::from_millis(250));
    }
    if saw_yes {
        Err("LockedHint stayed yes".into())
    } else {
        Err("LockedHint never became yes".into())
    }
}

fn lid_closed() -> Option<bool> {
    let out = Command::new("busctl")
        .args([
            "get-property",
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "LidClosed",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_lid_closed_property(&String::from_utf8_lossy(&out.stdout))
}

fn locked_hint(session: &str) -> Option<bool> {
    let args = locked_hint_args(session);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = Command::new("loginctl").args(&arg_refs).output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_locked_hint(&String::from_utf8_lossy(&out.stdout))
}

fn lid_property_known() -> bool {
    lid_closed().is_some()
}

fn drm_present() -> bool {
    std::path::Path::new("/sys/class/drm").is_dir()
}

fn journal_readable() -> bool {
    Command::new("journalctl")
        .args([
            "-n",
            "0",
            "-o",
            "cat",
            "--since",
            "now",
            "-u",
            "systemd-logind",
            "--no-pager",
        ])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn session_id() -> Option<String> {
    std::env::var("XDG_SESSION_ID")
        .ok()
        .filter(|value| !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()))
}

fn skipped(id: &str, name: &str, reason: &str, detail: &str) -> TestRow {
    let mut measurements = empty_measurements();
    measurements.insert("reason".into(), json!(reason));
    TestRow::skip(id, name, reason, true, false, detail, measurements)
}

#[cfg(test)]
mod tests {
    use super::{
        PhysicalFacts, format_utc, journal_has_power_key, journalctl_since_args, lock_session_args,
        locked_hint_args, precheck,
    };
    use crate::report::TestRow;

    fn facts() -> PhysicalFacts {
        PhysicalFacts {
            ci: false,
            logind: true,
            lid_known: true,
            drm: true,
            journal_readable: true,
            session_id: Some("3".into()),
        }
    }

    #[test]
    fn skips_are_decided_before_a_prompt() {
        let mut no_lid = facts();
        no_lid.lid_known = false;
        assert_eq!(precheck("physical.lid", &no_lid), Some("not-applicable"));
        let mut no_journal = facts();
        no_journal.journal_readable = false;
        assert_eq!(
            precheck("physical.power-key", &no_journal),
            Some("not-applicable")
        );
        let mut no_session = facts();
        no_session.session_id = None;
        assert_eq!(
            precheck("physical.password-unlock", &no_session),
            Some("not-applicable")
        );
        let mut ci = facts();
        ci.ci = true;
        assert_eq!(precheck("physical.lid", &ci), Some("ci"));
        assert_eq!(precheck("physical.lid", &facts()), None);
    }

    #[test]
    fn a_miss_is_a_failure() {
        let row = TestRow::fail(
            "physical.display-hotplug",
            "Plug/unplug a display",
            true,
            false,
            "no DRM connector change",
            crate::report::empty_measurements(),
        );
        assert_eq!(row.status, "fail");
        assert!(row.skip_reason.is_none());
    }

    #[test]
    fn journal_since_ignores_an_old_cursor() {
        let args = journalctl_since_args("2026-11-17 18:00:00");
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--since" && w[1] == "2026-11-17 18:00:00")
        );
        assert!(!args.iter().any(|arg| arg == "-n"));
        assert!(!journal_has_power_key(""));
        assert!(journal_has_power_key("Power key pressed\n"));
        assert!(journal_has_power_key("PrepareForSleep boolean true\n"));
        assert!(!journal_has_power_key("System resumed.\n"));
    }

    #[test]
    fn unlock_targets_the_session_id() {
        assert_eq!(
            lock_session_args("7"),
            vec!["lock-session".to_string(), "7".to_string()]
        );
        let hint = locked_hint_args("7");
        assert_eq!(hint[0], "show-session");
        assert_eq!(hint[1], "7");
        assert_eq!(hint[2], "--property=LockedHint");
    }

    #[test]
    fn utc_epoch_is_1970() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00");
    }
}
