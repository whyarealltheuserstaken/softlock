//! Drop lines that would record a hardware serial or platform UUID.

use crate::text::one_line;

pub const NEEDLES: &[&str] = &[
    "IOPlatformSerialNumber",
    "IOPlatformUUID",
    "Serial Number",
    "hw.serialnumber",
];

pub fn contains_marker(text: &str) -> bool {
    NEEDLES.iter().any(|needle| text.contains(needle))
}

pub fn sanitize(text: &str) -> String {
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let newline = &line[body.len()..];
        if contains_marker(body) {
            out.push_str("redacted");
        } else {
            out.push_str(&one_line(body));
        }
        out.push_str(newline);
    }
    out
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
        assert_eq!(sanitize(text), text);
    }
}
