//! Fail-closed helper protocol.
//!
//! The helper blocks on stdin. `complete` means the parent finished and the
//! helper must not lock. End of input without that line means the parent died.
//! The helper still has to pass the kit gate before it locks, and it never
//! unlocks.

pub const COMPLETE_LINE: &str = "complete";
pub const HELPER_ENV: &str = "MACOS_LOCK_KIT_HELPER";
pub const HELPER_NONCE_ENV: &str = "MACOS_LOCK_KIT_HELPER_NONCE";
pub const HELPER_RESULT: &str = "helper-result.txt";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperAction {
    Private,
    Documented,
    None,
}

impl HelperAction {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "private" => Some(Self::Private),
            "documented" => Some(Self::Documented),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Documented => "documented",
            Self::None => "none",
        }
    }
}

pub fn parent_completed(stdin_text: &str) -> bool {
    stdin_text.lines().any(|line| line.trim() == COMPLETE_LINE)
}

/// The helper locks only when the parent did not finish and a real path was armed.
pub fn helper_should_lock(action: HelperAction, stdin_text: &str, gate_allows_lock: bool) -> bool {
    gate_allows_lock && !parent_completed(stdin_text) && action != HelperAction::None
}

/// `helper-end` counts only when the file carries this run's nonce.
pub fn helper_end_for_nonce(text: &str, nonce: &str) -> Option<String> {
    if nonce.is_empty() || nonce.chars().any(|ch| !ch.is_ascii_hexdigit()) {
        return None;
    }
    let mut end = None;
    let mut nonce_ok = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("helper-end=") {
            if !value.is_empty() {
                end = Some(value.to_string());
            }
        } else if let Some(value) = line.strip_prefix("nonce=") {
            nonce_ok = value == nonce;
        }
    }
    if nonce_ok { end } else { None }
}

/// Write the helper result beside its destination, then rename it into place.
pub fn write_helper_result(dir: &std::path::Path, body: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let dest = dir.join(HELPER_RESULT);
    let tmp = dir.join(format!(".helper-result.{}.tmp", std::process::id()));
    std::fs::write(&tmp, body)?;
    if let Err(error) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    Ok(())
}

pub fn fresh_nonce() -> Option<String> {
    let mut bytes = [0u8; 16];
    let mut file = std::fs::File::open("/dev/urandom").ok()?;
    std::io::Read::read_exact(&mut file, &mut bytes).ok()?;
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Remove a previous helper result. Missing is fine. Anything else refuses to arm.
pub fn prepare_helper_slot(dir: &std::path::Path) -> Option<String> {
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join(HELPER_RESULT);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return None,
    }
    fresh_nonce()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_stands_down_and_a_dropped_parent_does_not_unlock() {
        assert!(parent_completed("complete\n"));
        assert!(!parent_completed(""));
        assert!(!parent_completed("error\n"));
        assert!(!helper_should_lock(
            HelperAction::Private,
            "complete\n",
            true
        ));
        assert!(helper_should_lock(HelperAction::Private, "", true));
        assert!(helper_should_lock(
            HelperAction::Documented,
            "error\n",
            true
        ));
        assert!(!helper_should_lock(HelperAction::None, "", true));
        assert!(!helper_should_lock(HelperAction::Private, "", false));
    }

    #[test]
    fn a_stale_result_with_the_wrong_nonce_is_not_os_locked() {
        let dir =
            std::env::temp_dir().join(format!("macos-lock-kit-stale-nonce-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let stale = "helper-end=lock-private\nnonce=stale\n";
        std::fs::write(dir.join(HELPER_RESULT), stale).expect("stale file");
        let text = std::fs::read_to_string(dir.join(HELPER_RESULT)).expect("read");
        let end = helper_end_for_nonce(&text, "abcdef");
        assert!(end.is_none(), "{text}");
        assert_eq!(
            crate::lock_attempt::signal_state(false, end.as_deref()),
            "helper-unread"
        );
        assert_ne!(
            crate::lock_attempt::signal_state(false, end.as_deref()),
            "os-locked"
        );
        let nonce = prepare_helper_slot(&dir).expect("nonce");
        assert!(!dir.join(HELPER_RESULT).exists());
        assert_ne!(nonce, "stale");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_helper_result_is_renamed_into_place_with_its_nonce() {
        let dir =
            std::env::temp_dir().join(format!("macos-lock-kit-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_helper_result(&dir, "helper-end=lock-private\nnonce=abc\n").expect("write");
        let text = std::fs::read_to_string(dir.join(HELPER_RESULT)).expect("read");
        assert_eq!(
            helper_end_for_nonce(&text, "abc").as_deref(),
            Some("lock-private")
        );
        assert!(helper_end_for_nonce(&text, "def").is_none());
        assert!(
            !dir.join(format!(".helper-result.{}.tmp", std::process::id()))
                .exists()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
