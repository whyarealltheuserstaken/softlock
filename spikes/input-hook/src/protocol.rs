//! Line-delimited JSON between the supervisor and the hook processes.

use std::io::{BufRead, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[cfg_attr(not(windows), allow(dead_code))]
pub const ROLE_MEDIUM: &str = "medium";
#[cfg_attr(not(windows), allow(dead_code))]
pub const ROLE_ELEVATED: &str = "elevated";
pub const ROLE_SYSTEM: &str = "system";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub role: String,
    pub pid: u32,
    pub il: u32,
    pub user: String,
    pub session: u32,
    pub station: String,
    pub desktop: String,
    pub timeout_ms: u64,
    pub timeout_raw: String,
    pub hwnd: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd")]
pub enum Command {
    #[serde(rename = "foreground")]
    Foreground,
    #[serde(rename = "reset")]
    Reset,
    #[serde(rename = "counts")]
    Counts,
    #[serde(rename = "slow")]
    Slow { timeout_ms: u64 },
    #[serde(rename = "quit")]
    Quit,
    #[serde(rename = "disarm")]
    Disarm,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply")]
pub enum Reply {
    #[serde(rename = "foreground")]
    Foreground {
        ok: bool,
        rect: Option<Rect>,
        error: Option<String>,
    },
    #[serde(rename = "reset")]
    Reset,
    #[serde(rename = "counts")]
    Counts {
        vis_seen: u64,
        vis_injected: u64,
        load_seen: u64,
        marker_seen: u64,
        window_keys: u64,
        slow_entered: u32,
        slow_returned: u32,
    },
    /// The system hook armed the removal probe. The supervisor injects the
    /// marker from outside that process; a same-process `SendInput` does not
    /// trip `LowLevelHooksTimeout`.
    #[serde(rename = "slow")]
    Slow {
        healthy_before: bool,
        sleep_ms: u64,
        marker_seen: u64,
        error: Option<String>,
    },
    #[serde(rename = "bye")]
    Bye,
    #[serde(rename = "disarm")]
    Disarm,
}

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchChild {
    pub role: String,
    pub pid: Option<u32>,
    pub token: String,
    pub il: Option<u32>,
    pub error: Option<String>,
}

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchReport {
    pub console_session: u32,
    pub user_il: u32,
    pub elevation_type: i32,
    pub linked_il: Option<u32>,
    pub children: Vec<LaunchChild>,
    pub notes: Vec<String>,
    pub error: Option<String>,
}

pub fn write_msg(writer: &mut impl Write, msg: &impl Serialize) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, msg).map_err(|error| error.to_string())?;
    writer
        .write_all(b"\n")
        .map_err(|error| format!("write message: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("flush message: {error}"))
}

pub fn read_msg<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<T, String> {
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .map_err(|error| format!("read message: {error}"))?;
    if read == 0 {
        return Err("connection closed".to_string());
    }
    serde_json::from_str(line.trim()).map_err(|error| format!("{error}: {}", line.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_and_reply_round_trip_as_one_json_line() {
        let command = Command::Slow { timeout_ms: 300 };
        let mut bytes = Vec::new();
        write_msg(&mut bytes, &command).unwrap();
        assert!(bytes.ends_with(b"\n"));
        assert!(!bytes[..bytes.len() - 1].contains(&b'\n'));
        let parsed: Command = read_msg(&mut bytes.as_slice()).unwrap();
        assert_eq!(parsed, command);

        let reply = Reply::Counts {
            vis_seen: 20,
            vis_injected: 20,
            load_seen: 40_000,
            marker_seen: 1,
            window_keys: 20,
            slow_entered: 0,
            slow_returned: 0,
        };
        let mut bytes = Vec::new();
        write_msg(&mut bytes, &reply).unwrap();
        let parsed: Reply = read_msg(&mut bytes.as_slice()).unwrap();
        assert_eq!(parsed, reply);
    }

    #[test]
    fn hello_keeps_the_integrity_and_the_timeout_raw_value() {
        let hello = Hello {
            role: ROLE_SYSTEM.to_string(),
            pid: 10,
            il: 0x4000,
            user: "SYSTEM".to_string(),
            session: 2,
            station: "WinSta0".to_string(),
            desktop: "Default".to_string(),
            timeout_ms: 300,
            timeout_raw: "absent".to_string(),
            hwnd: 0,
        };
        let mut bytes = Vec::new();
        write_msg(&mut bytes, &hello).unwrap();
        let parsed: Hello = read_msg(&mut bytes.as_slice()).unwrap();
        assert_eq!(parsed, hello);
    }
}
