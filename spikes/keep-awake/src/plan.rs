//! What the spike holds, and when a long run should refuse to start.
//!
//! Issue 20 ships `PowerRequestSystemRequired` plus
//! `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`, and adds
//! `PowerRequestDisplayRequired` on Modern Standby when keep-display is auto.
//! This spike always holds that same trio, including the display request, so
//! the four runs measure the combination those tickets depend on.
//! `ES_DISPLAY_REQUIRED` is deliberately absent: the display stay-awake is the
//! power request, which is the call the spec names for Modern Standby.

use crate::scenario::Scenario;

/// Printed when a lid-closed run is ready for the lid to shut.
pub const CLOSE_LID_LINE: &str = "You can close the lid now.";

pub const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
pub const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;
pub const ES_CONTINUOUS: u32 = 0x8000_0000;

pub const EX_TOPMOST: u32 = 0x0000_0008;
pub const EX_TRANSPARENT: u32 = 0x0000_0020;
pub const EX_TOOLWINDOW: u32 = 0x0000_0080;
pub const EX_LAYERED: u32 = 0x0008_0000;
pub const EX_NOACTIVATE: u32 = 0x0800_0000;
pub const POPUP_STYLE: u32 = 0x8000_0000;

pub const KEY_DOWN_MESSAGE: u32 = 0x0100;
pub const SYSKEY_DOWN_MESSAGE: u32 = 0x0104;

pub const DIM_ALPHA: u8 = 180;
pub const BLACK_ALPHA: u8 = 255;

/// A wait longer than this, between two log lines, counts as a gap.
pub const GAP_LIMIT_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerPlan {
    pub system_required: bool,
    pub display_required: bool,
    pub execution_state: u32,
}

pub fn spike_power_plan() -> PowerPlan {
    PowerPlan {
        system_required: true,
        display_required: true,
        execution_state: ES_CONTINUOUS | ES_SYSTEM_REQUIRED,
    }
}

pub fn requests_label(plan: &PowerPlan) -> &'static str {
    match (plan.system_required, plan.display_required) {
        (true, true) => "system-required+display-required",
        (true, false) => "system-required",
        (false, true) => "display-required",
        (false, false) => "none",
    }
}

pub fn format_execution_state(flags: u32) -> String {
    let mut parts = Vec::new();
    if flags & ES_CONTINUOUS != 0 {
        parts.push("ES_CONTINUOUS");
    }
    if flags & ES_SYSTEM_REQUIRED != 0 {
        parts.push("ES_SYSTEM_REQUIRED");
    }
    if flags & ES_DISPLAY_REQUIRED != 0 {
        parts.push("ES_DISPLAY_REQUIRED");
    }
    let known = ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED;
    let extra = flags & !known;
    if parts.is_empty() && extra == 0 {
        return "none".to_string();
    }
    let mut text = parts.join("|");
    if extra != 0 {
        if !text.is_empty() {
            text.push('|');
        }
        text.push_str(&format!("0x{extra:08X}"));
    }
    text
}

/// Click-through, no focus steal, layered so it can be dim, topmost, no taskbar button.
pub fn overlay_ex_style() -> u32 {
    EX_TOPMOST | EX_TRANSPARENT | EX_TOOLWINDOW | EX_LAYERED | EX_NOACTIVATE
}

pub fn overlay_style() -> u32 {
    POPUP_STYLE
}

