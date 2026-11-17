//! Fail-closed helper protocol.
//!
//! The helper blocks on stdin. `complete` means the parent finished and the
//! helper must not lock. End of input without that line means the parent died.
//! The helper still has to pass the kit gate before it locks, and it never
//! unlocks.

pub const COMPLETE_LINE: &str = "complete";
pub const HELPER_ENV: &str = "MACOS_LOCK_KIT_HELPER";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperAction {
    Private,
    Documented,
    None,
}

impl HelperAction {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "private" => Some(Self::Private),
            "documented" => Some(Self::Documented),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Documented => "documented",
            Self::None => "none",
        }
    }
}

pub fn parent_completed(stdin_text: &str) -> bool {
    stdin_text.lines().any(|line| line.trim() == COMPLETE_LINE)
}

/// The helper locks only when the parent did not finish and a real path was armed.
pub fn helper_should_lock(action: HelperAction, stdin_text: &str, gate_allows_lock: bool) -> bool {
    gate_allows_lock && !parent_completed(stdin_text) && action != HelperAction::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_stands_down_and_a_dropped_parent_does_not_unlock() {
        assert!(parent_completed("complete\n"));
        assert!(!parent_completed(""));
        assert!(!parent_completed("error\n"));
        assert!(!helper_should_lock(
            HelperAction::Private,
            "complete\n",
            true
        ));
        assert!(helper_should_lock(HelperAction::Private, "", true));
        assert!(helper_should_lock(
            HelperAction::Documented,
            "error\n",
            true
        ));
        assert!(!helper_should_lock(HelperAction::None, "", true));
        assert!(!helper_should_lock(HelperAction::Private, "", false));
    }
}
