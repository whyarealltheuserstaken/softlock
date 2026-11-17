//! Which lock path to arm. A pure function of three facts.
//!
//! The facts are whether `dlsym` returned `SACLockScreenImmediate`, whether
//! that call succeeded, and whether the documented screen-saver path can
//! require the account password with a zero delay. This module does not call
//! the symbol and does not start the screen saver.

use softlock_core::PRODUCT_DISPLAY_NAME;

/// `dlopen` path for the private framework. The binary must not link it.
pub const LOGIN_FRAMEWORK_IMAGE: &str =
    "/System/Library/PrivateFrameworks/login.framework/Versions/Current/login";

/// Private lock symbol. A missing symbol is a recorded miss, not a crash.
pub const PRIVATE_LOCK_SYMBOL: &str = "SACLockScreenImmediate";

/// Documented path: the screen-saver engine, not the login window.
pub const SCREEN_SAVER_ENGINE_CANDIDATES: &[&str] = &[
    "/System/Library/Frameworks/ScreenSaver.framework/Resources/ScreenSaverEngine.app/Contents/MacOS/ScreenSaverEngine",
    "/System/Library/CoreServices/ScreenSaverEngine.app/Contents/MacOS/ScreenSaverEngine",
];

/// Arm-time choice. There is no Unlocked variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockArm {
    /// The documented screen-saver path is the one requested.
    DocumentedPath,
    /// The private call succeeded. The documented path is not requested.
    PrivateCall,
    /// Refuse to arm. This is not Unlocked.
    RefuseToArm,
}

impl LockArm {
    pub const fn requests_documented_path(self) -> bool {
        matches!(self, Self::DocumentedPath)
    }

    pub const fn arms(self) -> bool {
        matches!(self, Self::DocumentedPath | Self::PrivateCall)
    }

    /// The chooser never reports Unlocked. A failure stays refused.
    pub const fn reports_unlocked(self) -> bool {
        false
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DocumentedPath => "documented",
            Self::PrivateCall => "private",
            Self::RefuseToArm => "refuse",
        }
    }
}

/// What to do after an error once an exercise has started.
///
/// A path that already failed is not chosen again. When no safe path remains,
/// the end is [`FailClosedEnd::NeverUnlocked`]. The kit does not unlock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailClosedEnd {
    LockPrivate,
    LockDocumented,
    NeverUnlocked,
}

impl FailClosedEnd {
    pub const fn locks_the_os(self) -> bool {
        matches!(self, Self::LockPrivate | Self::LockDocumented)
    }

    pub const fn reports_unlocked(self) -> bool {
        false
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LockPrivate => "lock-private",
            Self::LockDocumented => "lock-documented",
            Self::NeverUnlocked => "never-unlocked",
        }
    }
}

/// Decide the arm-time path.
///
/// `call_succeeded` without `symbol_present` is inconsistent. That row refuses
/// to arm instead of treating the call as a success or as a documented-path
/// request.
pub fn choose(symbol_present: bool, call_succeeded: bool, immediate_password: bool) -> LockArm {
    if call_succeeded {
        if symbol_present {
            LockArm::PrivateCall
        } else {
            LockArm::RefuseToArm
        }
    } else if immediate_password {
        LockArm::DocumentedPath
    } else {
        LockArm::RefuseToArm
    }
}

/// Owner-facing text when arming is refused because the documented path
/// cannot require an immediate password.
pub fn refuse_to_arm_message() -> String {
    format!(
        "{name} will not arm. Open System Settings, then Lock Screen. Turn on \"Require password after screen saver begins or display is turned off\" and set the delay to zero. Until that setting can require the account password immediately, {name} refuses to arm. The result is not Unlocked.",
        name = PRODUCT_DISPLAY_NAME
    )
}

