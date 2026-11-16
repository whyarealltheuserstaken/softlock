//! Count progress-log gaps longer than 30 seconds, per scenario.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::logline::{HeaderMeta, LogEvent, ParseError, parse_headers, parse_log};
use crate::plan::GAP_LIMIT_MS;
use crate::scenario::MEASUREMENT_SCENARIOS;
use crate::signals::SignalRollup;
use crate::timefmt::{format_ms_as_secs, format_unix_millis};

const GAPS_LISTED: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    pub from_ms: u64,
    pub to_ms: u64,
    pub delta_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioReport {
    pub scenario: String,
    pub samples: usize,
    pub gaps: Vec<Gap>,
    pub ui_failures: usize,
    pub stop: Option<String>,
    pub meta: HeaderMeta,
    pub missing: bool,
    pub span_ms: Option<u64>,
    pub signals: SignalRollup,
    pub ac_mismatches: usize,
    pub complete: bool,
}

impl ScenarioReport {
    pub fn missing(scenario: impl Into<String>) -> Self {
        Self {
            scenario: scenario.into(),
            samples: 0,
            gaps: Vec::new(),
            ui_failures: 0,
            stop: None,
            meta: HeaderMeta::default(),
            missing: true,
            span_ms: None,
            signals: SignalRollup::default(),
            ac_mismatches: 0,
            complete: false,
        }
    }

    pub fn longest_gap_ms(&self) -> Option<u64> {
        self.gaps.iter().map(|gap| gap.delta_ms).max()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub text: String,
    pub samples: u64,
    pub gaps: u64,
}

#[derive(Debug)]
pub enum AnalyzeError {
    Io { path: String, message: String },
    Parse { path: String, error: ParseError },
}

impl std::fmt::Display for AnalyzeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalyzeError::Io { path, message } => write!(f, "{path}: {message}"),
            AnalyzeError::Parse { path, error } => write!(f, "{path}: {error}"),
        }
    }
}

impl std::error::Error for AnalyzeError {}

pub fn summarize_text(text: &str, required: &[&str]) -> Result<Summary, ParseError> {
    let events = parse_log(text)?;
    let headers = parse_headers(text);
    let mut grouped: BTreeMap<String, Vec<LogEvent>> = BTreeMap::new();
    for event in events {
        grouped
            .entry(event.scenario().to_string())
            .or_default()
            .push(event);
    }
    let mut found = Vec::new();
    for (scenario, mut events) in grouped {
        events.sort_by_key(LogEvent::at_ms);
        found.push(report_for(&scenario, &events, headers.get(&scenario)));
    }
    Ok(assemble(found, required))
}

pub fn summarize_dir(dir: &Path) -> Result<Summary, AnalyzeError> {
    summarize_dir_with(dir, &MEASUREMENT_SCENARIOS)
}

