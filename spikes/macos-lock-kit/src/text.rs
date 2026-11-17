//! Log lines stay on one line.

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
    fn control_characters_become_spaces_and_the_line_is_capped() {
        assert_eq!(one_line("  status\nline\t"), "status line");
        assert_eq!(one_line(&"a".repeat(450)).chars().count(), 400);
    }
}