/// After an error during a lock exercise. Never returns an unlocked end.
pub fn fail_closed_after_error(
    symbol_present: bool,
    private_call_failed: bool,
    documented_path_failed: bool,
    immediate_password: bool,
) -> FailClosedEnd {
    let private_available = symbol_present && !private_call_failed;
    let documented_available = immediate_password && !documented_path_failed;
    if private_available {
        FailClosedEnd::LockPrivate
    } else if documented_available {
        FailClosedEnd::LockDocumented
    } else {
        FailClosedEnd::NeverUnlocked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_missing_requests_the_documented_path() {
        let arm = choose(false, false, true);
        assert_eq!(arm, LockArm::DocumentedPath);
        assert!(arm.requests_documented_path());
        assert!(arm.arms());
        assert!(!arm.reports_unlocked());
    }

    #[test]
    fn a_failed_call_requests_the_documented_path() {
        let arm = choose(true, false, true);
        assert_eq!(arm, LockArm::DocumentedPath);
        assert!(arm.requests_documented_path());
        assert!(!arm.reports_unlocked());
    }

    #[test]
    fn a_successful_private_call_does_not_request_the_documented_path() {
        for immediate in [false, true] {
            let arm = choose(true, true, immediate);
            assert_eq!(arm, LockArm::PrivateCall);
            assert!(!arm.requests_documented_path());
            assert!(arm.arms());
            assert!(!arm.reports_unlocked());
        }
    }

    #[test]
    fn documented_path_unavailable_or_password_not_required_refuses_to_arm() {
        let missing = choose(false, false, false);
        let failed = choose(true, false, false);
        for arm in [missing, failed] {
            assert_eq!(arm, LockArm::RefuseToArm);
            assert!(!arm.requests_documented_path());
            assert!(!arm.arms());
            assert!(!arm.reports_unlocked());
        }
        let message = refuse_to_arm_message();
        assert!(message.contains(PRODUCT_DISPLAY_NAME));
        assert!(message.contains("Require password after screen saver begins"));
        assert!(message.contains("set the delay to zero"));
        assert!(message.contains("not Unlocked"));
    }

    #[test]
    fn a_successful_call_without_a_symbol_refuses_to_arm() {
        assert_eq!(choose(false, true, true), LockArm::RefuseToArm);
        assert_eq!(choose(false, true, false), LockArm::RefuseToArm);
    }

    #[test]
    fn every_input_row_stays_off_unlocked() {
        let mut documented = 0;
        let mut private = 0;
        let mut refused = 0;
        for symbol in [false, true] {
            for call in [false, true] {
                for immediate in [false, true] {
                    let arm = choose(symbol, call, immediate);
                    assert!(!arm.reports_unlocked());
                    match arm {
                        LockArm::DocumentedPath => {
                            assert!(immediate);
                            assert!(!call || !symbol);
                            assert!(!call);
                            documented += 1;
                        }
                        LockArm::PrivateCall => {
                            assert!(symbol && call);
                            private += 1;
                        }
                        LockArm::RefuseToArm => {
                            assert!(!arm.arms());
                            refused += 1;
                        }
                    }
                }
            }
        }
        assert_eq!((documented, private, refused), (2, 2, 4));
    }

    #[test]
    fn an_error_ends_locked_or_never_unlocked() {
        let cases = [
            (true, false, false, false, FailClosedEnd::LockPrivate),
            (true, false, false, true, FailClosedEnd::LockPrivate),
            (true, true, false, true, FailClosedEnd::LockDocumented),
            (true, true, false, false, FailClosedEnd::NeverUnlocked),
            (false, false, false, true, FailClosedEnd::LockDocumented),
            (false, false, false, false, FailClosedEnd::NeverUnlocked),
            (true, true, true, true, FailClosedEnd::NeverUnlocked),
            (false, true, true, true, FailClosedEnd::NeverUnlocked),
        ];
        for (symbol, private_failed, documented_failed, immediate, expected) in cases {
            let end = fail_closed_after_error(symbol, private_failed, documented_failed, immediate);
            assert_eq!(end, expected);
            assert!(!end.reports_unlocked());
            assert_eq!(end.locks_the_os(), expected != FailClosedEnd::NeverUnlocked);
        }
    }

    #[test]
    fn the_private_symbol_is_the_one_decision_d5_named() {
        assert_eq!(PRIVATE_LOCK_SYMBOL, "SACLockScreenImmediate");
        assert!(LOGIN_FRAMEWORK_IMAGE.contains("login.framework"));
        assert!(!LOGIN_FRAMEWORK_IMAGE.contains("link"));
        assert_eq!(SCREEN_SAVER_ENGINE_CANDIDATES.len(), 2);
    }
}
