//! What the Service tells the Lock Host after a PIN check.
//!
//! The Lock Host does not hash the PIN and does not draw the nonce. A grant
//! carries the nonce the Service already drew. The controller compares it
//! with [`LaunchNonce::eq_ct`] and then consumes it.

use crate::state::{LaunchNonce, Secret};

/// Result of one PIN submission. `Hold` means the secret matched and the
/// nonce is still unused, because a grant is not allowed in this state.
#[derive(Debug)]
pub enum PinDecision {
    Grant(LaunchNonce),
    Deny,
    Hold,
}

pub trait PinAuthority: Send {
    fn decide(&mut self, secret: &Secret, allow_grant: bool) -> PinDecision;
}
