//! The probe installs a SYSTEM service. That is allowed in GitHub Actions, or with an explicit opt-in.

pub const LOCAL_REFUSAL: &str = "CI only. Do not run this probe on a local machine. It installs a SYSTEM service and changes lockdown policy values. GitHub Actions sets GITHUB_ACTIONS=true. Elsewhere, pass --allow-local-service only on a machine whose policies you can restore. The journal restores them on exit, on Ctrl+C, and after a crash.";

pub fn service_install_allowed(
    github_actions: Option<&str>,
    allow_local: bool,
) -> Result<(), &'static str> {
    if github_actions == Some("true") || allow_local {
        Ok(())
    } else {
        Err(LOCAL_REFUSAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_actions_may_install_the_service() {
        assert!(service_install_allowed(Some("true"), false).is_ok());
    }

    #[test]
    fn a_local_run_is_refused_without_the_opt_in() {
        assert_eq!(service_install_allowed(None, false), Err(LOCAL_REFUSAL));
        assert_eq!(
            service_install_allowed(Some("false"), false),
            Err(LOCAL_REFUSAL)
        );
    }

    #[test]
    fn the_opt_in_flag_allows_a_local_run() {
        assert!(service_install_allowed(None, true).is_ok());
    }
}
