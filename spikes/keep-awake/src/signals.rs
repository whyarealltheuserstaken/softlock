//! Battery percent and the thermal / power-throttling signals Windows exposes.
//!
//! Each progress tick records them so a 60-minute run can be used to choose
//! the default battery and heat floor. The spike only logs the signals. It
//! does not fall back to the real lock.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatteryFlag {
    pub high: bool,
    pub low: bool,
    pub critical: bool,
    pub charging: bool,
    pub no_battery: bool,
}

impl BatteryFlag {
    pub fn from_bits(flag: u8) -> Self {
        Self {
            high: flag & 0x01 != 0,
            low: flag & 0x02 != 0,
            critical: flag & 0x04 != 0,
            charging: flag & 0x08 != 0,
            no_battery: flag & 0x80 != 0,
        }
    }

    pub fn token(self) -> String {
        if self.no_battery {
            return "no-battery".to_string();
        }
        let mut parts = Vec::new();
        if self.charging {
            parts.push("charging");
        }
        if self.critical {
            parts.push("critical");
        }
        if self.low {
            parts.push("low");
        }
        if self.high {
            parts.push("high");
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join("-")
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        if token == "unknown" {
            return None;
        }
        if token == "none" {
            return Some(Self::default());
        }
        if token == "no-battery" {
            return Some(Self {
                no_battery: true,
                ..Self::default()
            });
        }
        let mut flag = Self::default();
        for part in token.split('-') {
            match part {
                "charging" => flag.charging = true,
                "critical" => flag.critical = true,
                "low" => flag.low = true,
                "high" => flag.high = true,
                _ => return None,
            }
        }
        Some(flag)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cooling {
    Active,
    Passive,
    Invalid,
}

impl Cooling {
    pub fn token(self) -> &'static str {
        match self {
            Cooling::Active => "active",
            Cooling::Passive => "passive",
            Cooling::Invalid => "invalid",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "active" => Some(Cooling::Active),
            "passive" => Some(Cooling::Passive),
            "invalid" => Some(Cooling::Invalid),
            _ => None,
        }
    }
}

/// One sample. `None` means that signal was not reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PowerSignals {
    pub battery_percent: Option<u8>,
    pub ac_online: Option<bool>,
    pub battery_saver: Option<bool>,
    pub battery_flag: Option<BatteryFlag>,
    pub cooling: Option<Cooling>,
    pub mhz_max: Option<u32>,
    pub mhz_limit: Option<u32>,
    pub mhz_current: Option<u32>,
    pub execution_throttled: Option<bool>,
    /// The heat sources were queried. `None` fields then mean unavailable.
    pub heat_checked: bool,
    pub passive_limit: Option<u8>,
    pub throttle_reasons: Option<u32>,
    pub zone_dc: Option<i32>,
    pub battery_dc: Option<i32>,
}

impl PowerSignals {
    pub fn is_recorded(self) -> bool {
        self.battery_percent.is_some()
            || self.ac_online.is_some()
            || self.battery_saver.is_some()
            || self.battery_flag.is_some()
            || self.cooling.is_some()
            || self.mhz_max.is_some()
            || self.mhz_limit.is_some()
            || self.mhz_current.is_some()
            || self.execution_throttled.is_some()
            || self.heat_checked
            || self.passive_limit.is_some()
            || self.throttle_reasons.is_some()
            || self.zone_dc.is_some()
            || self.battery_dc.is_some()
    }

    /// Processor speed cap below the maximum. A zero limit is not treated as a cap.
    pub fn speed_limited(self) -> Option<bool> {
        match (self.mhz_max, self.mhz_limit) {
            (Some(max), Some(limit)) if max > 0 && limit > 0 => Some(limit < max),
            _ => None,
        }
    }

    pub fn log_suffix(self) -> String {
        if !self.is_recorded() {
            return String::new();
        }
        let mut text = format!(
            " battery={battery} ac={ac} saver={saver} flag={flag} cooling={cooling} mhz_max={mhz_max} mhz_limit={mhz_limit} mhz_now={mhz_now} throttle={throttle}",
            battery = percent_token(self.battery_percent),
            ac = bool_token(self.ac_online),
            saver = bool_token(self.battery_saver),
            flag = self
                .battery_flag
                .map(BatteryFlag::token)
                .unwrap_or_else(|| "unknown".to_string()),
            cooling = self.cooling.map(Cooling::token).unwrap_or("unknown"),
            mhz_max = mhz_token(self.mhz_max),
            mhz_limit = mhz_token(self.mhz_limit),
            mhz_now = mhz_token(self.mhz_current),
            throttle = bool_token(self.execution_throttled),
        );
        if self.heat_checked
            || self.passive_limit.is_some()
            || self.throttle_reasons.is_some()
            || self.zone_dc.is_some()
            || self.battery_dc.is_some()
        {
            text.push_str(&format!(
                " passive_limit={passive} throttle_reasons={reasons} zone_dc={zone} battery_dc={battery_temp}",
                passive = unavailable_token(self.passive_limit),
                reasons = unavailable_token(self.throttle_reasons),
                zone = unavailable_token(self.zone_dc),
                battery_temp = unavailable_token(self.battery_dc),
            ));
        }
        text
    }