pub fn summarize_dir_with(dir: &Path, required: &[&str]) -> Result<Summary, AnalyzeError> {
    if !dir.exists() {
        let mut summary = summarize_text("", required).expect("empty log");
        summary.text = format!("No logs in {} yet.\n\n{}", dir.display(), summary.text);
        return Ok(summary);
    }
    let mut names = Vec::new();
    let entries = fs::read_dir(dir).map_err(|error| AnalyzeError::Io {
        path: dir.display().to_string(),
        message: error.to_string(),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| AnalyzeError::Io {
            path: dir.display().to_string(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("log") {
            names.push(path);
        }
    }
    names.sort();
    let mut text = String::new();
    for path in &names {
        let chunk = fs::read_to_string(path).map_err(|error| AnalyzeError::Io {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        text.push_str(&chunk);
        if !chunk.ends_with('\n') {
            text.push('\n');
        }
    }
    summarize_text(&text, required).map_err(|error| AnalyzeError::Parse {
        path: dir.display().to_string(),
        error,
    })
}

fn report_for(scenario: &str, events: &[LogEvent], meta: Option<&HeaderMeta>) -> ScenarioReport {
    let mut samples = 0;
    let mut ui_failures = 0;
    let mut stop = None;
    let mut signals = SignalRollup::default();
    let mut times = Vec::with_capacity(events.len());
    let mut ac_mismatches = 0usize;
    let expected_ac = expects_ac(scenario);
    for event in events {
        times.push(event.at_ms());
        match event {
            LogEvent::Tick {
                ui_ok,
                signals: sample,
                ..
            } => {
                samples += 1;
                if !ui_ok {
                    ui_failures += 1;
                }
                if let (Some(expected), Some(actual)) = (expected_ac, sample.ac_online)
                    && expected != actual
                {
                    ac_mismatches += 1;
                }
                signals.observe(*sample);
            }
            LogEvent::End { reason, .. } => stop = Some(reason.clone()),
        }
    }
    let mut gaps = Vec::new();
    for pair in times.windows(2) {
        let delta = pair[1].saturating_sub(pair[0]);
        if delta > GAP_LIMIT_MS {
            gaps.push(Gap {
                from_ms: pair[0],
                to_ms: pair[1],
                delta_ms: delta,
            });
        }
    }
    let span = times
        .last()
        .copied()
        .zip(times.first().copied())
        .map(|(end, start)| end.saturating_sub(start));
    let planned_secs = meta.and_then(|header| header.planned_secs);
    let complete = run_is_complete(span, planned_secs, stop.as_deref());
    ScenarioReport {
        scenario: scenario.to_string(),
        samples,
        gaps,
        ui_failures,
        stop,
        meta: meta.cloned().unwrap_or_default(),
        missing: false,
        span_ms: span,
        signals,
        ac_mismatches,
        complete,
    }
}

fn expects_ac(scenario: &str) -> Option<bool> {
    match scenario {
        "lid-open-plugged" | "lid-closed-plugged" => Some(true),
        "lid-open-battery" | "lid-closed-battery" => Some(false),
        _ => None,
    }
}

/// A finished hour ends on the timer and covers the planned length, with about a minute of slack.
pub const COMPLETE_SLACK_MS: u64 = 60_000;

fn run_is_complete(span_ms: Option<u64>, planned_secs: Option<u64>, stop: Option<&str>) -> bool {
    let (Some(span), Some(planned)) = (span_ms, planned_secs) else {
        return false;
    };
    stop == Some("timer") && span.saturating_add(COMPLETE_SLACK_MS) >= planned.saturating_mul(1000)
}

fn assemble(found: Vec<ScenarioReport>, required: &[&str]) -> Summary {
    let mut ordered = Vec::new();
    for id in required {
        if let Some(report) = found.iter().find(|report| report.scenario == *id) {
            ordered.push(report.clone());
        } else {
            ordered.push(ScenarioReport::missing((*id).to_string()));
        }
    }
    let mut extras: Vec<_> = found
        .iter()
        .filter(|report| !required.contains(&report.scenario.as_str()))
        .cloned()
        .collect();
    extras.sort_by(|left, right| left.scenario.cmp(&right.scenario));
    ordered.extend(extras);

    let samples = ordered.iter().map(|report| report.samples as u64).sum();
    let gaps = ordered.iter().map(|report| report.gaps.len() as u64).sum();
    let text = format_summary(&ordered);
    Summary {
        text,
        samples,
        gaps,
    }
}

pub fn format_summary(reports: &[ScenarioReport]) -> String {
    let mut text = format!(
        "Keep-awake log summary\nA gap is a wait longer than {} s between log lines.\n",
        GAP_LIMIT_MS / 1000
    );
    for report in reports {
        text.push('\n');
        text.push_str(&report.scenario);
        text.push('\n');
        if report.missing {
            text.push_str("  no log\n");
            continue;
        }
        text.push_str(&format!("  samples: {}\n", report.samples));
        text.push_str(&format!(
            "  gaps longer than {} s: {}\n",
            GAP_LIMIT_MS / 1000,
            report.gaps.len()
        ));
        text.push_str(&format!(
            "  longest gap: {}\n",
            report
                .longest_gap_ms()
                .map(format_ms_as_secs)
                .unwrap_or_else(|| "none".to_string())
        ));
        text.push_str(&format!("  ui failures: {}\n", report.ui_failures));
        text.push_str(&format!(
            "  span: {}\n",
            report
                .span_ms
                .map(format_ms_as_secs)
                .unwrap_or_else(|| "none".to_string())
        ));
        text.push_str(&report.signals.summary_lines());
        text.push_str(&format!(
            "  stop: {}\n",
            report.stop.as_deref().unwrap_or("not recorded")
        ));
        text.push_str(if report.complete {
            "  result: complete\n"
        } else {
            "  result: INCOMPLETE\n"
        });
        text.push_str(&format!("  ac mismatches: {}\n", report.ac_mismatches));
        push_meta(
            &mut text,
            "modern standby (AoAc)",
            report.meta.aoac.as_deref(),
        );
        push_meta(
            &mut text,
            "power source",
            report.meta.power_source.as_deref(),
        );
        push_meta(&mut text, "battery", report.meta.battery_percent.as_deref());
        push_meta(
            &mut text,
            "lid action on power",
            report.meta.lid_action_ac.as_deref(),
        );
        push_meta(
            &mut text,
            "lid action on battery",
            report.meta.lid_action_dc.as_deref(),
        );
        push_meta(&mut text, "requests", report.meta.requests.as_deref());
        push_meta(
            &mut text,
            "execution state",
            report.meta.execution_state.as_deref(),
        );
        push_meta(&mut text, "overlay", report.meta.overlay.as_deref());
        push_meta(&mut text, "ui automation", report.meta.probe.as_deref());
        for (index, gap) in report.gaps.iter().take(GAPS_LISTED).enumerate() {
            text.push_str(&format!(
                "  gap {}: {} -> {} ({})\n",
                index + 1,
                format_unix_millis(gap.from_ms),
                format_unix_millis(gap.to_ms),
                format_ms_as_secs(gap.delta_ms)
            ));
        }
        if report.gaps.len() > GAPS_LISTED {
            text.push_str(&format!(
                "  further gaps not listed: {}\n",
                report.gaps.len() - GAPS_LISTED
            ));
        }
    }
    text
}

fn push_meta(text: &mut String, label: &str, value: Option<&str>) {
    if let Some(value) = value {
        text.push_str(&format!("  {label}: {value}\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logline::LogEvent;
    use crate::signals::PowerSignals;

    fn tick(at_ms: u64, scenario: &str, ok: bool) -> String {
        LogEvent::Tick {
            at_ms,
            scenario: scenario.to_string(),
            ui_ok: ok,
            kind: "uia".to_string(),
            signals: PowerSignals::default(),
        }
        .format_line()
    }

    fn end(at_ms: u64, scenario: &str, reason: &str) -> String {
        LogEvent::End {
            at_ms,
            scenario: scenario.to_string(),
            reason: reason.to_string(),
        }
        .format_line()
    }

    #[test]
    fn thirty_seconds_is_not_a_gap_and_one_millisecond_more_is() {
        let exact = format!(
            "{}\n{}\n",
            tick(0, "lid-open-plugged", true),
            tick(30_000, "lid-open-plugged", true)
        );
        let summary = summarize_text(&exact, &MEASUREMENT_SCENARIOS).unwrap();
        assert_eq!(summary.gaps, 0);

        let over = format!(
            "{}\n{}\n",
            tick(0, "lid-open-plugged", true),
            tick(30_001, "lid-open-plugged", true)
        );
        let summary = summarize_text(&over, &MEASUREMENT_SCENARIOS).unwrap();
        assert_eq!(summary.gaps, 1);
        assert!(summary.text.contains("longest gap: 30.001s"));
    }

    #[test]
    fn scenarios_are_counted_separately_and_an_end_line_counts() {
        let text = format!(
            "# scenario=lid-open-plugged\n# aoac=yes\n# power_source=plugged-in\n# battery_percent=80\n# lid_action_ac=sleep\n# lid_action_dc=sleep\n# requests=system-required+display-required\n# execution_state=ES_CONTINUOUS|ES_SYSTEM_REQUIRED\n# overlay=dim\n# probe=uia\n{}\n{}\n{}\n# scenario=lid-open-battery\n{}\n",
            tick(0, "lid-open-plugged", true),
            tick(5_000, "lid-open-plugged", false),
            end(50_000, "lid-open-plugged", "timer"),
            tick(100_000, "lid-open-battery", true),
        );
        let summary = summarize_text(&text, &MEASUREMENT_SCENARIOS).unwrap();
        assert!(
            summary.text.contains("gaps longer than 30 s: 1"),
            "{}",
            summary.text
        );
        assert!(summary.text.contains("ui failures: 1"), "{}", summary.text);
        assert!(summary.text.contains("stop: timer"), "{}", summary.text);
        assert!(
            summary.text.contains("modern standby (AoAc): yes"),
            "{}",
            summary.text
        );
        assert!(summary.text.contains("span: 50.000s"), "{}", summary.text);
        assert!(
            summary
                .text
                .contains("gap 1: 1970-01-01T00:00:05.000Z -> 1970-01-01T00:00:50.000Z (45.000s)"),
            "{}",
            summary.text
        );
        assert!(
            summary.text.contains("lid-open-battery\n  samples: 1\n"),
            "{}",
            summary.text
        );
        assert!(
            summary.text.contains("lid-closed-plugged\n  no log\n"),
            "{}",
            summary.text
        );
        assert!(
            summary.text.contains("lid-closed-battery\n  no log\n"),
            "{}",
            summary.text
        );
        // The battery sample is alone, so it adds no gap. The plugged scenario has one.
        assert_eq!(summary.gaps, 1);
        assert_eq!(summary.samples, 3);
    }

    #[test]
    fn unsorted_lines_are_ordered_before_gaps_are_counted() {
        let text = format!(
            "{}\n{}\n",
            tick(5_000, "lid-open-plugged", true),
            tick(0, "lid-open-plugged", true)
        );
        let summary = summarize_text(&text, &["lid-open-plugged"]).unwrap();
        assert_eq!(summary.gaps, 0);
        assert!(summary.text.contains("span: 5.000s"));
    }

    #[test]
    fn long_gap_lists_are_capped() {
        let mut text = String::new();
        for index in 0..12 {
            text.push_str(&tick(index * 40_000, "lid-open-plugged", true));
            text.push('\n');
        }
        let summary = summarize_text(&text, &["lid-open-plugged"]).unwrap();
        assert_eq!(summary.gaps, 11);
        assert!(summary.text.contains("gap 10:"));
        assert!(!summary.text.contains("gap 11:"));
        assert!(summary.text.contains("further gaps not listed: 1"));
    }

    #[test]
    fn a_missing_folder_lists_the_four_scenarios() {
        let missing = std::env::temp_dir().join(format!(
            "softlock-spike-keep-awake-missing-{}-{}",
            std::process::id(),
            unix_tail()
        ));
        let summary = summarize_dir(&missing).unwrap();
        assert!(summary.text.contains("No logs in"));
        assert!(summary.text.contains("lid-open-plugged\n  no log"));
        assert_eq!(summary.gaps, 0);
        assert_eq!(summary.samples, 0);
    }

    #[test]
    fn battery_and_thermal_samples_are_counted_for_the_threshold() {
        let warm = LogEvent::Tick {
            at_ms: 0,
            scenario: "lid-open-battery".to_string(),
            ui_ok: true,
            kind: "uia".to_string(),
            signals: PowerSignals {
                battery_percent: Some(64),
                cooling: Some(crate::signals::Cooling::Active),
                mhz_max: Some(1800),
                mhz_limit: Some(1800),
                execution_throttled: Some(false),
                ..PowerSignals::default()
            },
        };
        let hot = LogEvent::Tick {
            at_ms: 5_000,
            scenario: "lid-open-battery".to_string(),
            ui_ok: true,
            kind: "uia".to_string(),
            signals: PowerSignals {
                battery_percent: Some(37),
                battery_saver: Some(true),
                cooling: Some(crate::signals::Cooling::Passive),
                mhz_max: Some(1800),
                mhz_limit: Some(700),
                execution_throttled: Some(true),
                ..PowerSignals::default()
            },
        };
        let text = format!("{}\n{}\n", warm.format_line(), hot.format_line());
        let summary = summarize_text(&text, &["lid-open-battery"]).unwrap();
        assert!(
            summary.text.contains("lowest battery: 37"),
            "{}",
            summary.text
        );
        assert!(
            summary.text.contains("passive cooling 1"),
            "{}",
            summary.text
        );
        assert!(
            summary.text.contains("cpu speed limit 1"),
            "{}",
            summary.text
        );
        assert!(
            summary.text.contains("process throttle 1"),
            "{}",
            summary.text
        );
        assert!(summary.text.contains("battery saver 1"), "{}", summary.text);
        assert!(summary.text.contains("policy/cap:"), "{}", summary.text);
        assert!(
            summary.text.contains("heat: not recorded"),
            "{}",
            summary.text
        );
        assert!(!summary.text.contains("thermal or throttling"));
        assert_eq!(summary.gaps, 0);
    }

    #[test]
    fn a_short_or_early_stop_is_incomplete_and_the_wrong_cord_is_flagged() {
        let header = "# scenario=lid-open-battery\n# planned_secs=3600\n";
        let tick = LogEvent::Tick {
            at_ms: 0,
            scenario: "lid-open-battery".to_string(),
            ui_ok: true,
            kind: "uia".to_string(),
            signals: PowerSignals {
                battery_percent: Some(90),
                ac_online: Some(true),
                ..PowerSignals::default()
            },
        };
        let end = LogEvent::End {
            at_ms: 5_000,
            scenario: "lid-open-battery".to_string(),
            reason: "key".to_string(),
        };
        let text = format!("{header}{}\n{}\n", tick.format_line(), end.format_line());
        let summary = summarize_text(&text, &["lid-open-battery"]).unwrap();
        assert!(
            summary.text.contains("result: INCOMPLETE"),
            "{}",
            summary.text
        );
        assert!(
            summary.text.contains("ac mismatches: 1"),
            "{}",
            summary.text
        );

        let done = format!(
            "# scenario=lid-open-plugged\n# planned_secs=60\n{}\n{}\n",
            LogEvent::Tick {
                at_ms: 0,
                scenario: "lid-open-plugged".to_string(),
                ui_ok: true,
                kind: "uia".to_string(),
                signals: PowerSignals {
                    ac_online: Some(true),
                    battery_percent: Some(100),
                    ..PowerSignals::default()
                },
            }
            .format_line(),
            LogEvent::End {
                at_ms: 60_000,
                scenario: "lid-open-plugged".to_string(),
                reason: "timer".to_string(),
            }
            .format_line()
        );
        let summary = summarize_text(&done, &["lid-open-plugged"]).unwrap();
        assert!(
            summary.text.contains("result: complete"),
            "{}",
            summary.text
        );
        assert!(
            summary.text.contains("ac mismatches: 0"),
            "{}",
            summary.text
        );
    }

    #[test]
    fn a_readable_calm_heat_sample_is_not_called_unavailable() {
        let calm = LogEvent::Tick {
            at_ms: 0,
            scenario: "practice".to_string(),
            ui_ok: true,
            kind: "uia".to_string(),
            signals: PowerSignals {
                heat_checked: true,
                passive_limit: Some(100),
                throttle_reasons: Some(0),
                ..PowerSignals::default()
            },
        };
        let calm_summary =
            summarize_text(&format!("{}\n", calm.format_line()), &["practice"]).unwrap();
        assert!(
            calm_summary.text.contains("heat: readable, no throttling"),
            "{}",
            calm_summary.text
        );
        assert!(
            !calm_summary.text.contains("heat: unavailable"),
            "{}",
            calm_summary.text
        );

        let missing = LogEvent::Tick {
            at_ms: 0,
            scenario: "practice".to_string(),
            ui_ok: true,
            kind: "uia".to_string(),
            signals: PowerSignals {
                heat_checked: true,
                ..PowerSignals::default()
            },
        };
        let missing_summary =
            summarize_text(&format!("{}\n", missing.format_line()), &["practice"]).unwrap();
        assert!(
            missing_summary.text.contains("heat: unavailable"),
            "{}",
            missing_summary.text
        );
        assert!(
            !missing_summary.text.contains("readable, no throttling"),
            "{}",
            missing_summary.text
        );

        let hot = LogEvent::Tick {
            at_ms: 0,
            scenario: "practice".to_string(),
            ui_ok: true,
            kind: "uia".to_string(),
            signals: PowerSignals {
                heat_checked: true,
                passive_limit: Some(80),
                throttle_reasons: Some(1),
                ..PowerSignals::default()
            },
        };
        let hot_summary =
            summarize_text(&format!("{}\n", hot.format_line()), &["practice"]).unwrap();
        assert!(
            hot_summary
                .text
                .contains("passive limit below 100 on 1 samples"),
            "{}",
            hot_summary.text
        );
        assert!(
            hot_summary
                .text
                .contains("thermal throttle bit on 1 samples"),
            "{}",
            hot_summary.text
        );
    }

    fn unix_tail() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
