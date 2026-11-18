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

/// AC and DC indexes for one power setting. The values come from the setting's
/// own block, so a subgroup dump cannot report a different button's index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingIndex {
    pub ac: u32,
    pub dc: u32,
}

/// Per-user `ACSettingIndex` and `DCSettingIndex` are independent. A missing
/// side falls back to that side's scheme default. Either side still missing
/// means the index is unknown.
pub fn resolve_power_index(
    user_ac: Option<u32>,
    user_dc: Option<u32>,
    default_ac: Option<u32>,
    default_dc: Option<u32>,
) -> Option<SettingIndex> {
    Some(SettingIndex {
        ac: user_ac.or(default_ac)?,
        dc: user_dc.or(default_dc)?,
    })
}

/// `powercfg /q` prints the current indexes as `0x........` after any minimum
/// or maximum. Labels are localized, so the GUID and those hex values are the
/// whole parse. Hidden settings omitted from the dump are not found.
pub fn parse_setting_index(text: &str, setting: &str) -> Option<SettingIndex> {
    let lower = text.to_ascii_lowercase();
    let needle = setting
        .trim()
        .trim_matches(|ch| ch == '{' || ch == '}')
        .to_ascii_lowercase();
    if needle.is_empty() {
        return None;
    }
    let start = lower.find(&needle)?;
    let rest = &text[start + needle.len()..];
    let end = next_guid_at(&rest.to_ascii_lowercase()).unwrap_or(rest.len());
    let values = hex_values(&rest[..end]);
    if values.len() < 2 {
        return None;
    }
    Some(SettingIndex {
        ac: values[values.len() - 2],
        dc: values[values.len() - 1],
    })
}

/// The first GUID in `powercfg /GETACTIVESCHEME`. The label around it changes
/// with the Windows language.
pub fn parse_scheme_guid(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let at = next_guid_at(&lower)?;
    Some(lower[at..at + 36].to_string())
}

