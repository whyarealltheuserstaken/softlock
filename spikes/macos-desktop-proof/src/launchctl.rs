//! `launchctl print` text. The command itself runs on the runner.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintState {
    Absent,
    Loaded,
    Running,
}

pub fn parse_print(success: bool, stdout: &str) -> PrintState {
    if !success {
        return PrintState::Absent;
    }
    if stdout.lines().any(|line| line.trim() == "state = running") {
        PrintState::Running
    } else {
        PrintState::Loaded
    }
}

pub fn parse_pid(stdout: &str) -> Option<u32> {
    for line in stdout.lines() {
        let Some(rest) = line.trim().strip_prefix("pid = ") else {
            continue;
        };
        if let Ok(pid) = rest.trim().parse::<u32>()
            && pid > 0
        {
            return Some(pid);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_print_is_absent_even_if_the_text_mentions_running() {
        let text = "state = running\npid = 9\n";
        assert_eq!(parse_print(false, text), PrintState::Absent);
        assert_eq!(parse_pid(text), Some(9));
    }

    #[test]
    fn running_and_loaded_are_distinct() {
        let running = "gui/501/com.runner.macosproof.agent = {\n\tstate = running\n\tpid = 42\n}\n";
        assert_eq!(parse_print(true, running), PrintState::Running);
        assert_eq!(parse_pid(running), Some(42));
        assert_eq!(parse_print(true, "state = waiting\n"), PrintState::Loaded);
        assert_eq!(parse_pid("pid = 0\n"), None);
    }
}
