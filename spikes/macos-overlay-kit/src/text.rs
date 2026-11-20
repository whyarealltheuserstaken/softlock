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
    fn control_characters_become_spaces() {
        assert_eq!(one_line("  overlay\nheld\t"), "overlay held");
    }
}
