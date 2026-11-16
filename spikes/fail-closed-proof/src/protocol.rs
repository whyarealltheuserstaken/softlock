//! Messages exchanged by the user-session probe and the session-0 service.

use serde::{Deserialize, Serialize};

pub const READY_FILE: &str = "ready.json";
pub const COMMAND_FILE: &str = "command.json";
pub const RESULT_FILE: &str = "command-result.json";
pub const WATCHING_FILE: &str = "watching.json";
pub const TIMING_FILE: &str = "timing.json";
pub const REPORT_FILE: &str = "report.txt";
pub const HEARTBEAT_FILE: &str = "heartbeat.log";
pub const HEARTBEAT_STOP: &str = "heartbeat.stop";
pub const JOURNAL_FILE: &str = "journal.json";
pub const BASELINE_FILE: &str = "baseline.json";
pub const ARMED_FILE: &str = "armed.json";

pub const OP_DISCONNECT_CONSOLE: &str = "disconnect-console";
pub const OP_WATCH_USER: &str = "watch-user-disconnect";
pub const OP_STOP: &str = "stop";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Command {
    pub id: u64,
    pub op: String,
    pub session: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandResult {
    pub id: u64,
    pub op: String,
    pub api_ok: bool,
    pub win32: u32,
    pub error: Option<String>,
    pub session: u32,
    pub t0: u64,
    pub t1: u64,
    pub freq: u64,
    pub notification: Option<u32>,
    pub notifications: Vec<u32>,
    pub reconnect_ok: bool,
    pub reconnect: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ready {
    pub session: u32,
    pub pid: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Timing {
    pub t0: u64,
    pub freq: u64,
    pub session: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Beat {
    pub count: u64,
    pub last_ms: u128,
}

pub fn parse_beats(text: &str) -> Beat {
    let mut beat = Beat {
        count: 0,
        last_ms: 0,
    };
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(ms) = parts.next().and_then(|value| value.parse().ok()) else {
            continue;
        };
        let Some(seq) = parts.next().and_then(|value| value.parse().ok()) else {
            continue;
        };
        beat.count = seq;
        beat.last_ms = ms;
    }
    beat
}

/// Longest pause between beats at or after `before`. A reconnect can take longer
/// than one second while the background process keeps writing every 50 ms.
pub fn max_beat_gap(text: &str, before: Beat) -> Option<u128> {
    let mut previous_ms = None;
    let mut previous_seq = 0u64;
    let mut max_gap = None;
    let mut saw_new = false;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(ms) = parts.next().and_then(|value| value.parse::<u128>().ok()) else {
            continue;
        };
        let Some(seq) = parts.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        if seq < before.count {
            continue;
        }
        if seq == before.count {
            previous_ms = Some(ms);
            previous_seq = seq;
            continue;
        }
        saw_new = true;
        if let Some(prev) = previous_ms
            && seq > previous_seq
            && ms >= prev
        {
            let gap = ms - prev;
            max_gap = Some(max_gap.map_or(gap, |old: u128| old.max(gap)));
        }
        previous_ms = Some(ms);
        previous_seq = seq;
    }
    if saw_new { max_gap } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_heartbeat_is_a_gap() {
        let before = parse_beats("1000 1\n1050 2\n");
        let after = "1000 1\n1050 2\n1100 3\n";
        assert_eq!(max_beat_gap(after, before), Some(50));
        assert_eq!(max_beat_gap("1000 1\n1050 2\n", before), None);
        let long = "1000 1\n1050 2\n1100 3\n1150 4\n5000 5\n5050 6\n";
        assert_eq!(max_beat_gap(long, before), Some(3850));
    }
}
