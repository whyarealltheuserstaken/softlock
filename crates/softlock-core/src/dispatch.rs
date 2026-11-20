//! One role × state table for public requests. The controller applies the
//! side effects after this says the request is allowed.

use crate::ipc::{RequestKind, ResponseKind};
use crate::state::{LockState, Role};

/// `None` means the request may proceed. `Some` is the rejection to return.
pub fn denied(
    role: Role,
    kind: &RequestKind,
    state: &LockState,
    session_os_locked: bool,
) -> Option<ResponseKind> {
    let writable = matches!(state, LockState::Unlocked) && !session_os_locked;
    match kind {
        RequestKind::Hello { .. } | RequestKind::GetStatus => None,
        RequestKind::Lock { .. } => {
            if !matches!(role, Role::App | Role::LockClient) {
                Some(ResponseKind::Unauthorized)
            } else if !matches!(state, LockState::Unlocked) {
                Some(ResponseKind::AlreadyLocked)
            } else {
                None
            }
        }
        RequestKind::HardLock => {
            let app_unlocked = role == Role::App && matches!(state, LockState::Unlocked);
            let host_locked = role == Role::LockHost && matches!(state, LockState::Locked { .. });
            if app_unlocked || host_locked {
                None
            } else {
                Some(ResponseKind::Unauthorized)
            }
        }
        RequestKind::GetConfig | RequestKind::ValidateConfig { .. } => {
            if matches!(role, Role::App | Role::LockClient) {
                None
            } else {
                Some(ResponseKind::Unauthorized)
            }
        }
        RequestKind::PutConfig { .. } => {
            if role != Role::App {
                Some(ResponseKind::Unauthorized)
            } else if writable {
                None
            } else {
                Some(ResponseKind::Locked)
            }
        }
        RequestKind::ConfirmSensitiveChange { .. }
        | RequestKind::SetPin { .. }
        | RequestKind::ClearPin
        | RequestKind::CheckUpdate
        | RequestKind::InstallUpdate { .. } => {
            if role == Role::App {
                None
            } else {
                Some(ResponseKind::Unauthorized)
            }
        }
        RequestKind::Subscribe => {
            if role == Role::App {
                None
            } else {
                Some(ResponseKind::Unauthorized)
            }
        }
    }
}
