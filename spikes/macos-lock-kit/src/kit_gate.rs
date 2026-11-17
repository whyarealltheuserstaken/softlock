//! Whether the hardware kit may call a lock path.
//!
//! Read-only collection is allowed on a GitHub runner. Locking is not: the
//! runner spike did not show that the session comes back after a lock. The
//! kit also refuses to run as root, so FileVault status is the console user's.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KitInvocation {
    ReadOnly,
    Exercise { acknowledged: bool },
    Helper { parent_armed: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KitPermission {
    ReadOnly,
    MayLock,
    Refuse(&'static str),
}

pub const ROOT_REFUSAL: &str = "run as the console user, not root";
pub const RUNNER_LOCK_REFUSAL: &str = "refusing to lock a GitHub runner; the desktop spike did not show that the runner comes back after a lock";
pub const ACK_REFUSAL: &str =
    "pass --i-will-unlock after reading the kit instructions; the kit cannot unlock the Mac";
pub const HELPER_REFUSAL: &str = "the fail-closed helper only runs when the kit parent arms it";

pub fn permit(invocation: KitInvocation, github_actions: bool, root: bool) -> KitPermission {
    if root {
        return KitPermission::Refuse(ROOT_REFUSAL);
    }
    match invocation {
        KitInvocation::ReadOnly => KitPermission::ReadOnly,
        KitInvocation::Exercise { acknowledged } if github_actions => {
            let _ = acknowledged;
            KitPermission::Refuse(RUNNER_LOCK_REFUSAL)
        }
        KitInvocation::Exercise {
            acknowledged: false,
        } => KitPermission::Refuse(ACK_REFUSAL),
        KitInvocation::Exercise { acknowledged: true } => KitPermission::MayLock,
        KitInvocation::Helper { parent_armed: _ } if github_actions => {
            KitPermission::Refuse(RUNNER_LOCK_REFUSAL)
        }
        KitInvocation::Helper {
            parent_armed: false,
        } => KitPermission::Refuse(HELPER_REFUSAL),
        KitInvocation::Helper { parent_armed: true } => KitPermission::MayLock,
    }
}

pub fn may_lock(permission: KitPermission) -> bool {
    matches!(permission, KitPermission::MayLock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_never_locks_including_on_a_runner() {
        for github in [false, true] {
            let permission = permit(KitInvocation::ReadOnly, github, false);
            assert_eq!(permission, KitPermission::ReadOnly);
            assert!(!may_lock(permission));
        }
    }

    #[test]
    fn the_runner_and_root_cannot_lock() {
        let exercise = KitInvocation::Exercise { acknowledged: true };
        assert_eq!(
            permit(exercise, true, false),
            KitPermission::Refuse(RUNNER_LOCK_REFUSAL)
        );
        assert_eq!(
            permit(exercise, false, true),
            KitPermission::Refuse(ROOT_REFUSAL)
        );
        assert_eq!(
            permit(KitInvocation::ReadOnly, true, true),
            KitPermission::Refuse(ROOT_REFUSAL)
        );
        let helper = KitInvocation::Helper { parent_armed: true };
        assert_eq!(
            permit(helper, true, false),
            KitPermission::Refuse(RUNNER_LOCK_REFUSAL)
        );
        assert!(!may_lock(permit(exercise, true, false)));
    }

    #[test]
    fn exercise_locks_only_after_the_acknowledgement() {
        assert_eq!(
            permit(
                KitInvocation::Exercise {
                    acknowledged: false
                },
                false,
                false
            ),
            KitPermission::Refuse(ACK_REFUSAL)
        );
        let allowed = permit(KitInvocation::Exercise { acknowledged: true }, false, false);
        assert_eq!(allowed, KitPermission::MayLock);
        assert!(may_lock(allowed));
        assert_eq!(
            permit(
                KitInvocation::Helper {
                    parent_armed: false
                },
                false,
                false
            ),
            KitPermission::Refuse(HELPER_REFUSAL)
        );
        assert!(may_lock(permit(
            KitInvocation::Helper { parent_armed: true },
            false,
            false
        )));
    }
}