pub fn key_message_stops_run(message: u32) -> bool {
    message == KEY_DOWN_MESSAGE || message == SYSKEY_DOWN_MESSAGE
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayLook {
    Dim,
    Black,
}

impl OverlayLook {
    pub fn alpha(self) -> u8 {
        match self {
            OverlayLook::Dim => DIM_ALPHA,
            OverlayLook::Black => BLACK_ALPHA,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OverlayLook::Dim => "dim",
            OverlayLook::Black => "black",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSource {
    PluggedIn,
    OnBattery,
    Unknown,
}

impl PowerSource {
    pub fn log_value(self) -> &'static str {
        match self {
            PowerSource::PluggedIn => "plugged-in",
            PowerSource::OnBattery => "on-battery",
            PowerSource::Unknown => "unknown",
        }
    }

    pub fn sentence(self) -> &'static str {
        match self {
            PowerSource::PluggedIn => "plugged in",
            PowerSource::OnBattery => "on battery",
            PowerSource::Unknown => "unknown",
        }
    }
}

pub fn classify_ac_line(status: u8) -> PowerSource {
    match status {
        0 => PowerSource::OnBattery,
        1 => PowerSource::PluggedIn,
        _ => PowerSource::Unknown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceCheck {
    Match,
    Unchecked,
    WrongSource,
    UnknownSource,
}

pub fn check_source(expects_battery: Option<bool>, source: PowerSource) -> SourceCheck {
    let Some(expects_battery) = expects_battery else {
        return SourceCheck::Unchecked;
    };
    match (expects_battery, source) {
        (true, PowerSource::OnBattery) | (false, PowerSource::PluggedIn) => SourceCheck::Match,
        (_, PowerSource::Unknown) => SourceCheck::UnknownSource,
        _ => SourceCheck::WrongSource,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AoacGate {
    Proceed,
    NotModernStandby,
    Unknown,
}

pub fn aoac_gate(required: bool, aoac: Option<bool>) -> AoacGate {
    if !required {
        return AoacGate::Proceed;
    }
    match aoac {
        Some(true) => AoacGate::Proceed,
        Some(false) => AoacGate::NotModernStandby,
        None => AoacGate::Unknown,
    }
}

pub fn format_lid_action(index: Option<u32>) -> String {
    match index {
        None => "unknown".to_string(),
        Some(0) => "do nothing".to_string(),
        Some(1) => "sleep".to_string(),
        Some(2) => "hibernate".to_string(),
        Some(3) => "shut down".to_string(),
        Some(other) => format!("other ({other})"),
    }
}

pub fn format_battery_percent(value: Option<u8>) -> String {
    match value {
        None | Some(255) => "unknown".to_string(),
        Some(percent) if percent <= 100 => percent.to_string(),
        Some(other) => format!("other ({other})"),
    }
}

pub fn format_yes_no(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

pub fn format_duration(secs: u64) -> String {
    if secs >= 60 && secs.is_multiple_of(60) {
        let minutes = secs / 60;
        if minutes == 1 {
            "1 minute".to_string()
        } else {
            format!("{minutes} minutes")
        }
    } else if secs == 1 {
        "1 second".to_string()
    } else {
        format!("{secs} seconds")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineView {
    pub aoac: Option<bool>,
    pub lid_present: Option<bool>,
    pub power_source: PowerSource,
    pub battery_percent: Option<u8>,
    pub lid_action_ac: Option<u32>,
    pub lid_action_dc: Option<u32>,
}

impl MachineView {
    pub fn unknown() -> Self {
        Self {
            aoac: None,
            lid_present: None,
            power_source: PowerSource::Unknown,
            battery_percent: None,
            lid_action_ac: None,
            lid_action_dc: None,
        }
    }
}

/// Why a measurement run must not start. Practice and smoke are never refused.
pub fn refuse_reason(
    scenario: Scenario,
    machine: &MachineView,
    allow_no_modern_standby: bool,
) -> Option<String> {
    if !scenario.is_measurement() {
        return None;
    }
    if !allow_no_modern_standby {
        match aoac_gate(true, machine.aoac) {
            AoacGate::Proceed => {}
            AoacGate::NotModernStandby => {
                return Some(
                    "This computer did not report Modern Standby (AoAc).\n\
                     The 60 minute tests would not answer the question, so this run was not started.\n\
                     Write \"no\" in the finding, with the laptop model. The practice scenario can still run."
                        .to_string(),
                );
            }
            AoacGate::Unknown => {
                return Some(
                    "Could not read Modern Standby (AoAc), so the long test was not started."
                        .to_string(),
                );
            }
        }
    }
    if scenario.lid_closed() && machine.lid_present != Some(true) {
        return Some(
            "Windows says this computer has no lid. The lid-closed test was not started."
                .to_string(),
        );
    }
    match check_source(scenario.expects_battery(), machine.power_source) {
        SourceCheck::Match | SourceCheck::Unchecked => {}
        SourceCheck::UnknownSource => {
            return Some(
                "Windows did not say whether the power cord is plugged in, so the long test was not started."
                    .to_string(),
            );
        }
        SourceCheck::WrongSource => return Some(wrong_cord_message(scenario)),
    }
    if scenario.expects_battery() == Some(true) {
        return crate::guard::battery_start_refusal(machine.battery_percent);
    }
    None
}

fn wrong_cord_message(scenario: Scenario) -> String {
    if scenario.expects_battery() == Some(true) {
        "This test needs the computer on battery, and Windows says the power cord is plugged in.\n\
         Unplug the cord, wait until Windows shows it is on battery, and run the command again."
            .to_string()
    } else {
        "This test needs the power cord plugged in, and Windows says the computer is on battery.\n\
         Plug in the cord and run the command again."
            .to_string()
    }
}

pub fn status_report(machine: &MachineView) -> String {
    format!(
        "Modern Standby (AoAc): {aoac}\n\
         Lid sensor: {lid}\n\
         Power cord: {cord}\n\
         Battery: {battery}\n\
         Lid close action on power: {ac}\n\
         Lid close action on battery: {dc}",
        aoac = format_yes_no(machine.aoac),
        lid = if machine.lid_present == Some(true) {
            "present"
        } else if machine.lid_present == Some(false) {
            "not present"
        } else {
            "unknown"
        },
        cord = machine.power_source.sentence(),
        battery = format_battery_percent(machine.battery_percent),
        ac = format_lid_action(machine.lid_action_ac),
        dc = format_lid_action(machine.lid_action_dc),
    )
}

pub fn holding_lines(plan: &PowerPlan) -> String {
    format!(
        "Holding a system-required power request and a display-required power request.\n\
         Holding SetThreadExecutionState ({}).",
        format_execution_state(plan.execution_state)
    )
}

pub fn ready_line(scenario: Scenario, duration_secs: u64) -> String {
    let length = format_duration(duration_secs);
    if scenario.lid_closed() {
        format!(
            "{CLOSE_LID_LINE} The run stops after {length}. To stop early, open the lid and press any key."
        )
    } else {
        format!("Leave the lid open. This run lasts {length}. Press a key only to stop early.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spike_holds_the_spec_combination_and_not_display_execution_state() {
        let plan = spike_power_plan();
        assert!(plan.system_required);
        assert!(plan.display_required);
        assert_eq!(plan.execution_state, ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
        assert_eq!(plan.execution_state & ES_DISPLAY_REQUIRED, 0);
        assert_eq!(requests_label(&plan), "system-required+display-required");
        assert_eq!(
            format_execution_state(plan.execution_state),
            "ES_CONTINUOUS|ES_SYSTEM_REQUIRED"
        );
        assert!(!holding_lines(&plan).contains("ES_DISPLAY_REQUIRED"));
        assert!(holding_lines(&plan).contains("system-required"));
        assert!(holding_lines(&plan).contains("display-required"));
    }

    #[test]
    fn overlay_does_not_block_input_or_take_focus() {
        let style = overlay_ex_style();
        assert_ne!(style & EX_TRANSPARENT, 0, "mouse clicks fall through");
        assert_ne!(style & EX_NOACTIVATE, 0, "the overlay does not take focus");
        assert_ne!(style & EX_LAYERED, 0, "the overlay can be dim");
        assert_ne!(style & EX_TOPMOST, 0);
        assert_ne!(style & EX_TOOLWINDOW, 0);
        assert_eq!(overlay_style(), POPUP_STYLE);
        assert_eq!(OverlayLook::Dim.alpha(), 180);
        assert_eq!(OverlayLook::Black.alpha(), 255);
        assert_eq!(OverlayLook::Dim.label(), "dim");
    }

    #[test]
    fn only_key_down_stops_the_run() {
        assert!(key_message_stops_run(KEY_DOWN_MESSAGE));
        assert!(key_message_stops_run(SYSKEY_DOWN_MESSAGE));
        assert!(!key_message_stops_run(0x0101), "key-up does not stop");
        assert!(
            !key_message_stops_run(0x0201),
            "a mouse button does not stop"
        );
    }

    #[test]
    fn ac_line_and_source_check() {
        assert_eq!(classify_ac_line(0), PowerSource::OnBattery);
        assert_eq!(classify_ac_line(1), PowerSource::PluggedIn);
        assert_eq!(classify_ac_line(255), PowerSource::Unknown);
        assert_eq!(
            check_source(Some(false), PowerSource::PluggedIn),
            SourceCheck::Match
        );
        assert_eq!(
            check_source(Some(true), PowerSource::OnBattery),
            SourceCheck::Match
        );
        assert_eq!(
            check_source(Some(false), PowerSource::OnBattery),
            SourceCheck::WrongSource
        );
        assert_eq!(
            check_source(Some(true), PowerSource::Unknown),
            SourceCheck::UnknownSource
        );
        assert_eq!(
            check_source(None, PowerSource::Unknown),
            SourceCheck::Unchecked
        );
    }

    #[test]
    fn aoac_gate_and_lid_names() {
        assert_eq!(aoac_gate(false, Some(false)), AoacGate::Proceed);
        assert_eq!(aoac_gate(true, Some(true)), AoacGate::Proceed);
        assert_eq!(aoac_gate(true, Some(false)), AoacGate::NotModernStandby);
        assert_eq!(aoac_gate(true, None), AoacGate::Unknown);
        assert_eq!(format_lid_action(Some(0)), "do nothing");
        assert_eq!(format_lid_action(Some(1)), "sleep");
        assert_eq!(format_lid_action(Some(2)), "hibernate");
        assert_eq!(format_lid_action(Some(3)), "shut down");
        assert_eq!(format_lid_action(Some(9)), "other (9)");
        assert_eq!(format_lid_action(None), "unknown");
        assert_eq!(format_battery_percent(Some(80)), "80");
        assert_eq!(format_battery_percent(Some(255)), "unknown");
        assert_eq!(format_battery_percent(Some(101)), "other (101)");
        assert_eq!(format_battery_percent(None), "unknown");
    }

    #[test]
    fn long_runs_refuse_without_modern_standby_a_lid_or_the_right_cord() {
        let mut machine = MachineView {
            aoac: Some(true),
            lid_present: Some(true),
            power_source: PowerSource::PluggedIn,
            battery_percent: Some(100),
            lid_action_ac: Some(1),
            lid_action_dc: Some(1),
        };
        assert_eq!(
            refuse_reason(Scenario::LidOpenPlugged, &machine, false),
            None
        );

        machine.aoac = Some(false);
        let refused = refuse_reason(Scenario::LidOpenPlugged, &machine, false).unwrap();
        assert!(refused.contains("did not report Modern Standby"));
        assert_eq!(
            refuse_reason(Scenario::Practice, &machine, false),
            None,
            "practice still runs so the report can be read"
        );
        assert_eq!(
            refuse_reason(Scenario::LidOpenPlugged, &machine, true),
            None,
            "the override is explicit"
        );

        machine.aoac = None;
        assert!(
            refuse_reason(Scenario::LidOpenPlugged, &machine, false)
                .unwrap()
                .contains("Could not read")
        );

        machine.aoac = Some(true);
        machine.lid_present = Some(false);
        assert!(
            refuse_reason(Scenario::LidClosedPlugged, &machine, false)
                .unwrap()
                .contains("no lid")
        );
        assert_eq!(
            refuse_reason(Scenario::LidOpenPlugged, &machine, false),
            None
        );

        machine.lid_present = Some(true);
        machine.power_source = PowerSource::OnBattery;
        assert!(
            refuse_reason(Scenario::LidOpenPlugged, &machine, false)
                .unwrap()
                .contains("Plug in the cord")
        );
        machine.power_source = PowerSource::PluggedIn;
        assert!(
            refuse_reason(Scenario::LidOpenBattery, &machine, false)
                .unwrap()
                .contains("Unplug the cord")
        );
        machine.power_source = PowerSource::Unknown;
        assert!(
            refuse_reason(Scenario::LidClosedBattery, &machine, false)
                .unwrap()
                .contains("did not say")
        );
        machine.power_source = PowerSource::OnBattery;
        machine.battery_percent = Some(100);
        assert_eq!(
            refuse_reason(Scenario::LidOpenBattery, &machine, false),
            None
        );
        machine.battery_percent = Some(79);
        assert!(
            refuse_reason(Scenario::LidClosedBattery, &machine, false)
                .unwrap()
                .contains("79%")
        );
        machine.battery_percent = None;
        assert!(
            refuse_reason(Scenario::LidOpenBattery, &machine, false)
                .unwrap()
                .contains("80%")
        );
    }

    #[test]
    fn status_and_ready_lines_use_plain_words() {
        let machine = MachineView {
            aoac: Some(false),
            lid_present: Some(true),
            power_source: PowerSource::PluggedIn,
            battery_percent: Some(100),
            lid_action_ac: Some(1),
            lid_action_dc: Some(0),
        };
        let status = status_report(&machine);
        assert!(status.contains("Modern Standby (AoAc): no"));
        assert!(status.contains("Lid close action on power: sleep"));
        assert!(status.contains("Lid close action on battery: do nothing"));
        assert!(ready_line(Scenario::LidClosedPlugged, 3600).contains(CLOSE_LID_LINE));
        assert!(ready_line(Scenario::LidOpenPlugged, 3600).contains("Leave the lid open"));
        assert_eq!(format_duration(3600), "60 minutes");
        assert_eq!(format_duration(60), "1 minute");
        assert_eq!(format_duration(6), "6 seconds");
    }
}
