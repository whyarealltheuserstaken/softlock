//! Low-level keyboard decisions from the input-hook proof. No I/O.

pub const EXTRA_VIS: usize = 0x51F7_0001;
pub const EXTRA_LOAD: usize = 0x51F7_0002;
pub const EXTRA_MARKER: usize = 0x51F7_0003;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookAction {
    PassCount,
    Pass,
    Swallow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HookEvent {
    pub extra: usize,
    pub key_down: bool,
    pub injected: bool,
}

pub fn classify(event: HookEvent) -> HookAction {
    if event.extra == EXTRA_MARKER {
        return HookAction::Swallow;
    }
    if event.extra == EXTRA_VIS && event.key_down {
        return HookAction::PassCount;
    }
    if event.extra == EXTRA_LOAD {
        return HookAction::PassCount;
    }
    HookAction::Pass
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_is_swallowed_and_visibility_keys_are_counted() {
        assert_eq!(
            classify(HookEvent {
                extra: EXTRA_MARKER,
                key_down: true,
                injected: true,
            }),
            HookAction::Swallow
        );
        assert_eq!(
            classify(HookEvent {
                extra: EXTRA_VIS,
                key_down: true,
                injected: true,
            }),
            HookAction::PassCount
        );
        assert_eq!(
            classify(HookEvent {
                extra: EXTRA_LOAD,
                key_down: false,
                injected: true,
            }),
            HookAction::PassCount
        );
        assert_eq!(
            classify(HookEvent {
                extra: 0,
                key_down: true,
                injected: false,
            }),
            HookAction::Pass
        );
    }
}
