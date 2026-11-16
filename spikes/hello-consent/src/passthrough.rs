//! Physical-key passthrough while the system credential UI is in front.

use std::time::Duration;

use crate::limit::PASSTHROUGH_TIMEOUT;

/// Virtual-key codes the decision logic needs. Values match Win32.
pub mod vk {
    pub const TAB: u16 = 0x09;
    pub const ESCAPE: u16 = 0x1B;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const Q: u16 = 0x51;
    pub const R: u16 = 0x52;
    pub const A: u16 = 0x41;
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
    /// Deliver the event to the next hook.
    Pass,
    /// Drop the event.
    Swallow,
    /// Drop the event and end the spike. Physical escape only.
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

/// Left and right modifiers tracked separately so releasing one side
/// does not clear the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ModifierKeys {
    lctrl: bool,
    rctrl: bool,
    lalt: bool,
    ralt: bool,
    lshift: bool,
    rshift: bool,
    lwin: bool,
    rwin: bool,
}

impl ModifierKeys {
    pub const fn new() -> Self {
        Self {
            lctrl: false,
            rctrl: false,
            lalt: false,
            ralt: false,
            lshift: false,
            rshift: false,
            lwin: false,
            rwin: false,
        }
    }

    pub fn snapshot(&self) -> Modifiers {
        Modifiers {
            ctrl: self.lctrl || self.rctrl,
            alt: self.lalt || self.ralt,
            shift: self.lshift || self.rshift,
            win: self.lwin || self.rwin,
        }
    }

    pub fn apply(&mut self, vk: u16, down: bool) {
        match vk {
            vk::LCONTROL | vk::CONTROL => self.lctrl = down,
            vk::RCONTROL => self.rctrl = down,
            vk::LMENU | vk::MENU => self.lalt = down,
            vk::RMENU => self.ralt = down,
            vk::LSHIFT | vk::SHIFT => self.lshift = down,
            vk::RSHIFT => self.rshift = down,
            vk::LWIN => self.lwin = down,
            vk::RWIN => self.rwin = down,
            _ => {}
        }
    }
}

/// Clock for one Hello dialog.
///
/// Passthrough starts the first time that dialog's credential UI is in
/// the foreground, and ends when the dialog closes or [`PASSTHROUGH_TIMEOUT`]
/// has elapsed. Leaving the foreground pauses it. The clock keeps running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassthroughWindow {
    timeout: Duration,
    started: Option<Duration>,
}

