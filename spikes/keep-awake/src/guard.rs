//! Safety stops for a run. These are test limits, not the product floor.
//!
//! A battery scenario refuses to start below 80%. Any run stops at 20% battery.
//! Heat uses the thermal-zone counters and temperatures. Cooling mode, a CPU
//! MHz cap, and process execution throttling are power policy, not heat.

use crate::stop::StopReason;

pub const BATTERY_START_PERCENT: u8 = 80;
pub const BATTERY_FLOOR_PERCENT: u8 = 20;
pub const HEAT_PERSIST_MS: u64 = 120_000;
pub const ZONE_LIMIT_DC: i32 = 950;
pub const BATTERY_LIMIT_DC: i32 = 500;

const ZONE_MIN_DECI_K: u32 = 2_732;
const ZONE_MAX_DECI_K: u32 = 4_232;
const BATTERY_MAX_DECI_K: u32 = 3_532;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunGuard {
    hot_since_ms: Option<u64>,
}

impl RunGuard {
    pub fn observe(
        &mut self,
        battery_percent: Option<u8>,
        passive_limit: Option<u8>,
        throttle_reasons: Option<u32>,
        zone_dc: Option<i32>,
        battery_dc: Option<i32>,
        now_ms: u64,
    ) -> Option<StopReason> {
        if battery_percent.is_some_and(|percent| percent <= BATTERY_FLOOR_PERCENT) {
            return Some(StopReason::BatteryFloor);
        }
        if zone_dc.is_some_and(|temp| temp >= ZONE_LIMIT_DC)
            || battery_dc.is_some_and(|temp| temp >= BATTERY_LIMIT_DC)
        {
            return Some(StopReason::HeatGuard);
        }
        if sustained_heat(passive_limit, throttle_reasons) {
            let since = *self.hot_since_ms.get_or_insert(now_ms);
            if now_ms.saturating_sub(since) >= HEAT_PERSIST_MS {
                return Some(StopReason::HeatGuard);
            }
        } else {
            self.hot_since_ms = None;
        }
        None
    }
}

fn sustained_heat(passive_limit: Option<u8>, throttle_reasons: Option<u32>) -> bool {
    passive_limit.is_some_and(|limit| limit < 100)
        || throttle_reasons.is_some_and(|reasons| reasons & 1 != 0)
}

/// Tenths of a degree Celsius, or nothing when the raw tenth-Kelvin value is implausible.
pub fn plausible_zone_dc(deci_kelvin: u32) -> Option<i32> {
    if (ZONE_MIN_DECI_K..=ZONE_MAX_DECI_K).contains(&deci_kelvin) {
        Some(deci_kelvin as i32 - ZONE_MIN_DECI_K as i32)
    } else {
        None
    }
}

pub fn plausible_battery_dc(deci_kelvin: u32) -> Option<i32> {
    if (ZONE_MIN_DECI_K..=BATTERY_MAX_DECI_K).contains(&deci_kelvin) {
        Some(deci_kelvin as i32 - ZONE_MIN_DECI_K as i32)
    } else {
        None
    }
}

pub fn plausible_passive_limit(value: f64) -> Option<u8> {
    if (0.0..=100.0).contains(&value) {
        Some(value.round() as u8)
    } else {
        None
    }
}

pub fn plausible_throttle_reasons(value: i64) -> Option<u32> {
    if (0..=i64::from(u32::MAX)).contains(&value) {
        Some(value as u32)
    } else {
        None
    }
}

pub fn battery_start_refusal(percent: Option<u8>) -> Option<String> {
    match percent {
        Some(percent) if percent >= BATTERY_START_PERCENT => None,
        Some(percent) => Some(format!(
            "This battery test needs the battery at {BATTERY_START_PERCENT}% or more. It is at {percent}%.\n\
             Charge it, then run the command again."
        )),
        None => Some(
            "Could not read the battery percent, so the battery test was not started.\n\
             Charge until Windows shows 80% or more, then run the command again."
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_floor_stops_at_twenty_and_ignores_an_unknown_percent() {
        let mut guard = RunGuard::default();
        assert_eq!(guard.observe(Some(21), None, None, None, None, 0), None);
        assert_eq!(
            guard.observe(Some(20), None, None, None, None, 1),
            Some(StopReason::BatteryFloor)
        );
        assert_eq!(guard.observe(None, None, None, None, None, 2), None);
        assert_eq!(
            guard.observe(Some(0), None, None, None, None, 3),
            Some(StopReason::BatteryFloor)
        );
    }

    #[test]
    fn heat_counters_must_persist_and_a_hot_temperature_stops_at_once() {
        let mut guard = RunGuard::default();
        assert_eq!(guard.observe(Some(90), Some(99), None, None, None, 0), None);
        assert_eq!(
            guard.observe(Some(90), Some(99), Some(1), None, None, HEAT_PERSIST_MS - 1),
            None
        );
        assert_eq!(
            guard.observe(Some(90), Some(100), Some(0), None, None, HEAT_PERSIST_MS),
            None,
            "a cool sample clears the timer"
        );
        assert_eq!(
            guard.observe(Some(90), Some(50), None, None, None, 10_000),
            None
        );
        assert_eq!(
            guard.observe(
                Some(90),
                Some(50),
                None,
                None,
                None,
                10_000 + HEAT_PERSIST_MS
            ),
            Some(StopReason::HeatGuard)
        );

        let mut fresh = RunGuard::default();
        assert_eq!(
            fresh.observe(Some(90), Some(100), Some(0), Some(ZONE_LIMIT_DC), None, 0),
            Some(StopReason::HeatGuard)
        );
        let mut battery = RunGuard::default();
        assert_eq!(
            battery.observe(Some(90), None, None, None, Some(BATTERY_LIMIT_DC - 1), 0),
            None
        );
        assert_eq!(
            battery.observe(Some(90), None, None, None, Some(BATTERY_LIMIT_DC), 1),
            Some(StopReason::HeatGuard)
        );
    }

    #[test]
    fn implausible_temperatures_and_limits_are_dropped() {
        assert_eq!(plausible_zone_dc(0), None);
        assert_eq!(plausible_zone_dc(2_731), None);
        assert_eq!(plausible_zone_dc(2_732), Some(0));
        assert_eq!(plausible_zone_dc(3_682), Some(950));
        assert_eq!(plausible_zone_dc(4_233), None);
        assert_eq!(plausible_battery_dc(3_232), Some(500));
        assert_eq!(plausible_battery_dc(3_533), None);
        assert_eq!(plausible_passive_limit(-1.0), None);
        assert_eq!(plausible_passive_limit(100.0), Some(100));
        assert_eq!(plausible_passive_limit(100.4), None);
        assert_eq!(plausible_passive_limit(99.6), Some(100));
        assert_eq!(plausible_passive_limit(99.4), Some(99));
        assert_eq!(plausible_throttle_reasons(-1), None);
        assert_eq!(plausible_throttle_reasons(1), Some(1));
    }

    #[test]
    fn a_battery_run_needs_eighty_percent_to_start() {
        assert_eq!(battery_start_refusal(Some(80)), None);
        assert!(battery_start_refusal(Some(79)).unwrap().contains("79%"));
        assert!(battery_start_refusal(None).unwrap().contains("80%"));
    }
}
