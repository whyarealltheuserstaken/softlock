//! Drop hardware serials and other personal details before a result is written.
//!
//! Henry commits the result. It may record OS version, a model identifier, and
//! FileVault on/off. It must not record an account name, a home path, an Apple
//! ID, a serial, another app, or another device.

use crate::text::one_line;

pub const NEEDLES: &[&str] = &[
    "IOPlatformSerialNumber",
    "IOPlatformUUID",
    "Serial Number",
    "hw.serialnumber",
];

const PERSONAL_KEYS: &[&str] = &[
    "ComputerName",
    "LocalHostName",
    "HostName",
    "UserName",
    "AppleID",
    "iCloud",
    "Bluetooth",
    "SerialNumber",
    "system_profiler",
    "SPApplications",
    "SPBluetooth",
    "DeviceName",
];

pub fn contains_marker(text: &str) -> bool {
    NEEDLES.iter().any(|needle| text.contains(needle))
}

pub fn leaks_personal(text: &str) -> bool {
    contains_marker(text)
        || text.contains("/Users/")
        || text.contains("/home/")
        || text.contains('@')
        || text.contains("user '")
        || text.contains("user \"")
        || contains_uuid(text)
}

pub fn sanitize(text: &str) -> String {
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let newline = &line[body.len()..];
        if contains_marker(body) || PERSONAL_KEYS.iter().any(|key| body.contains(key)) {
            out.push_str("redacted");
        } else {
            out.push_str(&scrub_inline(&one_line(body)));
        }
        out.push_str(newline);
    }
    out
}

pub fn numeric_uid(text: &str) -> String {
    let digits: String = sanitize(text)
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        "unknown".to_string()
    } else {
        digits
    }
}

pub fn model_identifier(text: &str) -> String {
    let clean = one_line(&sanitize(text));
    let token = clean.split_whitespace().next().unwrap_or("");
    if !token.is_empty()
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == ',' || ch == '-')
    {
        token.to_string()
    } else {
        "redacted".to_string()
    }
}

fn scrub_inline(text: &str) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < text.len() {
        if let Some(len) = match_path(text, index) {
            out.push_str("redacted-path");
            index += len;
            continue;
        }
        if let Some(len) = match_email(text, index) {
            out.push_str("redacted-email");
            index += len;
            continue;
        }
        if let Some(len) = match_uuid(text, index) {
            out.push_str("redacted-id");
            index += len;
            continue;
        }
        if let Some(len) = match_user_clause(text, index) {
            out.push_str("user redacted");
            index += len;
            continue;
        }
        if let Some(len) = match_serial_token(text, index) {
            out.push_str("redacted");
            index += len;
            continue;
        }
        let ch = text[index..]
            .chars()
            .next()
            .expect("index on a char boundary");
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn match_path(text: &str, index: usize) -> Option<usize> {
    let rest = &text[index..];
    let prefix = if rest.starts_with("/Users/") {
        "/Users/"
    } else if rest.starts_with("/home/") {
        "/home/"
    } else if rest.starts_with("~/") {
        "~/"
    } else {
        return None;
    };
    let mut len = prefix.len();
    let bytes = rest.as_bytes();
    while len < bytes.len() && !bytes[len].is_ascii_whitespace() {
        len += 1;
    }
    Some(len)
}

fn match_email(text: &str, index: usize) -> Option<usize> {
    let rest = &text[index..];
    let at = rest.find('@')?;
    if at == 0 || at > 64 {
        return None;
    }
    let local = &rest[..at];
    if !local
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '%' | '+' | '-'))
    {
        return None;
    }
    let after = &rest[at + 1..];
    let domain_len = after
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '.' | '-'))
        .map(char::len_utf8)
        .sum::<usize>();
    let domain = &after[..domain_len];
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return None;
    }
    Some(at + 1 + domain_len)
}

