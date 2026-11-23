//! How one key from `ToUnicodeEx` becomes password text.
//!
//! A dead key is held for the next character. A key that produces no text is
//! rejected, and a password that contains one is not sent to the Service, so
//! a layout the prompt cannot type never costs a Windows bad-password count.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DecodedKey {
    Text(Vec<u8>),
    Dead,
    Untranslated,
}

pub(crate) fn classify_translation(written: i32, bytes: &[u8]) -> DecodedKey {
    if written < 0 {
        DecodedKey::Dead
    } else if written == 0 || bytes.is_empty() {
        DecodedKey::Untranslated
    } else {
        DecodedKey::Text(bytes.to_vec())
    }
}

/// A password may be submitted only when every key translated and no dead key
/// is still waiting for the next character.
pub(crate) fn password_may_submit(untranslated: bool, dead_pending: bool) -> bool {
    !untranslated && !dead_pending
}

/// A key that is supposed to produce a character. Function keys, arrows, and
/// the wake key do not. `ToUnicodeEx` returns nothing for those, and that
/// must not throw away a password the owner then submits.
pub(crate) fn character_key(vk: u16) -> bool {
    matches!(vk, 0x20 | 0x30..=0x39 | 0x41..=0x5A | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE2 | 0xE7)
}

/// True when this key was a character the layout did not finish.
pub(crate) fn password_key_failed(vk: u16, decoded: &DecodedKey) -> bool {
    character_key(vk) && matches!(decoded, DecodedKey::Dead | DecodedKey::Untranslated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dead_key_and_an_untranslated_key_do_not_become_a_password() {
        assert_eq!(classify_translation(-1, b"e"), DecodedKey::Dead);
        assert_eq!(classify_translation(0, b""), DecodedKey::Untranslated);
        assert_eq!(classify_translation(1, b""), DecodedKey::Untranslated);
        assert_eq!(
            classify_translation(1, b"A"),
            DecodedKey::Text(b"A".to_vec())
        );
        assert!(!password_may_submit(true, false));
        assert!(!password_may_submit(false, true));
        assert!(password_may_submit(false, false));
        assert!(!password_key_failed(0x7C, &DecodedKey::Untranslated));
        assert!(password_key_failed(b'A' as u16, &DecodedKey::Untranslated));
        assert!(password_key_failed(b'E' as u16, &DecodedKey::Dead));
    }
}
