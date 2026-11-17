//! Runner proof versus the interactive kit. Children never hold assertions.

use std::path::{Path, PathBuf};

pub const REASON_PREFIX: &str = "m07-keep-awake-probe";
pub const DEFAULT_INTERVAL_MS: u64 = 50;
pub const RUNNER_SAMPLE_MS: u64 = 8_000;
pub const KIT_SAMPLE_MS: u64 = 60_000;
pub const APP_NAP_SAMPLE_MS: u64 = 60_000;
pub const DEFAULT_OUTPUT_DIR: &str = "keep-awake-output";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Runner,
    Kit,
    Timer,
    Overlay,
    Heartbeat,
}

impl Mode {
    pub fn holds_assertions(self) -> bool {
        matches!(self, Self::Runner | Self::Kit)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    pub output_dir: PathBuf,
    pub sample_ms: u64,
    pub interval_ms: u64,
    pub lid_on_power: Option<String>,
    pub lid_external: Option<String>,
    pub power_key: Option<String>,
    pub count_file: Option<String>,
    pub cover_frame: Option<(i32, i32, i32, i32)>,
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
    let mut role = None;
    let mut output_dir = PathBuf::from(DEFAULT_OUTPUT_DIR);
    let mut sample_ms = None;
    let mut interval_ms = DEFAULT_INTERVAL_MS;
    let mut lid_on_power = None;
    let mut lid_external = None;
    let mut power_key = None;
    let mut count_file = None;
    let mut cover_frame = None;
    let mut tokens = tokens.into_iter();
    while let Some(token) = tokens.next() {
        match token.as_str() {
            "--runner" => runner = true,
            "--kit" => kit = true,
            "--role" => {
                let value = tokens.next().ok_or("--role needs a value")?;
                if !matches!(value.as_str(), "timer" | "overlay" | "heartbeat") {
                    return Err(format!("unknown role {value}"));
                }
                role = Some(value);
            }
            "--output-dir" => {
                output_dir = PathBuf::from(tokens.next().ok_or("--output-dir needs a value")?);
            }
            "--sample-ms" => {
                sample_ms = Some(parse_ms(
                    &tokens.next().ok_or("--sample-ms needs a value")?,
                    "--sample-ms",
                )?);
            }
            "--interval-ms" => {
                interval_ms = parse_ms(
                    &tokens.next().ok_or("--interval-ms needs a value")?,
                    "--interval-ms",
                )?;
            }
            "--lid-on-power" => {
                lid_on_power = Some(require_choice(
                    tokens.next().ok_or("--lid-on-power needs a value")?,
                    &["slept", "awake", "skip"],
                    "--lid-on-power",
                )?);
            }
            "--lid-external" => {
                lid_external = Some(require_choice(
                    tokens.next().ok_or("--lid-external needs a value")?,
                    &["slept", "awake", "no-external-display", "skip"],
                    "--lid-external",
                )?);
            }
            "--power-key" => {
                power_key = Some(require_choice(
                    tokens.next().ok_or("--power-key needs a value")?,
                    &["slept", "restart-panel", "nothing", "skip"],
                    "--power-key",
                )?);
            }
            "--count-file" => {
                count_file = Some(tokens.next().ok_or("--count-file needs a value")?);
            }
            "--cover-frame" => {
                cover_frame = Some(parse_frame(
                    &tokens.next().ok_or("--cover-frame needs x,y,w,h")?,
                )?);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if runner && kit {
        return Err("--runner and --kit are different modes".to_string());
    }
    if role.is_some() && (runner || kit) {
        return Err("child roles are not the orchestrator".to_string());
    }
    let mode = if let Some(role) = role {
        match role.as_str() {
            "timer" => Mode::Timer,
            "overlay" => Mode::Overlay,
            "heartbeat" => Mode::Heartbeat,
            _ => unreachable!(),
        }
    } else if kit {
        Mode::Kit
    } else {
        Mode::Runner
    };
    let sample_ms = sample_ms.unwrap_or(match mode {
        Mode::Kit => KIT_SAMPLE_MS,
        _ => RUNNER_SAMPLE_MS,
    });
    Ok(Args {
        mode,
        output_dir: absolute_dir(&output_dir),
        sample_ms,
        interval_ms,
        lid_on_power,
        lid_external,
        power_key,
        count_file,
        cover_frame,
    })
}

fn parse_ms(text: &str, flag: &str) -> Result<u64, String> {
    text.parse()
        .map_err(|_| format!("{flag} needs a millisecond count"))
}

fn parse_frame(text: &str) -> Result<(i32, i32, i32, i32), String> {
    let parts: Vec<&str> = text.split(',').collect();
    if parts.len() != 4 {
        return Err("--cover-frame needs x,y,w,h".to_string());
    }
    let parse = |value: &str| {
        value
            .parse::<i32>()
            .map_err(|_| "--cover-frame needs integers".to_string())
    };
    Ok((
        parse(parts[0])?,
        parse(parts[1])?,
        parse(parts[2])?,
        parse(parts[3])?,
    ))
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
    fn the_default_mode_is_the_runner_and_it_holds_assertions() {
        let args = parse(tokens(&[])).unwrap();
        assert_eq!(args.mode, Mode::Runner);
        assert!(args.mode.holds_assertions());
        assert_eq!(args.sample_ms, RUNNER_SAMPLE_MS);
        assert_eq!(args.interval_ms, DEFAULT_INTERVAL_MS);
        assert!(args.output_dir.ends_with(DEFAULT_OUTPUT_DIR));
    }

    #[test]
    fn the_kit_holds_assertions_and_children_do_not() {
        let kit = parse(tokens(&["--kit"])).unwrap();
        assert_eq!(kit.mode, Mode::Kit);
        assert!(kit.mode.holds_assertions());
        assert_eq!(kit.sample_ms, KIT_SAMPLE_MS);
        assert_eq!(APP_NAP_SAMPLE_MS, 60_000);

        let timer = parse(tokens(&["--role", "timer"])).unwrap();
        assert_eq!(timer.mode, Mode::Timer);
        assert!(!timer.mode.holds_assertions());
        assert!(
            !parse(tokens(&["--role", "overlay"]))
                .unwrap()
                .mode
                .holds_assertions()
        );
        assert!(
            !parse(tokens(&["--role", "heartbeat"]))
                .unwrap()
                .mode
                .holds_assertions()
        );
    }

    #[test]
    fn mixed_modes_and_bad_lid_rows_are_refused() {
        assert!(parse(tokens(&["--runner", "--kit"])).is_err());
        assert!(parse(tokens(&["--kit", "--role", "timer"])).is_err());
        assert!(parse(tokens(&["--role", "daemon"])).is_err());
        assert!(
            parse(tokens(&["--lid-on-power", "no-lid"]))
                .unwrap_err()
                .contains("--lid-on-power")
        );
        assert!(parse(tokens(&["--elsewhere"])).is_err());
        let ok = parse(tokens(&[
            "--kit",
            "--lid-on-power",
            "slept",
            "--lid-external",
            "awake",
            "--power-key",
            "nothing",
        ]))
        .unwrap();
        assert_eq!(ok.lid_on_power.as_deref(), Some("slept"));
        assert_eq!(ok.lid_external.as_deref(), Some("awake"));
        assert_eq!(ok.power_key.as_deref(), Some("nothing"));
    }
}
