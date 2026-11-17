//! Drop serials, MAC addresses, and machine ids from written output.

/// Needles the isolation test also searches for. Do not read these sysfs nodes.
pub const BANNED_PATHS: &[&str] = &[
    "/sys/class/dmi/id/product_serial",
    "/sys/class/dmi/id/board_serial",
    "/sys/class/dmi/id/chassis_serial",
    "/sys/class/dmi/id/product_uuid",
];

pub const SERIAL_MARKERS: &[&str] = &[
    "IOPlatformSerialNumber",
    "Serial Number",
    "product_serial",
    "board_serial",
    "chassis_serial",
    "EVIOCGUNIQ",
];

pub fn contains_serial(text: &str) -> bool {
    for marker in SERIAL_MARKERS {
        if text.contains(marker) {
            return true;
        }
    }
    mac_addr(text) || long_hex_id(text)
}

pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let mut line = line.to_string();
        line = redact_mac(&line);
        line = redact_hex_ids(&line);
        for marker in SERIAL_MARKERS {
            if line.contains(marker) {
                line = "[redacted]".to_string();
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    if text.ends_with('\n') {
        out
    } else {
        out.trim_end_matches('\n').to_string()
    }
}

fn mac_addr(text: &str) -> bool {
    // 00:11:22:33:44:55
    let bytes = text.as_bytes();
    if bytes.len() < 17 {
        return false;
    }
    bytes.windows(17).any(is_mac)
}

fn is_mac(w: &[u8]) -> bool {
    if w.len() != 17 {
        return false;
    }
    for i in 0..6 {
        let a = w[i * 3];
        let b = w[i * 3 + 1];
        if !a.is_ascii_hexdigit() || !b.is_ascii_hexdigit() {
            return false;
        }
        if i < 5 && w[i * 3 + 2] != b':' {
            return false;
        }
    }
    true
}

fn long_hex_id(text: &str) -> bool {
    let mut run = 0;
    for c in text.chars() {
        if c.is_ascii_hexdigit() {
            run += 1;
            if run >= 32 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

fn redact_mac(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if i + 17 <= bytes.len() && is_mac(&bytes[i..i + 17]) {
            out.push_str("[mac]");
            i += 17;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn redact_hex_ids(line: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    for c in line.chars() {
        if c.is_ascii_hexdigit() {
            run.push(c);
            continue;
        }
        flush_run(&mut out, &mut run);
        out.push(c);
    }
    flush_run(&mut out, &mut run);
    out
}

fn flush_run(out: &mut String, run: &mut String) {
    if run.len() >= 32 {
        out.push_str("[id]");
    } else {
        out.push_str(run);
    }
    run.clear();
}

#[cfg(test)]
mod tests {
    use super::{contains_serial, redact};

    #[test]
    fn redacts_mac_and_machine_id() {
        let text = "nic 00:11:22:33:44:55 id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n";
        assert!(contains_serial(text));
        let cleaned = redact(text);
        assert!(!cleaned.contains("00:11:22:33:44:55"));
        assert!(cleaned.contains("[mac]"));
        assert!(cleaned.contains("[id]"));
        assert!(!contains_serial(&cleaned));
    }

    #[test]
    fn kernel_version_is_not_a_serial() {
        assert!(!contains_serial("kernel=6.8.0-40-generic"));
        assert!(!contains_serial("card0-HDMI-A-1 connected"));
    }
}
