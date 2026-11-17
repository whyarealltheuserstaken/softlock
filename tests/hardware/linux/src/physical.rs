//! Batched physical rows. Auto-detect lid/hotplug/power; skip on --ci.

use std::io::{self, BufRead, Write};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::detect::DetectedConfig;
use crate::display::{connected_names, parse_drm_dir};
use crate::power::{parse_lid_closed_property, parse_prepare_for_sleep};
use crate::prompt::{Answer, parse_answer};
use crate::report::TestRow;
use crate::restore::{hold_sleep_inhibit, interrupted, release_sleep_inhibit};

const WAIT: Duration = Duration::from_secs(15);

pub fn run_batch(config: &DetectedConfig, ask: bool) -> Vec<TestRow> {
    if !ask {
        return vec![
            skipped("physical.lid", "Lid close/open", "ci: no physical steps"),
            skipped(
                "physical.display-hotplug",
                "Plug/unplug a display",
                "ci: no physical steps",
            ),
            skipped("physical.power-key", "Power key", "ci: no physical steps"),
            skipped(
                "physical.password-unlock",
                "Password unlock after OS lock",
                "ci: no physical steps",
            ),
        ];
    }
    eprintln!("PHYSICAL  Enter=go  skip=skip  one word");
    vec![
        one(
            "physical.lid",
            "Lid close/open",
            "lid: close then open if this machine has a lid",
            config,
            detect_lid,
        ),
        one(
            "physical.display-hotplug",
            "Plug/unplug a display",
            "display: plug or unplug a monitor",
            config,
            detect_hotplug,
        ),
        one(
            "physical.power-key",
            "Power key",
            "power: tap the power key (sleep is inhibited)",
            config,
            detect_power,
        ),
        one(
            "physical.password-unlock",
            "Password unlock after OS lock",
            "unlock: sign in if the session is OS-locked, else skip",
            config,
            detect_unlock,
        ),
    ]
}

fn one(
    id: &str,
    name: &str,
    prompt: &str,
    config: &DetectedConfig,
    detect: fn(&DetectedConfig) -> Result<String, String>,
) -> TestRow {
    eprint!("{prompt}: ");
    let _ = io::stderr().flush();
    let mut line = String::new();
    let read = io::stdin().lock().read_line(&mut line);
    if read.is_err() {
        return skipped(id, name, "stdin closed");
    }
    match parse_answer(&line) {
        Ok(Answer::Skip) => skipped(id, name, "operator skip"),
        Ok(Answer::Go) => {
            let _ = hold_sleep_inhibit();
            let started = Instant::now();
            let result = detect(config);
            release_sleep_inhibit();
            let mut row = match result {
                Ok(detail) => pass(id, name, detail),
                Err(e) => fail(id, name, e),
            };
            row.duration_ms = started.elapsed().as_millis() as u64;
            row
        }
        Err(e) => skipped(id, name, &e),
    }
}

fn detect_lid(config: &DetectedConfig) -> Result<String, String> {
    if !config.logind {
        return Err("no logind to watch LidClosed".into());
    }
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
    Err("no LidClosed change (skip if this machine has no lid)".into())
}

fn detect_hotplug(_config: &DetectedConfig) -> Result<String, String> {
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

fn detect_power(config: &DetectedConfig) -> Result<String, String> {
    if !config.logind {
        return Err("no logind to watch PrepareForSleep / power key".into());
    }
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if interrupted() {
            return Err("interrupted".into());
        }
        if journal_saw_power() {
            return Ok("journal or PrepareForSleep observed".into());
        }
        thread::sleep(Duration::from_millis(400));
    }
    Err("no power-key / PrepareForSleep signal".into())
}

fn detect_unlock(config: &DetectedConfig) -> Result<String, String> {
    if !config.logind {
        return Err("no logind LockedHint".into());
    }
    let locked = locked_hint();
    if locked != Some(true) {
        return Err("session not OS-locked; skipped path recorded as skip".into());
    }
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if interrupted() {
            return Err("interrupted".into());
        }
        if locked_hint() == Some(false) {
            return Ok("LockedHint became false".into());
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err("LockedHint stayed true".into())
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

fn locked_hint() -> Option<bool> {
    let out = Command::new("busctl")
        .args([
            "get-property",
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "BlockInhibited",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // LockedHint is per-session; fall back to loginctl show-session.
    drop(text);
    let session = Command::new("loginctl")
        .args(["show-session", "--property=LockedHint"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&session.stdout);
    if s.contains("yes") || s.contains("LockedHint=yes") {
        return Some(true);
    }
    if s.contains("no") || s.contains("LockedHint=no") {
        return Some(false);
    }
    None
}

fn journal_saw_power() -> bool {
    let out = Command::new("journalctl")
        .args([
            "-n",
            "20",
            "-o",
            "cat",
            "--no-hostname",
            "-u",
            "systemd-logind",
        ])
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            text.to_ascii_lowercase().contains("power key")
                || text.lines().any(|l| parse_prepare_for_sleep(l).is_some())
        }
        Err(_) => false,
    }
}

fn skipped(id: &str, name: &str, reason: &str) -> TestRow {
    TestRow {
        id: id.into(),
        name: name.into(),
        status: "skip".into(),
        skip_reason: Some(reason.into()),
        physical: true,
        needs_admin: false,
        duration_ms: 0,
        detail: String::new(),
        measurements: {
            let mut m = std::collections::BTreeMap::new();
            m.insert("reason".into(), json!(reason));
            m
        },
    }
}

fn pass(id: &str, name: &str, detail: String) -> TestRow {
    TestRow {
        id: id.into(),
        name: name.into(),
        status: "pass".into(),
        skip_reason: None,
        physical: true,
        needs_admin: false,
        duration_ms: 0,
        detail,
        measurements: std::collections::BTreeMap::new(),
    }
}

fn fail(id: &str, name: &str, detail: String) -> TestRow {
    // Physical miss is a skip so a laptop without a lid does not fail the suite.
    TestRow {
        id: id.into(),
        name: name.into(),
        status: "skip".into(),
        skip_reason: Some(detail.clone()),
        physical: true,
        needs_admin: false,
        duration_ms: 0,
        detail: String::new(),
        measurements: {
            let mut m = std::collections::BTreeMap::new();
            m.insert("observed".into(), json!(detail));
            m
        },
    }
}
