//! Logon name forms for the offline password check.
//!
//! The string passed to `LogonUserW` is the form printed by the tool.
//! A SAM fallback is the session account. It is not labelled local.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountKind {
    Microsoft,
    Explicit,
    Session,
}

impl AccountKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Microsoft => "microsoft",
            Self::Explicit => "explicit",
            Self::Session => "session",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogonAccount {
    pub kind: AccountKind,
    /// Exact `domain\user` string passed to the network logon.
    pub form: String,
    pub domain: String,
    pub user: String,
}

/// `MicrosoftAccount\<email>`, `.\user`, or `MACHINE\user`, as typed.
pub fn parse_explicit_account(raw: &str) -> Result<LogonAccount, String> {
    let form = raw.trim();
    let Some((domain, user)) = form.split_once('\\') else {
        return Err(account_usage());
    };
    if domain.is_empty() || user.is_empty() || user.contains('\\') {
        return Err(account_usage());
    }
    let kind = if domain.eq_ignore_ascii_case("MicrosoftAccount") {
        AccountKind::Microsoft
    } else {
        AccountKind::Explicit
    };
    Ok(LogonAccount {
        kind,
        form: form.to_string(),
        domain: domain.to_string(),
        user: user.to_string(),
    })
}

/// Choose the network-logon form for the signed-in account.
/// An email UPN becomes `MicrosoftAccount\<email>`. Anything else keeps the
/// SAM `domain\user` and is a session account, not a local account.
pub fn session_logon(upn: Option<&str>, sam: &str) -> Option<LogonAccount> {
    let (sam_domain, sam_user) = split_sam(sam)?;
    if let Some(email) = upn.map(str::trim).filter(|value| value.contains('@')) {
        return Some(LogonAccount {
            kind: AccountKind::Microsoft,
            form: format!("MicrosoftAccount\\{email}"),
            domain: "MicrosoftAccount".to_string(),
            user: email.to_string(),
        });
    }
    if sam_domain.eq_ignore_ascii_case("MicrosoftAccount") {
        return Some(LogonAccount {
            kind: AccountKind::Microsoft,
            form: format!("{sam_domain}\\{sam_user}"),
            domain: sam_domain,
            user: sam_user,
        });
    }
    Some(LogonAccount {
        kind: AccountKind::Session,
        form: format!("{sam_domain}\\{sam_user}"),
        domain: sam_domain,
        user: sam_user,
    })
}

fn split_sam(sam: &str) -> Option<(String, String)> {
    let (domain, user) = sam.trim().split_once('\\')?;
    if domain.is_empty() || user.is_empty() || user.contains('\\') {
        return None;
    }
    Some((domain.to_string(), user.to_string()))
}

fn account_usage() -> String {
    "account must look like MicrosoftAccount\\email, .\\user, or MACHINE\\user".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_forms_are_used_as_typed() {
        let local = parse_explicit_account(r".\slp6test").unwrap();
        assert_eq!(local.kind, AccountKind::Explicit);
        assert_eq!(local.form, r".\slp6test");
        assert_eq!(local.domain, ".");
        assert_eq!(local.user, "slp6test");
        assert_ne!(local.kind.label(), "local");

        let msa = parse_explicit_account(r"MicrosoftAccount\h@outlook.com").unwrap();
        assert_eq!(msa.kind, AccountKind::Microsoft);
        assert_eq!(msa.form, r"MicrosoftAccount\h@outlook.com");
        assert_eq!(msa.domain, "MicrosoftAccount");
        assert_eq!(msa.user, "h@outlook.com");

        let machine = parse_explicit_account(r"DESKTOP\slp6test").unwrap();
        assert_eq!(machine.kind, AccountKind::Explicit);
        assert_eq!(machine.form, r"DESKTOP\slp6test");
        assert_eq!(machine.domain, "DESKTOP");
        assert_eq!(machine.user, "slp6test");
    }

    #[test]
    fn a_broken_account_string_is_rejected() {
        assert!(parse_explicit_account("slp6test").is_err());
        assert!(parse_explicit_account(r"\slp6test").is_err());
        assert!(parse_explicit_account(r"DESKTOP\").is_err());
        assert!(parse_explicit_account(r"A\B\C").is_err());
        assert!(parse_explicit_account("").is_err());
    }

    #[test]
    fn an_email_upn_uses_the_microsoft_account_form() {
        let account = session_logon(Some("h@outlook.com"), r"DESKTOP\Henry").unwrap();
        assert_eq!(account.kind, AccountKind::Microsoft);
        assert_eq!(account.form, r"MicrosoftAccount\h@outlook.com");
        assert_eq!(account.domain, "MicrosoftAccount");
        assert_eq!(account.user, "h@outlook.com");
    }

    #[test]
    fn a_sam_fallback_is_not_called_local() {
        let account = session_logon(None, r"DESKTOP\Henry").unwrap();
        assert_eq!(account.kind, AccountKind::Session);
        assert_eq!(account.form, r"DESKTOP\Henry");
        assert_eq!(account.domain, "DESKTOP");
        assert_eq!(account.user, "Henry");
        assert_ne!(account.kind.label(), "local");
    }

    #[test]
    fn a_microsoft_sam_stays_microsoft() {
        let account = session_logon(None, r"MicrosoftAccount\h@outlook.com").unwrap();
        assert_eq!(account.kind, AccountKind::Microsoft);
        assert_eq!(account.form, r"MicrosoftAccount\h@outlook.com");
    }
}
