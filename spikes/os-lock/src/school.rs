//! School-workstation identity. No operating-system calls.

#![cfg_attr(not(windows), allow(dead_code))]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchoolCheck {
    No,
    Yes,
    Unreadable,
}

/// `family`, `version`, and `product` are the BIOS strings. A missing or
/// empty string cannot prove this is not the school machine, so the caller
/// refuses.
pub fn classify(family: Option<&str>, version: Option<&str>, product: Option<&str>) -> SchoolCheck {
    let (Some(family), Some(version), Some(product)) = (family, version, product) else {
        return SchoolCheck::Unreadable;
    };
    if family.is_empty() || version.is_empty() || product.is_empty() {
        return SchoolCheck::Unreadable;
    }
    if is_school(family) || is_school(version) || is_school(product) {
        SchoolCheck::Yes
    } else {
        SchoolCheck::No
    }
}

fn is_school(value: &str) -> bool {
    value.to_ascii_lowercase().contains("thinkstation p7") || has_mtm(value)
}

/// Lenovo machine-type codes. Matched on alphanumeric tokens so a longer
/// product string such as `30F2S2US00` still matches the `30F2` prefix.
fn has_mtm(value: &str) -> bool {
    let upper = value.to_ascii_uppercase();
    upper
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| token.starts_with("30F2") || token.starts_with("30F3"))
}

/// The process user and the console session user. Empty or missing names are
/// unreadable and the caller refuses.
pub fn accounts_match(process_user: Option<&str>, console_user: Option<&str>) -> Result<bool, ()> {
    let (Some(process_user), Some(console_user)) = (process_user, console_user) else {
        return Err(());
    };
    let process_user = process_user.trim();
    let console_user = console_user.trim();
    if process_user.is_empty() || console_user.is_empty() {
        return Err(());
    }
    if process_user.eq_ignore_ascii_case(console_user) {
        return Ok(true);
    }
    let process_tail = process_user.rsplit('\\').next().unwrap_or(process_user);
    let console_tail = console_user.rsplit('\\').next().unwrap_or(console_user);
    Ok(!process_tail.is_empty() && process_tail.eq_ignore_ascii_case(console_tail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_benign_machine_is_not_the_school_workstation() {
        assert_eq!(
            classify(
                Some("Virtual Machine"),
                Some("Hyper-V"),
                Some("Virtual Machine")
            ),
            SchoolCheck::No
        );
    }

    #[test]
    fn the_model_name_matches_in_any_of_the_three_bios_strings() {
        assert_eq!(
            classify(Some("ThinkStation P7"), Some("1.0"), Some("Other")),
            SchoolCheck::Yes
        );
        assert_eq!(
            classify(Some("Other"), Some("thinkstation p7"), Some("Other")),
            SchoolCheck::Yes
        );
        assert_eq!(
            classify(Some("Other"), Some("1.0"), Some("ThinkStation P7")),
            SchoolCheck::Yes
        );
    }

    #[test]
    fn lenovo_machine_type_prefixes_match() {
        assert_eq!(
            classify(Some("Other"), Some("1.0"), Some("30F2S2US00")),
            SchoolCheck::Yes
        );
        assert_eq!(
            classify(Some("30F3"), Some("1.0"), Some("Other")),
            SchoolCheck::Yes
        );
        assert_eq!(
            classify(Some("Other"), Some("SKU 30F3CTO1WW"), Some("Other")),
            SchoolCheck::Yes
        );
    }

    #[test]
    fn a_digit_prefix_inside_a_longer_token_does_not_match() {
        assert_eq!(
            classify(Some("Other"), Some("1.0"), Some("X30F2")),
            SchoolCheck::No
        );
    }

    #[test]
    fn a_missing_or_empty_bios_string_is_unreadable() {
        assert_eq!(
            classify(None, Some("1.0"), Some("Other")),
            SchoolCheck::Unreadable
        );
        assert_eq!(
            classify(Some("Other"), None, Some("Other")),
            SchoolCheck::Unreadable
        );
        assert_eq!(
            classify(Some("Other"), Some("1.0"), None),
            SchoolCheck::Unreadable
        );
        assert_eq!(
            classify(Some(""), Some("1.0"), Some("Other")),
            SchoolCheck::Unreadable
        );
    }

    #[test]
    fn account_names_match_with_or_without_a_domain() {
        assert_eq!(accounts_match(Some("Alice"), Some("alice")), Ok(true));
        assert_eq!(
            accounts_match(Some(r"DOMAIN\Alice"), Some("alice")),
            Ok(true)
        );
        assert_eq!(accounts_match(Some("Alice"), Some("Bob")), Ok(false));
    }

    #[test]
    fn a_missing_account_name_is_unreadable() {
        assert_eq!(accounts_match(None, Some("Alice")), Err(()));
        assert_eq!(accounts_match(Some("Alice"), None), Err(()));
        assert_eq!(accounts_match(Some("  "), Some("Alice")), Err(()));
    }
}
