//! Hosted-runner gate for the SYSTEM session reconnect. No operating-system calls.

#![cfg_attr(not(windows), allow(dead_code))]

/// True only for a GitHub-hosted Actions runner. `--allow-local` is not an input.
pub fn hosted_runner(github_actions: Option<&str>, runner_environment: Option<&str>) -> bool {
    github_actions == Some("true") && runner_environment == Some("github-hosted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_github_hosted_runner_may_reconnect() {
        assert!(hosted_runner(Some("true"), Some("github-hosted")));
    }

    #[test]
    fn a_self_hosted_or_local_process_may_not_reconnect() {
        assert!(!hosted_runner(Some("true"), Some("self-hosted")));
        assert!(!hosted_runner(Some("true"), None));
        assert!(!hosted_runner(None, Some("github-hosted")));
        assert!(!hosted_runner(Some("false"), Some("github-hosted")));
        assert!(!hosted_runner(Some(""), Some("github-hosted")));
    }
}
