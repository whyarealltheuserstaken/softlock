//! Log lines stay on one line and stay short.

pub fn one_line(text: &str) -> String {
    let mut out = String::new();
    for (count, ch) in text.chars().enumerate() {
        if count >= 400 {
            break;
        }
        if ch.is_control() {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_become_spaces_and_the_line_is_trimmed() {
        assert_eq!(one_line("  launchctl\nfailed\t"), "launchctl failed");
    }

    #[test]
    fn a_long_line_is_capped() {
        let text = "a".repeat(450);
        assert_eq!(one_line(&text).chars().count(), 400);
    }
}
