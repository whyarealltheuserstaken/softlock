//! The four measurement scenarios, plus a one-minute practice and the CI smoke run.

pub const MEASUREMENT_SCENARIOS: [&str; 4] = [
    "lid-open-plugged",
    "lid-open-battery",
    "lid-closed-plugged",
    "lid-closed-battery",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    LidOpenPlugged,
    LidOpenBattery,
    LidClosedPlugged,
    LidClosedBattery,
    Practice,
    Smoke,
}

impl Scenario {
    pub fn id(self) -> &'static str {
        match self {
            Scenario::LidOpenPlugged => "lid-open-plugged",
            Scenario::LidOpenBattery => "lid-open-battery",
            Scenario::LidClosedPlugged => "lid-closed-plugged",
            Scenario::LidClosedBattery => "lid-closed-battery",
            Scenario::Practice => "practice",
            Scenario::Smoke => "smoke",
        }
    }

    pub fn is_measurement(self) -> bool {
        matches!(
            self,
            Scenario::LidOpenPlugged
                | Scenario::LidOpenBattery
                | Scenario::LidClosedPlugged
                | Scenario::LidClosedBattery
        )
    }

    pub fn lid_closed(self) -> bool {
        matches!(
            self,
            Scenario::LidClosedPlugged | Scenario::LidClosedBattery
        )
    }

    /// `Some(true)` means the cord must be unplugged. `None` skips the check.
    pub fn expects_battery(self) -> Option<bool> {
        match self {
            Scenario::LidOpenBattery | Scenario::LidClosedBattery => Some(true),
            Scenario::LidOpenPlugged | Scenario::LidClosedPlugged => Some(false),
            Scenario::Practice | Scenario::Smoke => None,
        }
    }

    pub fn default_minutes(self) -> u64 {
        if self.is_measurement() { 60 } else { 1 }
    }
}

pub fn parse_scenario(id: &str) -> Result<Scenario, String> {
    match id {
        "lid-open-plugged" => Ok(Scenario::LidOpenPlugged),
        "lid-open-battery" => Ok(Scenario::LidOpenBattery),
        "lid-closed-plugged" => Ok(Scenario::LidClosedPlugged),
        "lid-closed-battery" => Ok(Scenario::LidClosedBattery),
        "practice" => Ok(Scenario::Practice),
        "smoke" => Ok(Scenario::Smoke),
        _ => Err(format!(
            "Unknown scenario \"{id}\". Use one of: {}, practice.",
            MEASUREMENT_SCENARIOS.join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurement_matrix_matches_the_four_runs() {
        let cases = [
            ("lid-open-plugged", false, Some(false)),
            ("lid-open-battery", false, Some(true)),
            ("lid-closed-plugged", true, Some(false)),
            ("lid-closed-battery", true, Some(true)),
        ];
        assert_eq!(MEASUREMENT_SCENARIOS, cases.map(|(id, _, _)| id));
        for (id, lid_closed, battery) in cases {
            let scenario = parse_scenario(id).unwrap();
            assert_eq!(scenario.id(), id);
            assert!(scenario.is_measurement());
            assert_eq!(scenario.lid_closed(), lid_closed);
            assert_eq!(scenario.expects_battery(), battery);
            assert_eq!(scenario.default_minutes(), 60);
        }
    }

    #[test]
    fn practice_and_smoke_are_not_measurement_runs() {
        for id in ["practice", "smoke"] {
            let scenario = parse_scenario(id).unwrap();
            assert!(!scenario.is_measurement());
            assert!(!scenario.lid_closed());
            assert_eq!(scenario.expects_battery(), None);
            assert_eq!(scenario.default_minutes(), 1);
        }
    }

    #[test]
    fn unknown_scenario_lists_the_real_ones() {
        let message = parse_scenario("desk").unwrap_err();
        for id in MEASUREMENT_SCENARIOS {
            assert!(message.contains(id), "{message}");
        }
    }
}
