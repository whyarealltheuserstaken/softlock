//! Runner proof versus the interactive kit. Children never hold a tap.

use std::path::{Path, PathBuf};

use crate::classify::POST_SOURCES;

pub const DEFAULT_OUTPUT_DIR: &str = "physical-input-output";

pub const USAGE: &str = "usage: macos-physical-input --kit | --runner | --classify-target | --classify-post <source> [--output-dir DIR]\n\
Bare invocation prints this usage and does not post keys. --kit is the person-at-the-Mac run. --runner is the GitHub slice.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Runner,
    Kit,
    ClassifyTarget,
    ClassifyPost,
}

impl Mode {
    pub fn holds_tap(self) -> bool {
        matches!(self, Self::Runner | Self::Kit)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    pub output_dir: PathBuf,
    pub post_source: Option<String>,
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
    let mut classify_target = false;
    let mut post_source = None;
    let mut output_dir = PathBuf::from(DEFAULT_OUTPUT_DIR);
    let mut tokens = tokens.into_iter();
    while let Some(token) = tokens.next() {
        match token.as_str() {
            "--runner" => runner = true,
            "--kit" => kit = true,
            "--classify-target" => classify_target = true,
            "--classify-post" => {
                let source = tokens.next().ok_or("--classify-post needs a value")?;
                if !POST_SOURCES.contains(&source.as_str()) {
                    return Err(format!("unknown event source {source}"));
                }
                post_source = Some(source);
            }
            "--output-dir" => {
                output_dir = PathBuf::from(tokens.next().ok_or("--output-dir needs a value")?);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if runner && kit {
        return Err("--runner and --kit are different modes".to_string());
    }
    let selected = [runner, kit, classify_target, post_source.is_some()]
        .into_iter()
        .filter(|selected| *selected)
        .count();
    if selected > 1 {
        return Err("kit, runner, and classify children are different processes".to_string());
    }
    if selected == 0 {
        return Err(USAGE.to_string());
    }
    let mode = if classify_target {
        Mode::ClassifyTarget
    } else if post_source.is_some() {
        Mode::ClassifyPost
    } else if kit {
        Mode::Kit
    } else {
        Mode::Runner
    };
    Ok(Args {
        mode,
        output_dir: absolute_dir(&output_dir),
        post_source,
    })
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
    fn no_args_prints_usage_and_does_not_post_keys() {
        let error = parse(tokens(&[])).unwrap_err();
        assert!(error.contains("--kit"));
        assert!(error.contains("--runner"));
        assert!(error.contains("does not post keys"));
        let runner = parse(tokens(&["--runner"])).unwrap();
        assert_eq!(runner.mode, Mode::Runner);
        assert!(runner.mode.holds_tap());
        assert!(runner.output_dir.ends_with(DEFAULT_OUTPUT_DIR));
        let kit = parse(tokens(&["--kit"])).unwrap();
        assert_eq!(kit.mode, Mode::Kit);
        assert!(kit.mode.holds_tap());
        let target = parse(tokens(&["--classify-target"])).unwrap();
        assert!(!target.mode.holds_tap());
        let post = parse(tokens(&[
            "--classify-post",
            "kCGEventSourceStateHIDSystemState",
        ]))
        .unwrap();
        assert_eq!(post.mode, Mode::ClassifyPost);
        assert!(!post.mode.holds_tap());
    }

    #[test]
    fn mixed_modes_and_unknown_sources_are_refused() {
        assert!(parse(tokens(&["--runner", "--kit"])).is_err());
        assert!(parse(tokens(&["--kit", "--classify-target"])).is_err());
        assert!(parse(tokens(&["--classify-post", "hardware"])).is_err());
        assert!(parse(tokens(&["--elsewhere"])).is_err());
    }
}
