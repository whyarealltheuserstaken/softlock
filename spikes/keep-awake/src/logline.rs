//! One log line per progress tick, plus a closing line so a freeze that lasts
//! until the run stops still shows up as a gap.

use crate::signals::{
    BatteryFlag, Cooling, PowerSignals, parse_bool_token, parse_mhz, parse_percent,
};
use crate::timefmt::{format_unix_millis, parse_unix_millis};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEvent {
    Tick {
        at_ms: u64,
        scenario: String,
        ui_ok: bool,
        kind: String,
        signals: PowerSignals,
    },
    End {
        at_ms: u64,
        scenario: String,
        reason: String,
    },
}

impl LogEvent {
    pub fn at_ms(&self) -> u64 {
        match self {
            LogEvent::Tick { at_ms, .. } | LogEvent::End { at_ms, .. } => *at_ms,
        }
    }

    pub fn scenario(&self) -> &str {
        match self {
            LogEvent::Tick { scenario, .. } | LogEvent::End { scenario, .. } => scenario,
        }
    }

    pub fn format_line(&self) -> String {
        match self {
            LogEvent::Tick {
                at_ms,
                scenario,
                ui_ok,
                kind,
                signals,
            } => {
                let ui = if *ui_ok { "ok" } else { "fail" };
                format!(
                    "{} {scenario} tick ui={ui} kind={kind}{}",
                    format_unix_millis(*at_ms),
                    signals.log_suffix()
                )
            }
            LogEvent::End {
                at_ms,
                scenario,
                reason,
            } => format!(
                "{} {scenario} end reason={reason}",
                format_unix_millis(*at_ms)
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

pub fn parse_log(text: &str) -> Result<Vec<LogEvent>, ParseError> {
    let mut events = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        events.push(parse_event(line, line_number)?);
    }
    Ok(events)
}

fn parse_event(line: &str, line_number: usize) -> Result<LogEvent, ParseError> {
    let mut parts = line.split_whitespace();
    let timestamp = parts
        .next()
        .ok_or_else(|| bad(line_number, "missing timestamp"))?;
    let scenario = parts
        .next()
        .ok_or_else(|| bad(line_number, "missing scenario"))?;
    let event = parts
        .next()
        .ok_or_else(|| bad(line_number, "missing event"))?;
    let at_ms = parse_unix_millis(timestamp).ok_or_else(|| bad(line_number, "bad timestamp"))?;
    if !is_token(scenario) {
        return Err(bad(line_number, "bad scenario"));
    }
    let mut fields = Vec::new();
    for field in parts {
        let (key, value) = field
            .split_once('=')
            .ok_or_else(|| bad(line_number, "expected key=value"))?;
        if !is_token(key) || !is_token(value) {
            return Err(bad(line_number, "bad key or value"));
        }
        if fields.iter().any(|(existing, _)| existing == key) {
            return Err(bad(line_number, "duplicate field"));
        }
        fields.push((key.to_string(), value.to_string()));
    }
    match event {
        "tick" => {
            let ui = required(&fields, "ui", line_number)?;
            let kind = required(&fields, "kind", line_number)?;
            reject_unknown(
                &fields,
                &[
                    "ui",
                    "kind",
                    "battery",
                    "ac",
                    "saver",
                    "flag",
                    "cooling",
                    "mhz_max",
                    "mhz_limit",
                    "mhz_now",
                    "throttle",
                ],
                line_number,
            )?;
            let ui_ok = match ui.as_str() {
                "ok" => true,
                "fail" => false,
                _ => return Err(bad(line_number, "ui must be ok or fail")),
            };
            Ok(LogEvent::Tick {
                at_ms,
                scenario: scenario.to_string(),
                ui_ok,
                kind,
                signals: parse_signals(&fields, line_number)?,
            })
        }
        "end" => {
            let reason = required(&fields, "reason", line_number)?;
            reject_unknown(&fields, &["reason"], line_number)?;
            Ok(LogEvent::End {
                at_ms,
                scenario: scenario.to_string(),
                reason,
            })
        }
        _ => Err(bad(line_number, "event must be tick or end")),
    }
}

fn required(
    fields: &[(String, String)],
    key: &str,
    line_number: usize,
) -> Result<String, ParseError> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
        .ok_or_else(|| bad(line_number, &format!("missing {key}")))
}

fn reject_unknown(
    fields: &[(String, String)],
    allowed: &[&str],
    line_number: usize,
) -> Result<(), ParseError> {
    for (key, _) in fields {
        if !allowed.contains(&key.as_str()) {
            return Err(bad(line_number, &format!("unknown field {key}")));
        }
    }
    Ok(())
}

fn parse_signals(
    fields: &[(String, String)],
    line_number: usize,
) -> Result<PowerSignals, ParseError> {
    let value = |key: &str| {
        fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    let percent = match value("battery") {
        None => None,
        Some(token) => parse_percent(token).map_err(|_| bad(line_number, "bad battery"))?,
    };
    let ac_online = match value("ac") {
        None => None,
        Some(token) => parse_bool_token(token).map_err(|_| bad(line_number, "bad ac"))?,
    };
    let battery_saver = match value("saver") {
        None => None,
        Some(token) => parse_bool_token(token).map_err(|_| bad(line_number, "bad saver"))?,
    };
    let battery_flag = match value("flag") {
        None | Some("unknown") => None,
        Some(token) => Some(BatteryFlag::parse(token).ok_or_else(|| bad(line_number, "bad flag"))?),
    };
    let cooling = match value("cooling") {
        None | Some("unknown") => None,
        Some(token) => Some(Cooling::parse(token).ok_or_else(|| bad(line_number, "bad cooling"))?),
    };
    let mhz = |key: &str| match value(key) {
        None => Ok(None),
        Some(token) => parse_mhz(token).map_err(|_| bad(line_number, "bad mhz")),
    };
    let execution_throttled = match value("throttle") {
        None => None,
        Some(token) => parse_bool_token(token).map_err(|_| bad(line_number, "bad throttle"))?,
    };
    Ok(PowerSignals {
        battery_percent: percent,
        ac_online,
        battery_saver,
        battery_flag,
        cooling,
        mhz_max: mhz("mhz_max")?,
        mhz_limit: mhz("mhz_limit")?,
        mhz_current: mhz("mhz_now")?,
        execution_throttled,
    })
}

fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

fn bad(line: usize, message: &str) -> ParseError {
    ParseError {
        line,
        message: message.to_string(),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderMeta {
    pub aoac: Option<String>,
    pub power_source: Option<String>,
    pub battery_percent: Option<String>,
    pub lid_action_ac: Option<String>,
    pub lid_action_dc: Option<String>,
    pub requests: Option<String>,
    pub execution_state: Option<String>,
    pub overlay: Option<String>,
    pub probe: Option<String>,
}

/// Header keys attach to the scenario named by the latest `# scenario=` line.
pub fn parse_headers(text: &str) -> std::collections::BTreeMap<String, HeaderMeta> {
    let mut metas = std::collections::BTreeMap::<String, HeaderMeta>::new();
    let mut current: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix('#') else {
            continue;
        };
        let Some((key, value)) = rest.trim().split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key == "scenario" {
            current = Some(value.to_string());
            metas.entry(value.to_string()).or_default();
            continue;
        }
        let Some(scenario) = current.as_ref() else {
            continue;
        };
        let meta = metas.entry(scenario.clone()).or_default();
        match key {
            "aoac" => meta.aoac = Some(value.to_string()),
            "power_source" => meta.power_source = Some(value.to_string()),
            "battery_percent" => meta.battery_percent = Some(value.to_string()),
            "lid_action_ac" => meta.lid_action_ac = Some(value.to_string()),
            "lid_action_dc" => meta.lid_action_dc = Some(value.to_string()),
            "requests" => meta.requests = Some(value.to_string()),
            "execution_state" => meta.execution_state = Some(value.to_string()),
            "overlay" => meta.overlay = Some(value.to_string()),
            "probe" => meta.probe = Some(value.to_string()),
            _ => {}
        }
    }
    metas
}

pub fn format_header(fields: &[(&str, &str)]) -> String {
    let mut text = String::from("# keep-awake spike\n");
    for (key, value) in fields {
        text.push_str(&format!("# {key}={value}\n"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::{Cooling, PowerSignals};

    #[test]
    fn tick_and_end_lines_round_trip() {
        let tick = LogEvent::Tick {
            at_ms: 45_000,
            scenario: "lid-open-plugged".to_string(),
            ui_ok: true,
            kind: "uia".to_string(),
            signals: PowerSignals {
                battery_percent: Some(42),
                cooling: Some(Cooling::Passive),
                mhz_max: Some(2000),
                mhz_limit: Some(900),
                execution_throttled: Some(true),
                ..PowerSignals::default()
            },
        };
        let end = LogEvent::End {
            at_ms: 90_000,
            scenario: "lid-open-plugged".to_string(),
            reason: "timer".to_string(),
        };
        let text = format!(
            "# scenario=lid-open-plugged\n# aoac=yes\n\n{}\n{}\n",
            tick.format_line(),
            end.format_line()
        );
        assert_eq!(parse_log(&text).unwrap(), vec![tick, end]);
        let meta = parse_headers(&text);
        assert_eq!(meta["lid-open-plugged"].aoac.as_deref(), Some("yes"));
    }

    #[test]
    fn a_bad_line_reports_its_number() {
        let error = parse_log("# comment\n\nnot-a-timestamp x tick\n").unwrap_err();
        assert_eq!(error.line, 3);
    }

    #[test]
    fn unknown_tick_fields_are_rejected() {
        let error = parse_log(
            "1970-01-01T00:00:00.000Z lid-open-plugged tick ui=ok kind=uia title=secret\n",
        )
        .unwrap_err();
        assert!(error.message.contains("unknown field"));
    }
}
