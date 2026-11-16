use crate::error::Error;

/// A Windows account this program can name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub sid: String,
    pub name: String,
}

impl std::fmt::Display for Account {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} ({})", self.name, self.sid)
    }
}

/// Refuse when the window's account is not the person at the keyboard.
///
/// `console` is `None` when that person cannot be identified. A known mismatch
/// is refused. An unknown console user is not treated as a mismatch.
pub fn same_account(current: &Account, console: Option<&Account>) -> Result<(), Error> {
    let Some(console) = console else {
        return Ok(());
    };
    if current.sid.eq_ignore_ascii_case(&console.sid) {
        return Ok(());
    }
    Err(Error::new(format!(
        "This window is running as {current}. The person at the keyboard is {console}. Nothing was changed. Open Terminal (Admin) from your own account and click Yes. If Windows asks for a password instead of just Yes, stop."
    )))
}
