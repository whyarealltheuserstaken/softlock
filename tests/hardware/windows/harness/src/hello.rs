//! Hello passthrough decision from the hello-consent spike. No I/O.

use std::time::Duration;

pub const PASSTHROUGH_TIMEOUT: Duration = Duration::from_secs(60);

#[allow(dead_code)]
pub mod vk {
    pub const TAB: u16 = 0x09;
    pub const ESCAPE: u16 = 0x1B;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const Q: u16 = 0x51;
    pub const DIGIT_1: u16 = 0x31;
    pub const LSHIFT: u16 = 0xA0;
    pub const RSHIFT: u16 = 0xA1;
    pub const LCONTROL: u16 = 0xA2;
    pub const RCONTROL: u16 = 0xA3;
    pub const LMENU: u16 = 0xA4;
    pub const RMENU: u16 = 0xA5;
    pub const LWIN: u16 = 0x5B;
    pub const RWIN: u16 = 0x5C;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputOrigin {
    Physical,
    Injected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Pass,
    Swallow,
    Escape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceEvent {
    Key {
        origin: InputOrigin,
        vk: u16,
        down: bool,
    },
    Pointer {
        origin: InputOrigin,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassthroughWindow {
    timeout: Duration,
    started: Option<Duration>,
}

impl PassthroughWindow {
    pub fn spec() -> Self {
        Self {
            timeout: PASSTHROUGH_TIMEOUT,
            started: None,
        }
    }

    pub fn poll(&mut self, now: Duration, foreground_ok: bool) -> bool {
        if foreground_ok && self.started.is_none() {
            self.started = Some(now);
        }
        matches!(
            self.started,
            Some(start) if foreground_ok && now.saturating_sub(start) < self.timeout
        )
    }

    pub fn close(&mut self) {
        self.started = None;
    }
}

pub fn decide(allow_physical_keys: bool, mods: Modifiers, event: DeviceEvent) -> Action {
    match event {
        DeviceEvent::Key {
            origin: InputOrigin::Injected,
            ..
        }
        | DeviceEvent::Pointer {
            origin: InputOrigin::Injected,
        } => Action::Pass,
        DeviceEvent::Pointer {
            origin: InputOrigin::Physical,
        } => Action::Swallow,
        DeviceEvent::Key {
            origin: InputOrigin::Physical,
            vk,
            down,
        } => {
            if is_escape_key(mods, vk) {
                return if down {
                    Action::Escape
                } else {
                    Action::Swallow
                };
            }
            if !allow_physical_keys || is_system_chord(mods, vk) {
                Action::Swallow
            } else {
                Action::Pass
            }
        }
    }
}

fn is_escape_key(mods: Modifiers, vk: u16) -> bool {
    vk == vk::Q && mods.ctrl && mods.alt && mods.shift
}

fn is_system_chord(mods: Modifiers, vk: u16) -> bool {
    vk == vk::LWIN
        || vk == vk::RWIN
        || mods.win
        || (mods.alt && vk == vk::TAB)
        || (mods.alt && vk == vk::ESCAPE)
        || (mods.ctrl && vk == vk::ESCAPE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, alt: bool, shift: bool, win: bool) -> Modifiers {
        Modifiers {
            ctrl,
            alt,
            shift,
            win,
        }
    }

    #[test]
    fn injected_input_passes_and_physical_input_is_swallowed_until_passthrough() {
        assert_eq!(
            decide(
                false,
                Modifiers::default(),
                DeviceEvent::Key {
                    origin: InputOrigin::Injected,
                    vk: vk::DIGIT_1,
                    down: true,
                }
            ),
            Action::Pass
        );
        assert_eq!(
            decide(
                false,
                Modifiers::default(),
                DeviceEvent::Key {
                    origin: InputOrigin::Physical,
                    vk: vk::DIGIT_1,
                    down: true,
                }
            ),
            Action::Swallow
        );
        assert_eq!(
            decide(
                true,
                Modifiers::default(),
                DeviceEvent::Key {
                    origin: InputOrigin::Physical,
                    vk: vk::DIGIT_1,
                    down: true,
                }
            ),
            Action::Pass
        );
        assert_eq!(
            decide(
                true,
                Modifiers::default(),
                DeviceEvent::Pointer {
                    origin: InputOrigin::Physical,
                }
            ),
            Action::Swallow
        );
    }

    #[test]
    fn system_chords_stay_swallowed_and_the_escape_chord_ends_the_run() {
        assert_eq!(
            decide(
                true,
                mods(false, false, false, true),
                DeviceEvent::Key {
                    origin: InputOrigin::Physical,
                    vk: vk::LWIN,
                    down: true,
                }
            ),
            Action::Swallow
        );
        assert_eq!(
            decide(
                true,
                mods(false, true, false, false),
                DeviceEvent::Key {
                    origin: InputOrigin::Physical,
                    vk: vk::TAB,
                    down: true,
                }
            ),
            Action::Swallow
        );
        assert_eq!(
            decide(
                false,
                mods(true, true, true, false),
                DeviceEvent::Key {
                    origin: InputOrigin::Physical,
                    vk: vk::Q,
                    down: true,
                }
            ),
            Action::Escape
        );
        assert_ne!(vk::SHIFT, vk::LSHIFT);
        assert_ne!(vk::CONTROL, vk::LCONTROL);
        assert_ne!(vk::MENU, vk::LMENU);
        assert_ne!(vk::RSHIFT, vk::RCONTROL);
        assert_ne!(vk::RMENU, vk::RWIN);
    }

    #[test]
    fn passthrough_ends_at_sixty_seconds_and_when_the_dialog_closes() {
        let mut clock = PassthroughWindow::spec();
        assert!(!clock.poll(Duration::ZERO, false));
        assert!(clock.poll(Duration::from_secs(1), true));
        assert!(clock.poll(Duration::from_secs(60), true));
        assert!(!clock.poll(Duration::from_secs(61), true));
        clock.close();
        assert!(!clock.poll(Duration::from_secs(62), false));
        assert!(clock.poll(Duration::from_secs(70), true));
    }
}
