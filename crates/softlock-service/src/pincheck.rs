//! PIN check for the dev-launch Lock Host.
//!
//! The host used to hash the PIN and draw its own nonce. Both of those now
//! live here, in the service crate. A grant carries a nonce this checker
//! drew. The host only forwards it.

use softlock_core::{LaunchNonce, PinAuthority, PinDecision, Secret, Verdict, verify_softlock_pin};

pub struct PinChecker {
    hash: String,
}

impl PinChecker {
    pub fn issue(hash: impl Into<String>) -> Self {
        Self { hash: hash.into() }
    }
}

impl PinAuthority for PinChecker {
    fn decide(&mut self, secret: &Secret, allow_grant: bool) -> PinDecision {
        let matched = verify_softlock_pin(secret, &self.hash) == Verdict::Verified;
        if !matched {
            return PinDecision::Deny;
        }
        if !allow_grant {
            return PinDecision::Hold;
        }
        match LaunchNonce::generate() {
            Ok(nonce) => PinDecision::Grant(nonce),
            Err(_) => PinDecision::Deny,
        }
    }
}
