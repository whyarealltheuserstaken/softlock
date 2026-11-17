//! One-word physical answers.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Answer {
    Go,
    Skip,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PromptRead {
    Go,
    Skip(String),
    Stop,
}

/// EOF is `no-input`, not Enter. An interrupt flag or EINTR stops the suite.
pub fn interpret_read(
    interrupted_flag: bool,
    read: Result<usize, std::io::ErrorKind>,
    line: &str,
) -> PromptRead {
    if interrupted_flag {
        return PromptRead::Stop;
    }
    match read {
        Ok(0) => PromptRead::Skip("no-input".to_string()),
        Ok(_) => match parse_answer(line) {
            Ok(Answer::Go) => PromptRead::Go,
            Ok(Answer::Skip) => PromptRead::Skip("operator".to_string()),
            Err(_) => PromptRead::Skip("operator".to_string()),
        },
        Err(std::io::ErrorKind::Interrupted) => PromptRead::Stop,
        Err(_) => PromptRead::Skip("no-input".to_string()),
    }
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
    use super::{Answer, PromptRead, interpret_read, parse_answer};

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

    #[test]
    fn eof_is_no_input_not_enter() {
        assert_eq!(
            interpret_read(false, Ok(0), ""),
            PromptRead::Skip("no-input".into())
        );
        assert_eq!(interpret_read(false, Ok(1), "\n"), PromptRead::Go);
        assert_eq!(interpret_read(true, Ok(1), "\n"), PromptRead::Stop);
        assert_eq!(
            interpret_read(false, Err(std::io::ErrorKind::Interrupted), ""),
            PromptRead::Stop
        );
    }
}
