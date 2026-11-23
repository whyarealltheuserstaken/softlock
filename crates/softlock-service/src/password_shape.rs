//! Bounds for an account password before it is turned into UTF-16.
//!
//! Windows rejects a password longer than 127 characters. Anything past that,
//! or that is not UTF-8, is a failed attempt and is not passed to the logon API.

/// Windows' maximum password length, in Unicode scalar values.
pub const PASSWORD_MAX_CHARS: usize = 127;

/// UTF-8 bound. 127 non-BMP characters are four bytes each.
pub const PASSWORD_MAX_BYTES: usize = PASSWORD_MAX_CHARS * 4;

pub fn password_accepted_shape(bytes: &[u8]) -> bool {
    if bytes.len() > PASSWORD_MAX_BYTES || bytes.contains(&0) {
        return false;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    text.chars().count() <= PASSWORD_MAX_CHARS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_password_is_accepted_and_an_overlong_one_is_not() {
        assert!(password_accepted_shape(b"Aa1!qk"));
        assert!(password_accepted_shape(b""));
        assert!(!password_accepted_shape(&vec![
            b'a';
            PASSWORD_MAX_BYTES + 1
        ]));
        let mut chars = "a".repeat(PASSWORD_MAX_CHARS);
        assert!(password_accepted_shape(chars.as_bytes()));
        chars.push('b');
        assert!(!password_accepted_shape(chars.as_bytes()));
        assert!(!password_accepted_shape(b"ab\0c"));
        assert!(!password_accepted_shape(&[0xff, 0xfe]));
    }
}
