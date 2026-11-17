//! One-word physical answers. Stdin is polled so Ctrl-C does not wait for Enter.

use std::os::unix::io::RawFd;

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

const POLL_MS: i32 = 100;

/// Poll `fd` and `read` until a line, EOF, or the interrupt flag.
/// Rust's `read_line` retries on EINTR, so it cannot see Ctrl-C.
pub fn read_line_interruptible(fd: RawFd, interrupted: impl Fn() -> bool) -> PromptRead {
    if interrupted() {
        return PromptRead::Stop;
    }
    let mut buf = Vec::new();
    loop {
        if interrupted() {
            return PromptRead::Stop;
        }
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pollfd, 1, POLL_MS) };
        if interrupted() {
            return PromptRead::Stop;
        }
        if ready < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return PromptRead::Skip("no-input".into());
        }
        if ready == 0 {
            continue;
        }
        let hangup = pollfd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0;
        if hangup && pollfd.revents & libc::POLLIN == 0 {
            return interpret_read(interrupted(), Ok(buf.len()), &lossy(&buf));
        }
        if pollfd.revents & libc::POLLIN == 0 {
            continue;
        }
        let mut tmp = [0u8; 256];
        let n = unsafe { libc::read(fd, tmp.as_mut_ptr().cast(), tmp.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return PromptRead::Skip("no-input".into());
        }
        if n == 0 {
            return interpret_read(interrupted(), Ok(buf.len()), &lossy(&buf));
        }
        buf.extend_from_slice(&tmp[..n as usize]);
        if buf.contains(&b'\n') {
            return interpret_read(interrupted(), Ok(buf.len()), &lossy(&buf));
        }
    }
}

fn lossy(buf: &[u8]) -> String {
    String::from_utf8_lossy(buf).into_owned()
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
    use super::{Answer, PromptRead, interpret_read, parse_answer, read_line_interruptible};
    use std::os::unix::io::AsRawFd;
    use std::sync::atomic::{AtomicBool, Ordering};

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

    #[test]
    fn poll_sees_the_interrupt_flag_without_enter() {
        let flag = AtomicBool::new(true);
        let file = std::fs::File::open("/dev/null").expect("null");
        let decision = read_line_interruptible(file.as_raw_fd(), || flag.load(Ordering::SeqCst));
        assert_eq!(decision, PromptRead::Stop);
    }
}
