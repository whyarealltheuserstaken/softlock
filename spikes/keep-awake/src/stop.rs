//! Why a run ended. The log stores the token.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StopReason {
    Key = 1,
    CtrlC = 2,
    Timer = 3,
    BatteryFloor = 4,
    HeatGuard = 5,
    Window = 6,
    Stopped = 7,
}

impl StopReason {
    pub fn from_code(code: u8) -> Self {
        match code {
            1 => StopReason::Key,
            2 => StopReason::CtrlC,
            3 => StopReason::Timer,
            4 => StopReason::BatteryFloor,
            5 => StopReason::HeatGuard,
            6 => StopReason::Window,
            _ => StopReason::Stopped,
        }
    }

    pub fn token(self) -> &'static str {
        match self {
            StopReason::Key => "key",
            StopReason::CtrlC => "ctrl-c",
            StopReason::Timer => "timer",
            StopReason::BatteryFloor => "battery-floor",
            StopReason::HeatGuard => "heat-guard",
            StopReason::Window => "window",
            StopReason::Stopped => "stopped",
        }
    }

    pub fn sentence(self) -> &'static str {
        match self {
            StopReason::Key => "Stopped because a key was pressed.",
            StopReason::CtrlC => "Stopped because Ctrl+C was pressed.",
            StopReason::Timer => "The run finished.",
            StopReason::BatteryFloor => "Stopped because the battery reached 20%.",
            StopReason::HeatGuard => "Stopped because a heat signal crossed the guard.",
            StopReason::Window => "The window loop failed, so the run was stopped.",
            StopReason::Stopped => "The run stopped.",
        }
    }
}
