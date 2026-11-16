//! The PIN goes through Core: Argon2id, a wiped secret, and a redacted debug string.

use std::sync::OnceLock;

use softlock_core::{PinHashError, Secret, Verdict, hash_softlock_pin, verify_softlock_pin};

const PIN: &[u8] = b"135742";

fn encoded_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| hash_softlock_pin(&Secret::new(PIN)).expect("spec Argon2id hash"))
}

#[test]
fn correct_pin_verifies_and_a_wrong_pin_does_not() {
    let encoded = encoded_hash();
    assert!(
        encoded.starts_with("$argon2id$"),
        "the stored hash must be an Argon2id PHC string"
    );
    assert_eq!(
        verify_softlock_pin(&Secret::new(PIN), encoded),
        Verdict::Verified
    );
    assert_eq!(
        verify_softlock_pin(&Secret::new(b"000000"), encoded),
        Verdict::Rejected
    );
    assert_eq!(
        verify_softlock_pin(&Secret::new(b""), encoded),
        Verdict::Rejected
    );
}

#[test]
fn a_hash_outside_the_spec_parameters_is_rejected() {
    let encoded = encoded_hash();
    let weaker = encoded.replace("m=65536", "m=4096");
    assert_ne!(weaker, encoded);
    assert_eq!(
        verify_softlock_pin(&Secret::new(PIN), &weaker),
        Verdict::Rejected
    );
    let argon2i = encoded.replacen("$argon2id$", "$argon2i$", 1);
    assert_eq!(
        verify_softlock_pin(&Secret::new(PIN), &argon2i),
        Verdict::Rejected
    );
    assert_eq!(
        verify_softlock_pin(&Secret::new(PIN), "not-a-phc-string"),
        Verdict::Rejected
    );
}

#[test]
fn the_pin_is_not_in_debug_output() {
    let secret = Secret::new(PIN);
    let rendered = format!("{secret:?}");
    let pin = std::str::from_utf8(PIN).expect("pin is utf-8");
    assert!(
        !rendered.contains(pin),
        "secret debug output included the pin"
    );
    let verdict = verify_softlock_pin(&secret, "not-a-phc-string");
    let verdict = format!("{verdict:?}");
    assert!(!verdict.contains(pin));
    let error = format!("{:?}", PinHashError);
    assert!(!error.contains(pin));
}
