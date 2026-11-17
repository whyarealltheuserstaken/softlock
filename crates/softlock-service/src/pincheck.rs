//! PIN check for the dev-launch Lock Host.
//!
//! The host used to hash the PIN and draw its own nonce. Both of those now
//! live here, in the service crate. A grant carries a nonce this checker
//! drew. The host only forwards it.

use softlock_core::{LaunchNonce, PinAuthority, PinDecision, Secret, Verdict, verify_softlock_pin};

/// Hyphenated marker. The release-binary check rejects this byte string.
pub const SIMULATED_PHYSICAL_MARKER: &[u8] = b"softlock-simulated-physical-marker";

/// Symbol the release-binary check rejects. Present only with the test feature.
#[used]
#[unsafe(no_mangle)]
pub static softlock_simulated_physical_marker: [u8; SIMULATED_PHYSICAL_MARKER.len()] =
    *b"softlock-simulated-physical-marker";

/// Keeps the marker bytes reachable from the service binary.
#[inline(never)]
pub fn marker_bytes() -> &'static [u8] {
    &softlock_simulated_physical_marker
}

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
