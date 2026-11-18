//! Machine facts the suite is willing to keep. No serials and no hardware ids.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::power::{PowerAvailability, parse_powercfg_a};
use crate::redact::redact;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelloKind {
    Biometric,
    Pin,
    Password,
    Unknown,
}

impl HelloKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Biometric => "biometric",
            Self::Pin => "pin",
            Self::Password => "password",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Console,
    Remote,
    Unknown,
}

impl SessionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Console => "console",
            Self::Remote => "remote",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    pub windows: String,
    pub build: u32,
    pub sku: String,
    pub power: PowerAvailability,
    pub displays: u32,
    pub hello: HelloKind,
    pub session: SessionKind,
    pub elevated: bool,
    pub lid: String,
    pub ac: String,
}

impl Machine {
    pub fn to_json(&self) -> Value {
        json!({
            "windows": self.windows,
            "build": self.build,
            "sku": self.sku,
            "sleep": self.power.sleep_label(),
            "s3": self.power.s3,
            "modern_standby": self.power.modern_standby,
            "hibernate": self.power.hibernate,
            "displays": self.displays,
            "hello": self.hello.as_str(),
            "session": self.session.as_str(),
            "elevated": self.elevated,
            "lid": self.lid,
            "ac": self.ac,
        })
    }
}

#[derive(Debug, Deserialize)]
struct Facts {
    #[serde(default)]
    current_build: String,
    #[serde(default)]
    installation_type: String,
    #[serde(default)]
    powercfg_a: String,
    #[serde(default)]
    monitor_count: u32,
    biometric: Option<bool>,
    pin: Option<bool>,
    #[serde(default)]
    client_protocol: Option<i32>,
    #[serde(default)]
    session: String,
    #[serde(default)]
    elevated: bool,
    chassis_laptop: Option<bool>,
    #[serde(default)]
    ac_line: Option<u8>,
}

pub fn decode_facts(value: &Value) -> Result<Machine, String> {
    let facts: Facts = serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    let build = facts.current_build.trim().parse::<u32>().unwrap_or(0);
    Ok(Machine {
        windows: windows_family(build),
        build,
        sku: sku(&facts.installation_type),
        power: parse_powercfg_a(&facts.powercfg_a),
        displays: facts.monitor_count,
        hello: classify_hello(facts.biometric, facts.pin),
        session: classify_session(facts.client_protocol, &facts.session),
        elevated: facts.elevated,
        lid: lid_label(facts.chassis_laptop),
        ac: ac_label(facts.ac_line),
    })
}

pub fn windows_family(build: u32) -> String {
    if build >= 22000 {
        "11".to_string()
    } else if build >= 10240 {
        "10".to_string()
    } else {
        "unknown".to_string()
    }
}

fn sku(installation_type: &str) -> String {
    let cleaned = redact(installation_type);
    if cleaned.eq_ignore_ascii_case("client") {
        "client".to_string()
    } else if cleaned.eq_ignore_ascii_case("server") {
        "server".to_string()
    } else {
        "unknown".to_string()
    }
}

pub fn classify_hello(biometric: Option<bool>, pin: Option<bool>) -> HelloKind {
    if biometric == Some(true) {
        return HelloKind::Biometric;
    }
    match (biometric, pin) {
        (Some(false), Some(true)) => HelloKind::Pin,
        (Some(false), Some(false)) => HelloKind::Password,
        _ => HelloKind::Unknown,
    }
}

pub fn classify_session(protocol: Option<i32>, label: &str) -> SessionKind {
    match protocol {
        Some(0) => return SessionKind::Console,
        Some(1) | Some(2) => return SessionKind::Remote,
        Some(_) => {}
        None => {}
    }
    if label.eq_ignore_ascii_case("console") {
        SessionKind::Console
    } else if label.eq_ignore_ascii_case("remote") || label.eq_ignore_ascii_case("rdp") {
        SessionKind::Remote
    } else {
        SessionKind::Unknown
    }
}

fn lid_label(chassis_laptop: Option<bool>) -> String {
    match chassis_laptop {
        Some(true) => "present".to_string(),
        Some(false) => "absent".to_string(),
        None => "unknown".to_string(),
    }
}

fn ac_label(ac_line: Option<u8>) -> String {
    match ac_line {
        Some(0) => "on-battery".to_string(),
        Some(1) => "plugged-in".to_string(),
        _ => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_22000_is_windows_11_and_19045_is_windows_10() {
        assert_eq!(windows_family(26100), "11");
        assert_eq!(windows_family(22000), "11");
        assert_eq!(windows_family(19045), "10");
        assert_eq!(windows_family(0), "unknown");
    }

    #[test]
    fn hello_and_session_follow_the_signals_we_keep() {
        assert_eq!(
            classify_hello(Some(true), Some(false)),
            HelloKind::Biometric
        );
        assert_eq!(classify_hello(Some(false), Some(true)), HelloKind::Pin);
        assert_eq!(
            classify_hello(Some(false), Some(false)),
            HelloKind::Password
        );
        assert_eq!(classify_hello(None, None), HelloKind::Unknown);
        assert_eq!(classify_session(Some(0), "rdp"), SessionKind::Console);
        assert_eq!(classify_session(Some(2), ""), SessionKind::Remote);
        assert_eq!(classify_session(Some(-1), ""), SessionKind::Unknown);
        assert_eq!(classify_session(None, "console"), SessionKind::Console);
        assert_eq!(classify_session(None, ""), SessionKind::Unknown);
    }

    #[test]
    fn facts_decode_without_keeping_a_product_string() {
        let machine = decode_facts(&json!({
            "current_build": "19045",
            "installation_type": "Client",
            "powercfg_a": "The following sleep states are available on this system:\n    Standby (S3)\nThe following sleep states are not available on this system:\n    Standby (S0 Low Power Idle)\n",
            "monitor_count": 2,
            "biometric": false,
            "pin": false,
            "client_protocol": 0,
            "elevated": false,
            "chassis_laptop": false,
            "ac_line": 1,
            "serial": "SHOULD-NOT-APPEAR"
        }))
        .unwrap();
        assert_eq!(machine.windows, "10");
        assert_eq!(machine.build, 19045);
        assert_eq!(machine.power.sleep_label(), "s3");
        assert_eq!(machine.displays, 2);
        assert_eq!(machine.hello, HelloKind::Password);
        assert_eq!(machine.lid, "absent");
        assert_eq!(machine.ac, "plugged-in");
        let encoded = machine.to_json().to_string();
        assert!(!encoded.contains("SHOULD-NOT-APPEAR"));
        assert!(!encoded.to_ascii_lowercase().contains("serial"));
    }
}