fn next_guid_at(lower: &str) -> Option<usize> {
    let bytes = lower.as_bytes();
    let mut index = 0;
    while index + 36 <= bytes.len() {
        if is_guid(&lower[index..index + 36]) {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn is_guid(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        let dash = index == 8 || index == 13 || index == 18 || index == 23;
        if dash {
            if *byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn hex_values(text: &str) -> Vec<u32> {
    let bytes = text.as_bytes();
    let mut values = Vec::new();
    let mut index = 0;
    while index + 2 < bytes.len() {
        if bytes[index] == b'0' && (bytes[index + 1] == b'x' || bytes[index + 1] == b'X') {
            let start = index + 2;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                end += 1;
            }
            if end > start
                && let Ok(value) = u32::from_str_radix(&text[start..end], 16)
            {
                values.push(value);
            }
            index = end;
            continue;
        }
        index += 1;
    }
    values
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
hardware suite

AWAYMODE:
None.
";
        assert!(!section_contains(text, "DISPLAY", "hardware suite"));
        assert!(section_contains(text, "SYSTEM", "hardware suite"));
        assert!(!section_contains(text, "AWAYMODE", "hardware suite"));
        assert!(!section_contains(text, "SYSTEM", ""));
    }

    /// Published `powercfg /query SCHEME_CURRENT SUB_BUTTONS` text. The lid
    /// block is the query output recorded for this subgroup; the power-button
    /// block is the same format with that setting's documented GUID.
    const SUB_BUTTONS: &str = "\
Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)
  GUID Alias: SCHEME_BALANCED
  Subgroup GUID: 4f971e89-eebd-4455-a8de-9e59040e7347  (Power buttons and lid)
    GUID Alias: SUB_BUTTONS
    Power Setting GUID: 5ca83367-6e45-459f-a27b-476b1d01c936  (Lid close action)
      GUID Alias: LIDACTION
      Possible Setting Index: 000
      Possible Setting Friendly Name: Do nothing
      Possible Setting Index: 001
      Possible Setting Friendly Name: Sleep
      Possible Setting Index: 002
      Possible Setting Friendly Name: Hibernate
      Possible Setting Index: 003
      Possible Setting Friendly Name: Shut down
    Current AC Power Setting Index: 0x00000001
    Current DC Power Setting Index: 0x00000001

    Power Setting GUID: 7648efa3-dd9c-4e3e-b566-50f929386280  (Power button action)
      GUID Alias: PBUTTONACTION
      Possible Setting Index: 000
      Possible Setting Friendly Name: Do nothing
      Possible Setting Index: 001
      Possible Setting Friendly Name: Sleep
      Possible Setting Index: 002
      Possible Setting Friendly Name: Hibernate
      Possible Setting Index: 003
      Possible Setting Friendly Name: Shut down
    Current AC Power Setting Index: 0x00000003
    Current DC Power Setting Index: 0x00000001
";

    #[test]
    fn a_subgroup_query_keeps_each_setting_s_own_ac_and_dc_index() {
        let lid = parse_setting_index(SUB_BUTTONS, "5ca83367-6e45-459f-a27b-476b1d01c936").unwrap();
        assert_eq!(lid, SettingIndex { ac: 1, dc: 1 });
        let button =
            parse_setting_index(SUB_BUTTONS, "7648efa3-dd9c-4e3e-b566-50f929386280").unwrap();
        assert_eq!(button, SettingIndex { ac: 3, dc: 1 });
        assert_eq!(
            parse_scheme_guid(SUB_BUTTONS).as_deref(),
            Some("381b4222-f694-41f0-9685-ff5bb260df2e")
        );
        assert_eq!(
            parse_scheme_guid(
                "Power Scheme GUID: 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c  (High performance)"
            )
            .as_deref(),
            Some("8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c")
        );
    }

    #[test]
    fn a_hidden_setting_omitted_from_the_query_is_not_another_buttons_index() {
        let visible = "\
Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)
  Subgroup GUID: 4f971e89-eebd-4455-a8de-9e59040e7347  (Power buttons and lid)
    Power Setting GUID: 7648efa3-dd9c-4e3e-b566-50f929386280  (Power button action)
    Current AC Power Setting Index: 0x00000001
    Current DC Power Setting Index: 0x00000001
";
        assert!(parse_setting_index(visible, "5ca83367-6e45-459f-a27b-476b1d01c936").is_none());
        let button =
            parse_setting_index(visible, "{7648efa3-dd9c-4e3e-b566-50f929386280}").unwrap();
        assert_eq!(button.ac, 1);
    }

    #[test]
    fn localized_labels_and_minimum_values_do_not_move_the_current_index() {
        let german = "\
Energieschema-GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Ausbalanciert)
    Energieeinstellungs-GUID: 5ca83367-6e45-459f-a27b-476b1d01c936
      Minimum Possible Setting: 0x00000000
      Maximum Possible Setting: 0x00000003
    Aktueller Wechselstrom-Energieeinstellungsindex: 0x00000002
    Aktueller Gleichstrom-Energieeinstellungsindex: 0x00000000
";
        let lid = parse_setting_index(german, "5CA83367-6E45-459F-A27B-476B1D01C936").unwrap();
        assert_eq!(lid, SettingIndex { ac: 2, dc: 0 });
        assert!(parse_scheme_guid("").is_none());
        assert!(parse_setting_index(german, "7648efa3-dd9c-4e3e-b566-50f929386280").is_none());
    }

    #[test]
    fn each_power_index_falls_back_on_its_own() {
        assert_eq!(
            resolve_power_index(Some(3), None, Some(1), Some(2)),
            Some(SettingIndex { ac: 3, dc: 2 })
        );
        assert_eq!(
            resolve_power_index(None, Some(0), Some(1), Some(2)),
            Some(SettingIndex { ac: 1, dc: 0 })
        );
        assert_eq!(
            resolve_power_index(Some(3), Some(0), Some(1), Some(2)),
            Some(SettingIndex { ac: 3, dc: 0 })
        );
        assert_eq!(
            resolve_power_index(None, None, Some(1), Some(2)),
            Some(SettingIndex { ac: 1, dc: 2 })
        );
        assert_eq!(
            resolve_power_index(Some(3), None, None, Some(2)),
            Some(SettingIndex { ac: 3, dc: 2 })
        );
        assert_eq!(
            resolve_power_index(None, Some(0), Some(1), None),
            Some(SettingIndex { ac: 1, dc: 0 })
        );
        assert_eq!(resolve_power_index(Some(3), None, None, None), None);
        assert_eq!(resolve_power_index(None, None, None, Some(2)), None);
        assert_eq!(resolve_power_index(None, None, Some(1), None), None);
    }
}