impl PassthroughWindow {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            started: None,
        }
    }

    pub fn spec() -> Self {
        Self::new(PASSTHROUGH_TIMEOUT)
    }

    /// `dialog_alive` is true until the consent request finishes.
    /// `foreground_ok` is true when the foreground window passed the
    /// image-path and signature check.
    pub fn poll(&mut self, now: Duration, dialog_alive: bool, foreground_ok: bool) -> bool {
        if !dialog_alive {
            self.started = None;
            return false;
        }
        if foreground_ok && self.started.is_none() {
            self.started = Some(now);
        }
        matches!(
            self.started,
            Some(start) if foreground_ok && now.saturating_sub(start) < self.timeout
        )
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

/// Apply one key. Modifier state used for the decision is the state
/// before this key. Injected and physical keys both update modifiers.
pub fn on_key(
    mods: &mut ModifierKeys,
    allow_physical_keys: bool,
    origin: InputOrigin,
    vk: u16,
    down: bool,
) -> Action {
    let before = mods.snapshot();
    let action = decide(
        allow_physical_keys,
        before,
        DeviceEvent::Key { origin, vk, down },
    );
    mods.apply(vk, down);
    action
}

pub fn on_pointer(allow_physical_keys: bool, origin: InputOrigin) -> Action {
    decide(
        allow_physical_keys,
        Modifiers::default(),
        DeviceEvent::Pointer { origin },
    )
}

fn is_escape_key(mods: Modifiers, vk: u16) -> bool {
    vk == vk::Q && mods.ctrl && mods.alt && mods.shift
}

fn is_win_key(vk: u16) -> bool {
    vk == vk::LWIN || vk == vk::RWIN
}

/// Win, Alt+Tab, Alt+Esc, Ctrl+Esc, and Ctrl+Shift+Esc.
/// Any other key while Win is held is swallowed with the Win chord.
fn is_system_chord(mods: Modifiers, vk: u16) -> bool {
    is_win_key(vk)
        || mods.win
        || (mods.alt && vk == vk::TAB)
        || (mods.alt && vk == vk::ESCAPE)
        || (mods.ctrl && vk == vk::ESCAPE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(mods: &mut ModifierKeys, allow: bool, vk: u16) -> Action {
        on_key(mods, allow, InputOrigin::Physical, vk, true)
    }

    fn chord(keys: &[u16]) -> Action {
        let mut mods = ModifierKeys::new();
        let mut last = Action::Pass;
        for vk in keys {
            last = press(&mut mods, true, *vk);
        }
        last
    }

    #[test]
    fn injected_input_always_passes() {
        let mut mods = ModifierKeys::new();
        assert_eq!(
            on_key(&mut mods, false, InputOrigin::Injected, vk::A, true),
            Action::Pass
        );
        assert_eq!(
            on_key(&mut mods, false, InputOrigin::Injected, vk::LWIN, true),
            Action::Pass
        );
        assert_eq!(
            on_key(&mut mods, true, InputOrigin::Injected, vk::TAB, true),
            Action::Pass
        );
        assert_eq!(on_pointer(true, InputOrigin::Injected), Action::Pass);
        assert_eq!(on_pointer(false, InputOrigin::Injected), Action::Pass);
    }

    #[test]
    fn injected_escape_chord_does_not_end_the_spike() {
        let mut mods = ModifierKeys::new();
        for vk in [vk::LCONTROL, vk::LMENU, vk::LSHIFT] {
            assert_eq!(
                on_key(&mut mods, true, InputOrigin::Injected, vk, true),
                Action::Pass
            );
        }
        assert_eq!(
            on_key(&mut mods, true, InputOrigin::Injected, vk::Q, true),
            Action::Pass
        );
    }

    #[test]
    fn physical_keys_and_pointer_are_swallowed_when_passthrough_is_closed() {
        let mut mods = ModifierKeys::new();
        assert_eq!(press(&mut mods, false, vk::DIGIT_1), Action::Swallow);
        assert_eq!(press(&mut mods, false, vk::A), Action::Swallow);
        assert_eq!(on_pointer(false, InputOrigin::Physical), Action::Swallow);
        assert_eq!(on_pointer(true, InputOrigin::Physical), Action::Swallow);
    }

    #[test]
    fn physical_escape_works_while_keys_are_swallowed() {
        let mut mods = ModifierKeys::new();
        assert_eq!(press(&mut mods, false, vk::LCONTROL), Action::Swallow);
        assert_eq!(press(&mut mods, false, vk::LMENU), Action::Swallow);
        assert_eq!(press(&mut mods, false, vk::LSHIFT), Action::Swallow);
        assert_eq!(press(&mut mods, false, vk::Q), Action::Escape);
        assert_eq!(
            on_key(&mut mods, false, InputOrigin::Physical, vk::Q, false),
            Action::Swallow
        );
    }

    #[test]
    fn pin_digit_passes_only_during_passthrough() {
        let mut mods = ModifierKeys::new();
        assert_eq!(press(&mut mods, true, vk::DIGIT_1), Action::Pass);
        assert_eq!(press(&mut mods, true, vk::A), Action::Pass);
    }

    #[test]
    fn system_chords_are_swallowed_during_passthrough() {
        assert_eq!(chord(&[vk::LWIN]), Action::Swallow);
        assert_eq!(chord(&[vk::RWIN]), Action::Swallow);
        assert_eq!(chord(&[vk::LMENU, vk::TAB]), Action::Swallow);
        assert_eq!(chord(&[vk::RMENU, vk::TAB]), Action::Swallow);
        assert_eq!(chord(&[vk::LMENU, vk::ESCAPE]), Action::Swallow);
        assert_eq!(chord(&[vk::LCONTROL, vk::ESCAPE]), Action::Swallow);
        assert_eq!(
            chord(&[vk::LCONTROL, vk::LSHIFT, vk::ESCAPE]),
            Action::Swallow
        );
        assert_eq!(chord(&[vk::LWIN, vk::R]), Action::Swallow);
    }

    #[test]
    fn tab_escape_and_shift_alone_pass_during_passthrough() {
        let mut mods = ModifierKeys::new();
        assert_eq!(press(&mut mods, true, vk::TAB), Action::Pass);
        assert_eq!(press(&mut mods, true, vk::ESCAPE), Action::Pass);
        assert_eq!(press(&mut mods, true, vk::LSHIFT), Action::Pass);
    }

    #[test]
    fn escape_cancels_hello_after_ctrl_is_released() {
        let mut mods = ModifierKeys::new();
        assert_eq!(press(&mut mods, true, vk::LCONTROL), Action::Pass);
        assert_eq!(
            on_key(&mut mods, true, InputOrigin::Physical, vk::LCONTROL, false),
            Action::Pass
        );
        assert_eq!(press(&mut mods, true, vk::ESCAPE), Action::Pass);
    }

    #[test]
    fn releasing_one_shift_key_keeps_shift_down() {
        let mut mods = ModifierKeys::new();
        press(&mut mods, true, vk::LSHIFT);
        press(&mut mods, true, vk::RSHIFT);
        on_key(&mut mods, true, InputOrigin::Physical, vk::LSHIFT, false);
        assert!(mods.snapshot().shift);
        on_key(&mut mods, true, InputOrigin::Physical, vk::RSHIFT, false);
        assert!(!mods.snapshot().shift);
    }

    #[test]
    fn passthrough_ends_at_sixty_seconds_and_when_the_dialog_closes() {
        let mut clock = PassthroughWindow::spec();
        assert!(!clock.poll(Duration::ZERO, true, false));
        assert!(clock.poll(Duration::from_secs(1), true, true));
        assert!(clock.poll(Duration::from_secs(60), true, true));
        assert!(!clock.poll(Duration::from_secs(61), true, true));

        let mut fresh = PassthroughWindow::spec();
        assert!(fresh.poll(Duration::ZERO, true, true));
        assert!(!fresh.poll(Duration::from_secs(10), true, false));
        assert!(fresh.poll(Duration::from_secs(59), true, true));
        assert!(!fresh.poll(Duration::from_secs(60), true, true));

        let mut closed = PassthroughWindow::spec();
        assert!(closed.poll(Duration::from_secs(5), true, true));
        assert!(!closed.poll(Duration::from_secs(6), false, true));
        assert!(closed.poll(Duration::from_secs(70), true, true));
    }

    #[test]
    fn a_key_after_the_timeout_or_dialog_close_reaches_nothing() {
        let mut clock = PassthroughWindow::new(Duration::from_millis(200));
        let mut mods = ModifierKeys::new();
        assert!(clock.poll(Duration::from_millis(0), true, true));
        assert_eq!(
            press(&mut mods, true, vk::DIGIT_1),
            Action::Pass,
            "inside the open window"
        );
        let allow = clock.poll(Duration::from_millis(200), true, true);
        assert!(!allow);
        assert_eq!(press(&mut mods, allow, vk::DIGIT_1), Action::Swallow);

        let mut closed = PassthroughWindow::new(Duration::from_secs(60));
        assert!(closed.poll(Duration::from_secs(1), true, true));
        let allow = closed.poll(Duration::from_secs(2), false, false);
        assert_eq!(
            press(&mut ModifierKeys::new(), allow, vk::A),
            Action::Swallow
        );
    }
}