    pub fn progress_note(self) -> String {
        format!(
            "battery={}% cooling={} cap={} heat_zone={} heat_battery={}",
            percent_token(self.battery_percent),
            self.cooling.map(Cooling::token).unwrap_or("unknown"),
            match (self.speed_limited(), self.execution_throttled) {
                (Some(true), _) | (_, Some(true)) => "yes",
                (Some(false), Some(false)) => "no",
                _ => "unknown",
            },
            unavailable_token(self.zone_dc),
            unavailable_token(self.battery_dc),
        )
    }
}

fn percent_token(value: Option<u8>) -> String {
    match value {
        Some(percent) => percent.to_string(),
        None => "unknown".to_string(),
    }
}

fn bool_token(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

fn mhz_token(value: Option<u32>) -> String {
    match value {
        Some(mhz) => mhz.to_string(),
        None => "unknown".to_string(),
    }
}

fn unavailable_token(value: Option<impl ToString>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => "unavailable".to_string(),
    }
}

pub fn parse_percent(token: &str) -> Result<Option<u8>, ()> {
    if token == "unknown" {
        return Ok(None);
    }
    let value: u8 = token.parse().map_err(|_| ())?;
    if value > 100 {
        return Err(());
    }
    Ok(Some(value))
}

pub fn parse_bool_token(token: &str) -> Result<Option<bool>, ()> {
    match token {
        "yes" => Ok(Some(true)),
        "no" => Ok(Some(false)),
        "unknown" => Ok(None),
        _ => Err(()),
    }
}

pub fn parse_mhz(token: &str) -> Result<Option<u32>, ()> {
    if token == "unknown" {
        return Ok(None);
    }
    token.parse().map(Some).map_err(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SignalRollup {
    pub battery_samples: usize,
    pub battery_min: Option<u8>,
    pub battery_max: Option<u8>,
    pub passive_cooling: usize,
    pub speed_limited: usize,
    pub process_throttled: usize,
    pub battery_saver: usize,
    pub battery_critical: usize,
    pub battery_low: usize,
    pub recorded_samples: usize,
    pub heat_samples: usize,
    pub passive_below: usize,
    pub throttle_bit: usize,
    pub hottest_zone_dc: Option<i32>,
    pub hottest_battery_dc: Option<i32>,
}

impl SignalRollup {
    pub fn observe(&mut self, signals: PowerSignals) {
        if !signals.is_recorded() {
            return;
        }
        self.recorded_samples += 1;
        if let Some(percent) = signals.battery_percent {
            self.battery_samples += 1;
            self.battery_min = Some(self.battery_min.map_or(percent, |min| min.min(percent)));
            self.battery_max = Some(self.battery_max.map_or(percent, |max| max.max(percent)));
        }
        if signals.cooling == Some(Cooling::Passive) {
            self.passive_cooling += 1;
        }
        if signals.speed_limited() == Some(true) {
            self.speed_limited += 1;
        }
        if signals.execution_throttled == Some(true) {
            self.process_throttled += 1;
        }
        if signals.battery_saver == Some(true) {
            self.battery_saver += 1;
        }
        if let Some(flag) = signals.battery_flag {
            if flag.critical {
                self.battery_critical += 1;
            }
            if flag.low {
                self.battery_low += 1;
            }
        }
        if signals.heat_checked
            || signals.passive_limit.is_some()
            || signals.throttle_reasons.is_some()
            || signals.zone_dc.is_some()
            || signals.battery_dc.is_some()
        {
            self.heat_samples += 1;
            if signals.passive_limit.is_some_and(|limit| limit < 100) {
                self.passive_below += 1;
            }
            if signals
                .throttle_reasons
                .is_some_and(|reasons| reasons & 1 != 0)
            {
                self.throttle_bit += 1;
            }
            if let Some(zone) = signals.zone_dc {
                self.hottest_zone_dc = Some(self.hottest_zone_dc.map_or(zone, |hot| hot.max(zone)));
            }
            if let Some(battery) = signals.battery_dc {
                self.hottest_battery_dc = Some(
                    self.hottest_battery_dc
                        .map_or(battery, |hot| hot.max(battery)),
                );
            }
        }
    }

    pub fn summary_lines(self) -> String {
        let battery = match self.battery_min {
            Some(min) => format!(
                "lowest battery: {min} (range {min} to {}, {} readings)",
                self.battery_max.unwrap_or(min),
                self.battery_samples
            ),
            None => "lowest battery: not recorded".to_string(),
        };
        let policy = if self.recorded_samples == 0 {
            "policy/cap: not recorded".to_string()
        } else {
            format!(
                "policy/cap: passive cooling {}, cpu speed limit {}, process throttle {}, battery saver {}, battery critical {}, battery low {}",
                self.passive_cooling,
                self.speed_limited,
                self.process_throttled,
                self.battery_saver,
                self.battery_critical,
                self.battery_low
            )
        };
        let heat = if self.heat_samples == 0 {
            "heat: not recorded".to_string()
        } else if self.passive_below == 0
            && self.throttle_bit == 0
            && self.hottest_zone_dc.is_none()
            && self.hottest_battery_dc.is_none()
        {
            "heat: unavailable".to_string()
        } else {
            format!(
                "heat: passive limit below 100 on {} samples, thermal throttle bit on {} samples, hottest zone {}, hottest battery {}",
                self.passive_below,
                self.throttle_bit,
                format_dc(self.hottest_zone_dc),
                format_dc(self.hottest_battery_dc)
            )
        };
        format!("  {battery}\n  {heat}\n  {policy}\n")
    }
}

fn format_dc(tenths: Option<i32>) -> String {
    match tenths {
        None => "not recorded".to_string(),
        Some(tenths) => {
            let whole = tenths / 10;
            let frac = tenths.abs() % 10;
            if frac == 0 {
                format!("{whole} C")
            } else {
                format!("{whole}.{frac} C")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_flags_and_speed_limit_round_trip() {
        let flag = BatteryFlag::from_bits(0x08 | 0x01);
        assert_eq!(flag.token(), "charging-high");
        assert_eq!(BatteryFlag::parse("charging-high"), Some(flag));
        assert_eq!(BatteryFlag::from_bits(0x04).token(), "critical");
        assert_eq!(BatteryFlag::from_bits(0x80).token(), "no-battery");
        assert_eq!(BatteryFlag::parse("nope"), None);

        let limited = PowerSignals {
            mhz_max: Some(2400),
            mhz_limit: Some(800),
            ..PowerSignals::default()
        };
        assert_eq!(limited.speed_limited(), Some(true));
        let open = PowerSignals {
            mhz_max: Some(2400),
            mhz_limit: Some(2400),
            ..PowerSignals::default()
        };
        assert_eq!(open.speed_limited(), Some(false));
        let zero = PowerSignals {
            mhz_max: Some(2400),
            mhz_limit: Some(0),
            ..PowerSignals::default()
        };
        assert_eq!(zero.speed_limited(), None);
    }

    #[test]
    fn rollup_keeps_the_lowest_battery_and_counts_throttle_signals() {
        let mut rollup = SignalRollup::default();
        rollup.observe(PowerSignals {
            battery_percent: Some(80),
            ac_online: Some(true),
            battery_saver: Some(false),
            battery_flag: Some(BatteryFlag::from_bits(0x08)),
            cooling: Some(Cooling::Active),
            mhz_max: Some(2000),
            mhz_limit: Some(2000),
            mhz_current: Some(1900),
            execution_throttled: Some(false),
            ..PowerSignals::default()
        });
        rollup.observe(PowerSignals {
            battery_percent: Some(41),
            ac_online: Some(false),
            battery_saver: Some(true),
            battery_flag: Some(BatteryFlag::from_bits(0x04 | 0x02)),
            cooling: Some(Cooling::Passive),
            mhz_max: Some(2000),
            mhz_limit: Some(900),
            mhz_current: Some(900),
            execution_throttled: Some(true),
            ..PowerSignals::default()
        });
        assert_eq!(rollup.battery_min, Some(41));
        assert_eq!(rollup.battery_max, Some(80));
        assert_eq!(rollup.passive_cooling, 1);
        assert_eq!(rollup.speed_limited, 1);
        assert_eq!(rollup.process_throttled, 1);
        assert_eq!(rollup.battery_saver, 1);
        assert_eq!(rollup.battery_critical, 1);
        assert_eq!(rollup.battery_low, 1);
        let text = rollup.summary_lines();
        assert!(text.contains("lowest battery: 41"));
        assert!(text.contains("passive cooling 1"));
        assert!(text.contains("cpu speed limit 1"));
    }

    #[test]
    fn an_empty_sample_is_not_a_reading() {
        let signals = PowerSignals::default();
        assert!(!signals.is_recorded());
        assert_eq!(signals.log_suffix(), "");
        assert!(
            SignalRollup::default()
                .summary_lines()
                .contains("not recorded")
        );
    }
}
