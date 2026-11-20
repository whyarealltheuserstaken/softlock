//! Accessibility and Input Monitoring skip text. Same wording as the hardware suite.

pub fn input_note(accessibility: bool, listen: bool) -> String {
    let mut missing = Vec::new();
    if !accessibility {
        missing.push("Accessibility");
    }
    if !listen {
        missing.push("Input Monitoring");
    }
    if missing.is_empty() {
        return String::new();
    }
    let names = missing.join(" and ");
    format!("missing {names}; grant {names}, then restart Terminal")
}

#[cfg(test)]
mod tests {
    use super::input_note;

    #[test]
    fn missing_grants_name_the_permission_and_say_to_restart_terminal() {
        let both = input_note(false, false);
        assert!(both.contains("Accessibility"));
        assert!(both.contains("Input Monitoring"));
        assert!(both.contains("restart Terminal"));
        let ax = input_note(false, true);
        assert!(ax.contains("Accessibility"));
        assert!(!ax.contains("Input Monitoring"));
        assert!(ax.contains("restart Terminal"));
        let listen = input_note(true, false);
        assert!(listen.contains("Input Monitoring"));
        assert!(!listen.contains("Accessibility"));
        assert!(input_note(true, true).is_empty());
    }
}
