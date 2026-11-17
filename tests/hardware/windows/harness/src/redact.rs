//! Drop serial numbers and hardware ids before anything is written.

pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(skip) = redacted_span(&lower, index) {
            out.push_str("[redacted]");
            index += skip;
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

fn redacted_span(lower: &str, index: usize) -> Option<usize> {
    let rest = &lower[index..];
    for prefix in [
        "serialnumber",
        "serial number",
        "serial",
        "machineguid",
        "hardwareid",
    ] {
        if let Some(skip) = assignment_span(rest, prefix) {
            return Some(skip);
        }
    }
    for prefix in ["pci\\ven_", "usb\\vid_", "hid\\vid_", "acpi\\"] {
        if rest.starts_with(prefix) {
            return Some(id_token_len(rest));
        }
    }
    if rest.starts_with("c:\\users\\") || rest.starts_with("c:/users/") {
        return Some(user_path_len(rest));
    }
    None
}

fn assignment_span(rest: &str, key: &str) -> Option<usize> {
    if !rest.starts_with(key) {
        return None;
    }
    let after = &rest[key.len()..];
    let after = after.trim_start_matches([' ', '\t']);
    let sep = after.starts_with('=') || after.starts_with(':');
    if !sep && !key.contains(' ') {
        return None;
    }
    let tail = if sep {
        after[1..].trim_start_matches([' ', '\t'])
    } else {
        after
    };
    let value = tail
        .chars()
        .take_while(|ch| !ch.is_whitespace() && *ch != ',' && *ch != ';')
        .map(char::len_utf8)
        .sum::<usize>();
    if value == 0 {
        return None;
    }
    Some(rest.len() - tail.len() + value)
}

fn id_token_len(rest: &str) -> usize {
    rest.chars()
        .take_while(|ch| !ch.is_whitespace() && *ch != ',' && *ch != '"')
        .map(char::len_utf8)
        .sum()
}

fn user_path_len(rest: &str) -> usize {
    let slash = if rest.starts_with("c:\\users\\") {
        "c:\\users\\".len()
    } else {
        "c:/users/".len()
    };
    let name = rest[slash..]
        .chars()
        .take_while(|ch| *ch != '\\' && *ch != '/')
        .map(char::len_utf8)
        .sum::<usize>();
    slash + name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serials_guids_and_hardware_ids_are_removed() {
        let raw = "serial=ABC123 machineguid: {dead} PCI\\VEN_8086&DEV_1234 USB\\VID_046D&PID_C52B path=C:\\Users\\student\\Desktop";
        let clean = redact(raw);
        assert!(!clean.to_ascii_lowercase().contains("abc123"));
        assert!(!clean.contains("dead"));
        assert!(!clean.contains("VEN_8086"));
        assert!(!clean.contains("VID_046D"));
        assert!(!clean.to_ascii_lowercase().contains("student"));
        assert!(clean.contains("[redacted]"));
    }

    #[test]
    fn ordinary_build_numbers_stay() {
        let clean = redact("Windows build 26100 displays 1");
        assert_eq!(clean, "Windows build 26100 displays 1");
    }
}
