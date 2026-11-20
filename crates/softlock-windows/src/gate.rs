//! Hosted-runner and school-workstation gates. No operating-system calls.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchoolCheck {
    No,
    Yes,
    Unreadable,
}

/// `family`, `version`, and `product` are the BIOS strings. A missing or
/// empty string cannot prove this is not the school machine.
pub fn classify_school(
    family: Option<&str>,
    version: Option<&str>,
    product: Option<&str>,
) -> SchoolCheck {
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
    value.to_ascii_lowercase().contains("thinkstation p7") || has_machine_type(value)
}

fn has_machine_type(value: &str) -> bool {
    let upper = value.to_ascii_uppercase();
    upper
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| token.starts_with("30F2") || token.starts_with("30F3"))
}

/// True only for a GitHub-hosted Actions runner.
pub fn hosted_runner(github_actions: Option<&str>, runner_environment: Option<&str>) -> bool {
    github_actions == Some("true") && runner_environment == Some("github-hosted")
}

/// The confirm job refuses anything that is not a hosted runner, and it
/// refuses the school workstation even when those variables are set.
pub fn proof_allowed(hosted: bool, school: SchoolCheck) -> Result<(), &'static str> {
    if !hosted {
        return Err("not-github-hosted");
    }
    match school {
        SchoolCheck::No => Ok(()),
        SchoolCheck::Yes => Err("school-workstation"),
        SchoolCheck::Unreadable => Err("identity-unreadable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_benign_machine_is_not_the_school_workstation() {
        assert_eq!(
            classify_school(
                Some("Virtual Machine"),
                Some("Hyper-V"),
                Some("Virtual Machine")
            ),
            SchoolCheck::No
        );
    }

    #[test]
    fn the_model_name_or_machine_type_is_the_school_workstation() {
        assert_eq!(
            classify_school(Some("ThinkStation P7"), Some("1.0"), Some("Other")),
            SchoolCheck::Yes
        );
        assert_eq!(
            classify_school(Some("Other"), Some("30F2S2US00"), Some("Other")),
            SchoolCheck::Yes
        );
        assert_eq!(
            classify_school(Some("Other"), Some("1.0"), Some("30F3")),
            SchoolCheck::Yes
        );
    }

    #[test]
    fn a_missing_bios_string_is_unreadable() {
        assert_eq!(
            classify_school(None, Some("1.0"), Some("Other")),
            SchoolCheck::Unreadable
        );
        assert_eq!(
            classify_school(Some(""), Some("1.0"), Some("Other")),
            SchoolCheck::Unreadable
        );
    }

    #[test]
    fn only_a_github_hosted_runner_may_run_the_confirm_job() {
        assert!(hosted_runner(Some("true"), Some("github-hosted")));
        assert!(!hosted_runner(Some("true"), Some("self-hosted")));
        assert!(!hosted_runner(Some("true"), None));
        assert!(!hosted_runner(None, Some("github-hosted")));
        assert!(!hosted_runner(Some("false"), Some("github-hosted")));
    }

    #[test]
    fn the_school_workstation_is_refused_even_on_a_hosted_runner() {
        assert!(proof_allowed(true, SchoolCheck::No).is_ok());
        assert_eq!(
            proof_allowed(true, SchoolCheck::Yes),
            Err("school-workstation")
        );
        assert_eq!(
            proof_allowed(true, SchoolCheck::Unreadable),
            Err("identity-unreadable")
        );
        assert_eq!(
            proof_allowed(false, SchoolCheck::No),
            Err("not-github-hosted")
        );
    }
}
