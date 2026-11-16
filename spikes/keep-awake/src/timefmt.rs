//! UTC timestamps with millisecond precision, using the civil calendar.
//!
//! The conversion is Howard Hinnant's public-domain `civil_from_days` /
//! `days_from_civil` algorithm. Logs stay in UTC so a sleep shows up as a
//! jump in the timestamps instead of a clock-change in local time.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

pub fn format_unix_millis(ms: u64) -> String {
    let secs = ms / 1000;
    let millis = (ms % 1000) as u32;
    let days = i64::try_from(secs / 86_400).unwrap_or(i64::MAX);
    let tod = (secs % 86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Parse `YYYY-MM-DDTHH:MM:SS.mmmZ`. Rejects dates that do not round-trip.
pub fn parse_unix_millis(text: &str) -> Option<u64> {
    if text.len() != 24 || !text.ends_with('Z') || text.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    let year: i32 = text.get(0..4)?.parse().ok()?;
    let month: u32 = text.get(5..7)?.parse().ok()?;
    let day: u32 = text.get(8..10)?.parse().ok()?;
    let hour: u32 = text.get(11..13)?.parse().ok()?;
    let minute: u32 = text.get(14..16)?.parse().ok()?;
    let second: u32 = text.get(17..19)?.parse().ok()?;
    let millis: u32 = text.get(20..23)?.parse().ok()?;
    if text.as_bytes().get(4) != Some(&b'-')
        || text.as_bytes().get(7) != Some(&b'-')
        || text.as_bytes().get(13) != Some(&b':')
        || text.as_bytes().get(16) != Some(&b':')
        || text.as_bytes().get(19) != Some(&b'.')
    {
        return None;
    }
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
        || millis > 999
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return None;
    }
    let secs = (days as u64)
        .checked_mul(86_400)?
        .checked_add(u64::from(hour * 3600 + minute * 60 + second))?;
    let ms = secs.checked_mul(1000)?.checked_add(u64::from(millis))?;
    if format_unix_millis(ms) != text {
        return None;
    }
    Some(ms)
}

pub fn format_ms_as_secs(ms: u64) -> String {
    format!("{}.{:03}s", ms / 1000, ms % 1000)
}

fn civil_from_days(mut z: i64) -> (i32, u32, u32) {
    z += 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month as u32, day as u32)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * month_prime + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + u64::from(doy);
    era * 146_097 + doe as i64 - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_utc_instants_round_trip() {
        let cases = [
            (0, "1970-01-01T00:00:00.000Z"),
            (86_400_000, "1970-01-02T00:00:00.000Z"),
            (946_684_800_000, "2000-01-01T00:00:00.000Z"),
            (1_582_934_400_000, "2020-02-29T00:00:00.000Z"),
            (1_709_210_096_123, "2024-02-29T12:34:56.123Z"),
            (1_794_802_740_000, "2026-11-16T04:19:00.000Z"),
        ];
        for (ms, text) in cases {
            assert_eq!(format_unix_millis(ms), text);
            assert_eq!(parse_unix_millis(text), Some(ms));
        }
    }

    #[test]
    fn impossible_dates_are_rejected() {
        assert_eq!(parse_unix_millis("1970-02-31T00:00:00.000Z"), None);
        assert_eq!(parse_unix_millis("1970-01-01T00:00:00Z"), None);
        assert_eq!(parse_unix_millis("1970-01-01 00:00:00.000Z"), None);
    }

    #[test]
    fn durations_keep_millisecond_precision() {
        assert_eq!(format_ms_as_secs(0), "0.000s");
        assert_eq!(format_ms_as_secs(45_000), "45.000s");
        assert_eq!(format_ms_as_secs(30_001), "30.001s");
    }
}
