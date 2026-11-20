//! Runner proof versus the interactive kit.

use std::path::{Path, PathBuf};

use crate::wait::DEFAULT_TIMEOUT_MS;

pub const DEFAULT_OUTPUT_DIR: &str = "overlay-output";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Runner,
    Kit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    pub output_dir: PathBuf,
    pub spaces: Option<String>,
    pub fullscreen: Option<String>,
    pub click: Option<String>,
    pub focus: Option<String>,
    pub command_tab: Option<String>,
    pub force_quit: Option<String>,
    pub power_key: Option<String>,
    pub hotplug: Option<String>,
    pub timeout_ms: u64,
}

pub fn parse_env() -> Result<Args, String> {
    parse(std::env::args().skip(1))
}

pub fn parse<I>(tokens: I) -> Result<Args, String>
where
    I: IntoIterator<Item = String>,
{
    let mut runner = false;
    let mut kit = false;
    let mut output_dir = PathBuf::from(DEFAULT_OUTPUT_DIR);
    let mut spaces = None;
    let mut fullscreen = None;
    let mut click = None;
    let mut focus = None;
    let mut command_tab = None;
    let mut force_quit = None;
    let mut power_key = None;
    let mut hotplug = None;
    let mut timeout_ms = DEFAULT_TIMEOUT_MS;
    let mut tokens = tokens.into_iter();
    while let Some(token) = tokens.next() {
        match token.as_str() {
            "--runner" => runner = true,
            "--kit" => kit = true,
            "--output-dir" => {
                output_dir = PathBuf::from(tokens.next().ok_or("--output-dir needs a value")?);
            }
            "--spaces" => {
                spaces = Some(require_choice(
                    tokens.next().ok_or("--spaces needs a value")?,
                    &["stayed-up", "covered-by-space", "miss", "skip"],
                    "--spaces",
                )?);
            }
            "--fullscreen" => {
                fullscreen = Some(require_choice(
                    tokens.next().ok_or("--fullscreen needs a value")?,
                    &["stayed-up", "covered-by-fullscreen", "miss", "skip"],
                    "--fullscreen",
                )?);
            }
            "--click" => {
                click = Some(require_choice(
                    tokens.next().ok_or("--click needs a value")?,
                    &["fell-through", "blocked", "skip"],
                    "--click",
                )?);
            }
            "--focus" => {
                focus = Some(require_choice(
                    tokens.next().ok_or("--focus needs a value")?,
                    &["held", "moved", "skip"],
                    "--focus",
                )?);
            }
            "--command-tab" => {
                command_tab = Some(require_choice(
                    tokens.next().ok_or("--command-tab needs a value")?,
                    &[
                        "swallowed",
                        "presentation-option-worked",
                        "desktop-reachable",
                        "skip",
                    ],
                    "--command-tab",
                )?);
            }
            "--force-quit" => {
                force_quit = Some(require_choice(
                    tokens.next().ok_or("--force-quit needs a value")?,
                    &[
                        "swallowed",
                        "presentation-option-worked",
                        "desktop-reachable",
                        "skip",
                    ],
                    "--force-quit",
                )?);
            }
            "--power-key" => {
                power_key = Some(require_choice(
                    tokens.next().ok_or("--power-key needs a value")?,
                    &[
                        "swallowed",
                        "presentation-option-worked",
                        "desktop-reachable",
                        "skip",
                    ],
                    "--power-key",
                )?);
            }
            "--hotplug" => {
                hotplug = Some(require_choice(
                    tokens.next().ok_or("--hotplug needs a value")?,
                    &["covered", "miss", "no-external-display", "skip"],
                    "--hotplug",
                )?);
            }
            "--timeout-ms" => {
                timeout_ms = tokens
                    .next()
                    .ok_or("--timeout-ms needs a value")?
                    .parse()
                    .map_err(|_| "--timeout-ms needs a millisecond count".to_string())?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if runner && kit {
        return Err("--runner and --kit are different modes".to_string());
    }
    Ok(Args {
        mode: if kit { Mode::Kit } else { Mode::Runner },
        output_dir: absolute_dir(&output_dir),
        spaces,
        fullscreen,
        click,
        focus,
        command_tab,
        force_quit,
        power_key,
        hotplug,
        timeout_ms,
    })
}

fn require_choice(value: String, allowed: &[&str], flag: &str) -> Result<String, String> {
    if allowed.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(format!("{flag} must be one of {}", allowed.join(", ")))
    }
}

fn absolute_dir(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn the_default_mode_is_the_runner() {
        let args = parse(tokens(&[])).unwrap();
        assert_eq!(args.mode, Mode::Runner);
        assert!(args.output_dir.ends_with(DEFAULT_OUTPUT_DIR));
        assert_eq!(args.timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(
            parse(tokens(&["--timeout-ms", "50"])).unwrap().timeout_ms,
            50
        );
    }

    #[test]
    fn mixed_modes_and_unknown_rows_are_refused() {
        assert!(parse(tokens(&["--runner", "--kit"])).is_err());
        assert!(parse(tokens(&["--elsewhere"])).is_err());
        assert!(
            parse(tokens(&["--hotplug", "none"]))
                .unwrap_err()
                .contains("--hotplug")
        );
        let ok = parse(tokens(&[
            "--kit",
            "--spaces",
            "stayed-up",
            "--fullscreen",
            "covered-by-fullscreen",
            "--click",
            "fell-through",
            "--focus",
            "held",
            "--command-tab",
            "desktop-reachable",
            "--force-quit",
            "presentation-option-worked",
            "--power-key",
            "swallowed",
            "--hotplug",
            "no-external-display",
        ]))
        .unwrap();
        assert_eq!(ok.mode, Mode::Kit);
        assert_eq!(ok.hotplug.as_deref(), Some("no-external-display"));
        assert_eq!(ok.spaces.as_deref(), Some("stayed-up"));
    }
}
