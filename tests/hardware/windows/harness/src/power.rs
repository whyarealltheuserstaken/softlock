//! `powercfg /a` and `powercfg /requests` text. No operating-system calls.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerAvailability {
    pub known: bool,
    pub s3: bool,
    pub modern_standby: bool,
    pub hibernate: bool,
}

impl PowerAvailability {
    pub fn sleep_label(&self) -> &'static str {
        if !self.known {
            return "unknown";
        }
        if self.modern_standby {
            "modern-standby"
        } else if self.s3 {
            "s3"
        } else if self.hibernate {
            "hibernate"
        } else {
            "none"
        }
    }

    pub fn any_sleep(&self) -> bool {
        self.known && (self.s3 || self.modern_standby || self.hibernate)
    }
}

pub fn parse_powercfg_a(text: &str) -> PowerAvailability {
    let available = available_section(text);
    let known = available.is_some();
    let body = available.unwrap_or_default();
    let lower = body.to_ascii_lowercase();
    PowerAvailability {
        known,
        s3: lower.contains("(s3)"),
        modern_standby: lower.contains("(s0"),
        hibernate: hibernate_word(&lower),
    }
}

fn available_section(text: &str) -> Option<String> {
    let mut available = String::new();
    let mut in_available = false;
    let mut saw_header = false;
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if is_unavailable_header(line) {
            in_available = false;
            saw_header = true;
            continue;
        }
        if is_available_header(line) {
            in_available = true;
            saw_header = true;
            continue;
        }
        if in_available {
            available.push_str(line);
            available.push('\n');
        }
    }
    if saw_header { Some(available) } else { None }
}

fn is_available_header(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if is_unavailable_header(line) {
        return false;
    }
    lower.contains("available on this system")
        || lower.contains("sind auf diesem system verfügbar")
        || lower.contains("sont disponibles")
        || lower.contains("están disponibles")
}

fn is_unavailable_header(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("not available")
        || lower.contains("nicht verfügbar")
        || lower.contains("ne sont pas disponibles")
        || lower.contains("no están disponibles")
}

fn hibernate_word(lower: &str) -> bool {
    lower.contains("hibernate")
        || lower.contains("ruhezustand")
        || lower.contains("hibernación")
        || lower.contains("hibernacion")
        || lower.contains("veille prolongée")
        || lower.contains("veille prolongee")
        || lower.contains("hibernação")
        || lower.contains("hibernacao")
}

/// True when `needle` sits in the named `powercfg /requests` section.
pub fn section_contains(text: &str, section: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut current = String::new();
    let mut body = String::new();
    let mut capture = false;
    for raw in text.lines() {
        let line = raw.trim().trim_end_matches(':').trim();
        if is_request_header(line) {
            if capture && current.eq_ignore_ascii_case(section) && body.contains(needle) {
                return true;
            }
            current = line.to_string();
            body.clear();
            capture = true;
            continue;
        }
        if capture {
            body.push_str(raw);
            body.push('\n');
        }
    }
    capture && current.eq_ignore_ascii_case(section) && body.contains(needle)
}

fn is_request_header(line: &str) -> bool {
    matches!(
        line.to_ascii_uppercase().as_str(),
        "DISPLAY" | "SYSTEM" | "AWAYMODE" | "EXECUTION" | "PERFBOOST" | "ACTIVELOCKSCREEN"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const S3: &str = "\
The following sleep states are available on this system:
    Standby (S3)
    Hibernate

The following sleep states are not available on this system:
    Standby (S0 Low Power Idle)
    Standby (S1)
";

    const AOAC: &str = "\
The following sleep states are available on this system:
    Standby (S0 Low Power Idle) Network Connected
    Hibernate

The following sleep states are not available on this system:
    Standby (S3)
";

    #[test]
    fn s3_hibernate_and_modern_standby_come_from_the_available_section() {
        let s3 = parse_powercfg_a(S3);
        assert!(s3.known);
        assert!(s3.s3);
        assert!(s3.hibernate);
        assert!(!s3.modern_standby);
        assert_eq!(s3.sleep_label(), "s3");

        let aoac = parse_powercfg_a(AOAC);
        assert!(aoac.modern_standby);
        assert!(!aoac.s3);
        assert_eq!(aoac.sleep_label(), "modern-standby");
        assert!(aoac.any_sleep());
    }

    #[test]
    fn an_empty_or_unrecognized_report_is_unknown() {
        let empty = parse_powercfg_a("");
        assert!(!empty.known);
        assert_eq!(empty.sleep_label(), "unknown");
        assert!(!empty.any_sleep());
        let none = parse_powercfg_a(
            "The following sleep states are available on this system:\n\n\
             The following sleep states are not available on this system:\n    Standby (S3)\n",
        );
        assert!(none.known);
        assert_eq!(none.sleep_label(), "none");
    }

    #[test]
    fn german_headers_still_split_available_from_unavailable() {
        let text = "\
Die folgenden Ruhezustände sind auf diesem System verfügbar:
    Standby (S3)
    Ruhezustand

Die folgenden Ruhezustände sind auf diesem System nicht verfügbar:
    Standby (S0-Energiezustand mit geringem Stromverbrauch)
";
        let parsed = parse_powercfg_a(text);
        assert!(parsed.s3);
        assert!(parsed.hibernate);
        assert!(!parsed.modern_standby);
    }

    #[test]
    fn request_sections_do_not_leak_into_each_other() {
        let text = "\
DISPLAY:
None.

SYSTEM:
[PROCESS] \\Device\\HarddiskVolume3\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe
SoftLock hardware suite

AWAYMODE:
None.
";
        assert!(!section_contains(
            text,
            "DISPLAY",
            "SoftLock hardware suite"
        ));
        assert!(section_contains(text, "SYSTEM", "SoftLock hardware suite"));
        assert!(!section_contains(
            text,
            "AWAYMODE",
            "SoftLock hardware suite"
        ));
        assert!(!section_contains(text, "SYSTEM", ""));
    }
}
