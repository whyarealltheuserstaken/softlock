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

/// Refuse when this window is not the person at the keyboard.
///
/// An unknown console user is refused as well. The program does not change
/// settings until it can see that person.
pub fn same_account(current: &Account, console: Option<&Account>) -> Result<(), Error> {
    let Some(console) = console else {
        return Err(Error::new(
            "Could not see who is at the keyboard. Nothing was changed. Open Terminal (Admin) from your own account and click Yes. If Windows asks for a password instead of just Yes, stop.",
        ));
    };
    if current.sid.eq_ignore_ascii_case(&console.sid) {
        return Ok(());
    }
    Err(Error::new(format!(
        "This window is running as {current}. The person at the keyboard is {console}. Nothing was changed. Open Terminal (Admin) from your own account and click Yes. If Windows asks for a password instead of just Yes, stop."
    )))
}
