//! One-word physical answers.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Answer {
    Go,
    Skip,
}

/// Empty / Enter / yes / y / ok → Go. skip / n / no / s → Skip.
pub fn parse_answer(line: &str) -> Result<Answer, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(Answer::Go);
    }
    let word = trimmed.split_whitespace().next().unwrap_or("");
    let lower = word.to_ascii_lowercase();
    match lower.as_str() {
        "y" | "yes" | "ok" | "enter" | "go" => Ok(Answer::Go),
        "skip" | "s" | "n" | "no" => Ok(Answer::Skip),
        other => Err(format!("need one word (Enter, skip), not {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Answer, parse_answer};

    #[test]
    fn enter_means_go() {
        assert_eq!(parse_answer("").unwrap(), Answer::Go);
        assert_eq!(parse_answer("  \n").unwrap(), Answer::Go);
        assert_eq!(parse_answer("yes").unwrap(), Answer::Go);
    }

    #[test]
    fn skip_words() {
        assert_eq!(parse_answer("skip").unwrap(), Answer::Skip);
        assert_eq!(parse_answer("SKIP").unwrap(), Answer::Skip);
        assert_eq!(parse_answer("n").unwrap(), Answer::Skip);
    }
}