fn match_uuid(text: &str, index: usize) -> Option<usize> {
    let rest = text[index..].as_bytes();
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut cursor = 0;
    for (group, size) in GROUPS.iter().enumerate() {
        if group > 0 {
            if cursor >= rest.len() || rest[cursor] != b'-' {
                return None;
            }
            cursor += 1;
        }
        if rest.len() < cursor + size {
            return None;
        }
        if !rest[cursor..cursor + size]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        cursor += size;
    }
    Some(cursor)
}

fn contains_uuid(text: &str) -> bool {
    text.char_indices()
        .any(|(index, _)| match_uuid(text, index).is_some())
}

fn match_user_clause(text: &str, index: usize) -> Option<usize> {
    if index > 0 {
        let previous = text[..index].chars().next_back()?;
        if previous.is_ascii_alphanumeric() {
            return None;
        }
    }
    let rest = &text[index..];
    let prefix = if rest.starts_with("for user ") {
        "for user "
    } else if rest.starts_with("user ") {
        "user "
    } else {
        return None;
    };
    let after = &rest[prefix.len()..];
    if after.starts_with('\'') || after.starts_with('"') {
        let quote = after.chars().next()?;
        let tail = &after[quote.len_utf8()..];
        let inside = tail.find(quote)?;
        return Some(prefix.len() + quote.len_utf8() + inside + quote.len_utf8());
    }
    let mut chars = after.chars();
    let first = chars.next()?;
    if !first.is_ascii_uppercase() {
        return None;
    }
    let token = first.len_utf8()
        + chars
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '.' | '_' | '-'))
            .map(char::len_utf8)
            .sum::<usize>();
    Some(prefix.len() + token)
}

fn match_serial_token(text: &str, index: usize) -> Option<usize> {
    if index > 0 {
        let previous = text[..index].chars().next_back()?;
        if previous.is_ascii_alphanumeric() {
            return None;
        }
    }
    let rest = text[index..].as_bytes();
    if rest.len() < 10 || rest[0] != b'C' || rest[1] != b'0' {
        return None;
    }
    let mut len = 2;
    while len < rest.len() && len < 12 && rest[len].is_ascii_alphanumeric() {
        len += 1;
    }
    if len < 10 {
        return None;
    }
    if rest.len() > len && rest[len].is_ascii_alphanumeric() {
        return None;
    }
    Some(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hardware_serial_line_is_removed_and_flagged() {
        let planted = format!("model=ok\n{}=C02HIDDEN\n", NEEDLES[0]);
        assert!(contains_marker(&planted));
        let clean = sanitize(&planted);
        assert!(!contains_marker(&clean));
        assert!(clean.contains("redacted"));
        assert!(clean.contains("model=ok"));
    }

    #[test]
    fn ordinary_filevault_text_is_kept() {
        let text = "FileVault is On.\ntrue\n";
        assert!(!contains_marker(text));
        assert!(!leaks_personal(text));
        assert_eq!(sanitize(text), text);
    }

    #[test]
    fn account_names_paths_and_other_devices_are_removed() {
        let text = "\
Deferred enablement appears to be active for user 'Henry'
home=/Users/Henry/Library mail=henry@example.com
id=550e8400-e29b-41d4-a716-446655440000 serial=C02ABCDEFGH
ComputerName: Henry's Mac
FileVault is On.
";
        let clean = sanitize(text);
        assert!(!leaks_personal(&clean), "{clean}");
        assert!(!clean.contains("Henry"));
        assert!(!clean.contains("example.com"));
        assert!(!clean.contains("C02ABCDEFGH"));
        assert!(clean.contains("FileVault is On."));
        assert!(clean.contains("user redacted"));
        assert!(clean.contains("redacted-path"));
        assert!(clean.contains("redacted-email"));
        assert!(clean.contains("redacted-id"));
    }

    #[test]
    fn uid_and_model_keep_only_the_public_identifiers() {
        assert_eq!(numeric_uid("501\n"), "501");
        assert_eq!(numeric_uid("user Henry\n"), "unknown");
        assert_eq!(model_identifier("MacBookPro16,2\n"), "MacBookPro16,2");
        assert_eq!(
            model_identifier("MacBookPro16,2 /Users/Henry\n"),
            "MacBookPro16,2"
        );
    }
}
