//! Process modes. The default is read-only and cannot lock.

use std::path::{Path, PathBuf};

use crate::helper::HelperAction;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExercisePath {
    Private,
    Documented,
}

impl ExercisePath {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Documented => "documented",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    ReadOnly,
    Exercise {
        which: ExercisePath,
        acknowledged: bool,
    },
    Helper(HelperAction),
}

impl Mode {
    pub const fn may_request_a_lock(self) -> bool {
        matches!(self, Self::Exercise { .. } | Self::Helper(_))
    }
}

#[derive(Debug)]
pub struct Args {
    pub output_dir: PathBuf,
    pub mode: Mode,
    pub auto_observe: bool,
}

pub fn parse_env() -> Result<Args, String> {
    parse(std::env::args().skip(1))
}

pub fn parse<I>(tokens: I) -> Result<Args, String>
where
    I: IntoIterator<Item = String>,
{
    let mut output_dir = PathBuf::from("lock-kit-output");
    let mut read_only = false;
    let mut exercise = None;
    let mut acknowledged = false;
    let mut helper = None;
    let mut auto_observe = false;
    let mut tokens = tokens.into_iter();
    while let Some(token) = tokens.next() {
        match token.as_str() {
            "--read-only" => read_only = true,
            "--i-will-unlock" => acknowledged = true,
            "--auto-observe" => auto_observe = true,
            "--output-dir" => {
                output_dir = PathBuf::from(tokens.next().ok_or("--output-dir needs a value")?);
            }
            "--exercise" => {
                let which = tokens
                    .next()
                    .ok_or("--exercise needs private or documented")?;
                exercise = Some(match which.as_str() {
                    "private" => ExercisePath::Private,
                    "documented" => ExercisePath::Documented,
                    other => return Err(format!("unknown exercise {other}")),
                });
            }
            "--helper" => {
                let action = tokens
                    .next()
                    .ok_or("--helper needs private, documented, or none")?;
                helper = Some(
                    HelperAction::parse(&action)
                        .ok_or_else(|| format!("unknown helper {action}"))?,
                );
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let selected = [read_only, exercise.is_some(), helper.is_some()]
        .into_iter()
        .filter(|selected| *selected)
        .count();
    if selected > 1 {
        return Err("read-only, exercise, and helper are different runs".to_string());
    }
    if acknowledged && exercise.is_none() {
        return Err("--i-will-unlock is only valid with --exercise".to_string());
    }
    if auto_observe && exercise.is_none() {
        return Err("--auto-observe is only valid with --exercise".to_string());
    }
    let mode = if let Some(action) = helper {
        Mode::Helper(action)
    } else if let Some(which) = exercise {
        Mode::Exercise {
            which,
            acknowledged,
        }
    } else {
        Mode::ReadOnly
    };
    Ok(Args {
        output_dir: absolute_dir(&output_dir),
        mode,
        auto_observe,
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
    fn the_default_is_read_only_and_cannot_request_a_lock() {
        let args = parse(tokens(&[])).unwrap();
        assert_eq!(args.mode, Mode::ReadOnly);
        assert!(!args.mode.may_request_a_lock());
        assert!(args.output_dir.ends_with("lock-kit-output"));
        let explicit = parse(tokens(&["--read-only"])).unwrap();
        assert_eq!(explicit.mode, Mode::ReadOnly);
    }

    #[test]
    fn exercise_is_opt_in_and_mixed_modes_are_refused() {
        let missing = parse(tokens(&["--exercise", "private"])).unwrap();
        assert_eq!(
            missing.mode,
            Mode::Exercise {
                which: ExercisePath::Private,
                acknowledged: false,
            }
        );
        let ready = parse(tokens(&["--exercise", "documented", "--i-will-unlock"])).unwrap();
        assert_eq!(
            ready.mode,
            Mode::Exercise {
                which: ExercisePath::Documented,
                acknowledged: true,
            }
        );
        assert!(ready.mode.may_request_a_lock());
        assert!(parse(tokens(&["--i-will-unlock"])).is_err());
        assert!(parse(tokens(&["--read-only", "--exercise", "private"])).is_err());
        assert!(parse(tokens(&["--exercise", "loginwindow"])).is_err());
        assert!(parse(tokens(&["--auto-observe"])).is_err());
        let auto = parse(tokens(&[
            "--exercise",
            "private",
            "--i-will-unlock",
            "--auto-observe",
        ]))
        .unwrap();
        assert!(auto.auto_observe);
        assert!(parse(tokens(&["--helper"])).is_err());
        let helper = parse(tokens(&["--helper", "none"])).unwrap();
        assert_eq!(helper.mode, Mode::Helper(crate::helper::HelperAction::None));
    }
}
