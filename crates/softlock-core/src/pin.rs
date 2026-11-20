//! PIN check. Argon2id with the parameters from the spec.
//!
//! The secret is borrowed, never cloned, and never written into an error.
//! The hash compare is the constant-time compare inside the Argon2 verifier.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

use crate::platform::Verdict;
use crate::state::Secret;

/// Memory cost in kibibytes. 64 MiB.
pub const PIN_MEMORY_KIB: u32 = 64 * 1024;
/// Iterations.
pub const PIN_TIME_COST: u32 = 3;
/// Lanes.
pub const PIN_PARALLELISM: u32 = 1;
/// Salt length in bytes.
pub const PIN_SALT_LEN: usize = 16;
/// Minimum PIN length the settings field accepts, in Unicode scalar values.
pub const MIN_PIN_LEN: usize = 4;

/// Hashing failed before a PHC string existed. The secret is not part of this error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinHashError;

/// Hashes a PIN for the dev launch and, later, the service secret store.
pub fn hash_softlock_pin(secret: &Secret) -> Result<String, PinHashError> {
    let mut salt_bytes = [0u8; PIN_SALT_LEN];
    getrandom::getrandom(&mut salt_bytes).map_err(|_| PinHashError)?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|_| PinHashError)?;
    salt_bytes.fill(0);
    let hasher = spec_hasher()?;
    let hash = hasher
        .hash_password(secret.expose(), &salt)
        .map_err(|_| PinHashError)?;
    Ok(hash.to_string())
}

/// Checks `secret` against a PHC string. A hash that is not spec Argon2id is rejected.
pub fn verify_softlock_pin(secret: &Secret, encoded: &str) -> Verdict {
    let Ok(hash) = PasswordHash::new(encoded) else {
        return Verdict::Rejected;
    };
    if !spec_parameters(&hash) {
        return Verdict::Rejected;
    }
    let Ok(hasher) = spec_hasher() else {
        return Verdict::Rejected;
    };
    match hasher.verify_password(secret.expose(), &hash) {
        Ok(()) => Verdict::Verified,
        Err(_) => Verdict::Rejected,
    }
}

fn spec_hasher() -> Result<Argon2<'static>, PinHashError> {
    let params = Params::new(PIN_MEMORY_KIB, PIN_TIME_COST, PIN_PARALLELISM, None)
        .map_err(|_| PinHashError)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

fn spec_parameters(hash: &PasswordHash<'_>) -> bool {
    if hash.algorithm != Algorithm::Argon2id.ident() {
        return false;
    }
    if hash.version != Some(Version::V0x13 as u32) {
        return false;
    }
    if hash.params.get_decimal("m") != Some(PIN_MEMORY_KIB)
        || hash.params.get_decimal("t") != Some(PIN_TIME_COST)
        || hash.params.get_decimal("p") != Some(PIN_PARALLELISM)
    {
        return false;
    }
    let Some(salt) = hash.salt else {
        return false;
    };
    let mut decoded = [0u8; 64];
    match salt.decode_b64(&mut decoded) {
        Ok(bytes) => bytes.len() == PIN_SALT_LEN,
        Err(_) => false,
    }
}
