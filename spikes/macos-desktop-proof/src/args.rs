//! Process modes. Only the orchestrator installs a daemon.

use std::path::{Path, PathBuf};

use crate::classify::POST_SOURCES;
use crate::labels::CAPTURE_APIS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Orchestrator,
    Job,
    TapOnly,
    WindowOnly,
    CaptureOnly,
    ActiveTap,
    M03,
    ClassifyTarget,
    ClassifyPost,
    Smoke,
}

impl Mode {
    pub fn installs_daemon(self) -> bool {
        matches!(self, Self::Orchestrator)
    }
}

#[derive(Debug)]
pub struct Args {
    pub allow_local_service: bool,
    pub runner_image: String,
    pub output_dir: PathBuf,
    pub mode: Mode,
    pub job_kind: Option<String>,
    pub tap_location: Option<String>,
    pub capture_api: Option<String>,
    pub post_source: Option<String>,
}

pub fn parse_env() -> Result<Args, String> {
    parse(std::env::args().skip(1))
}

pub fn parse<I>(tokens: I) -> Result<Args, String>
where
    I: IntoIterator<Item = String>,
{
    let mut allow_local_service = false;
    let mut runner_image = "macos-latest".to_string();
    let mut output_dir = PathBuf::from("proof-output");
    let mut job_kind = None;
    let mut tap_location = None;
    let mut window_only = false;
    let mut capture_api = None;
    let mut active_tap = false;
    let mut m03 = false;
    let mut classify_target = false;
    let mut post_source = None;
    let mut smoke = false;
    let mut tokens = tokens.into_iter();
    while let Some(token) = tokens.next() {
        match token.as_str() {
            "--allow-local-service" => allow_local_service = true,
            "--smoke" => smoke = true,
            "--runner-image" => {
                runner_image = tokens
                    .next()
                    .ok_or("--runner-image needs a value")?
                    .to_string();
            }
            "--output-dir" => {
                output_dir = PathBuf::from(tokens.next().ok_or("--output-dir needs a value")?);
            }
            "--job" => {
                let kind = tokens.next().ok_or("--job needs a value")?;
                if kind != "agent" && kind != "daemon" {
                    return Err(format!("unknown job {kind}"));
                }
                job_kind = Some(kind);
            }
            "--tap-only" => {
                let location = tokens.next().ok_or("--tap-only needs a value")?;
                if location != "kCGSessionEventTap" && location != "kCGHIDEventTap" {
                    return Err(format!("unknown tap location {location}"));
                }
                tap_location = Some(location);
            }
            "--window-only" => window_only = true,
            "--active-tap" => active_tap = true,
            "--m03" => m03 = true,
            "--classify-target" => classify_target = true,
            "--classify-post" => {
                let source = tokens.next().ok_or("--classify-post needs a value")?;
                if !POST_SOURCES.contains(&source.as_str()) {
                    return Err(format!("unknown event source {source}"));
                }
                post_source = Some(source);
            }
            "--capture-only" => {
                let api = tokens.next().ok_or("--capture-only needs a value")?;
                if !CAPTURE_APIS.contains(&api.as_str()) {
                    return Err(format!("unknown capture api {api}"));
                }
                capture_api = Some(api);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let selected = [
        job_kind.is_some(),
        tap_location.is_some(),
        window_only,
        capture_api.is_some(),
        active_tap,
        m03,
        classify_target,
        post_source.is_some(),
        smoke,
    ]
    .into_iter()
    .filter(|selected| *selected)
    .count();
    if selected > 1 {
        return Err("job, tap, window, capture, and classify are different processes".to_string());
    }
    let mode = if job_kind.is_some() {
        Mode::Job
    } else if tap_location.is_some() {
        Mode::TapOnly
    } else if window_only {
        Mode::WindowOnly
    } else if capture_api.is_some() {
        Mode::CaptureOnly
    } else if active_tap {
        Mode::ActiveTap
    } else if m03 {
        Mode::M03
    } else if classify_target {
        Mode::ClassifyTarget
    } else if post_source.is_some() {
        Mode::ClassifyPost
    } else if smoke {
        Mode::Smoke
    } else {
        Mode::Orchestrator
    };
    Ok(Args {
        allow_local_service,
        runner_image,
        output_dir: absolute_dir(&output_dir),
        mode,
        job_kind,
        tap_location,
        capture_api,
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
    fn the_orchestrator_is_the_only_mode_that_installs_a_daemon() {
        let args = parse(tokens(&[])).unwrap();
        assert_eq!(args.mode, Mode::Orchestrator);
        assert!(args.mode.installs_daemon());
        assert!(!args.allow_local_service);
        assert!(args.output_dir.ends_with("proof-output"));

        let job = parse(tokens(&["--job", "daemon", "--allow-local-service"])).unwrap();
        assert_eq!(job.mode, Mode::Job);
        assert!(!job.mode.installs_daemon());
        assert!(job.allow_local_service);
        assert_eq!(job.job_kind.as_deref(), Some("daemon"));

        let tap = parse(tokens(&["--tap-only", "kCGHIDEventTap"])).unwrap();
        assert_eq!(tap.mode, Mode::TapOnly);
        assert!(!tap.mode.installs_daemon());
        assert_eq!(tap.tap_location.as_deref(), Some("kCGHIDEventTap"));
    }

    #[test]
    fn unknown_flags_and_mixed_modes_are_refused() {
        assert!(parse(tokens(&["--job"])).unwrap_err().contains("--job"));
        assert!(parse(tokens(&["--job", "gui"])).is_err());
        assert!(
            parse(tokens(&["--tap-only", "annotated"]))
                .unwrap_err()
                .contains("unknown tap")
        );
        assert!(
            parse(tokens(&[
                "--job",
                "agent",
                "--tap-only",
                "kCGSessionEventTap"
            ]))
            .is_err()
        );
        let window = parse(tokens(&["--window-only"])).unwrap();
        assert_eq!(window.mode, Mode::WindowOnly);
        assert!(!window.mode.installs_daemon());
        let capture = parse(tokens(&["--capture-only", "screencapture"])).unwrap();
        assert_eq!(capture.mode, Mode::CaptureOnly);
        assert_eq!(capture.capture_api.as_deref(), Some("screencapture"));
        let active = parse(tokens(&["--active-tap"])).unwrap();
        assert_eq!(active.mode, Mode::ActiveTap);
        assert!(!active.mode.installs_daemon());
        let smoke = parse(tokens(&["--smoke"])).unwrap();
        assert_eq!(smoke.mode, Mode::Smoke);
        assert!(!smoke.mode.installs_daemon());
        assert!(parse(tokens(&["--smoke", "--window-only"])).is_err());
        assert!(parse(tokens(&["--capture-only", "camera"])).is_err());
        assert!(parse(tokens(&["--window-only", "--tap-only", "kCGHIDEventTap"])).is_err());
        let m03 = parse(tokens(&["--m03"])).unwrap();
        assert_eq!(m03.mode, Mode::M03);
        assert!(!m03.mode.installs_daemon());
        let post = parse(tokens(&[
            "--classify-post",
            "kCGEventSourceStateHIDSystemState",
        ]))
        .unwrap();
        assert_eq!(post.mode, Mode::ClassifyPost);
        assert!(!post.mode.installs_daemon());
        assert!(parse(tokens(&["--classify-post", "hardware"])).is_err());
        assert!(parse(tokens(&["--m03", "--classify-target"])).is_err());
        assert!(parse(tokens(&["--elsewhere"])).is_err());
        assert!(parse(tokens(&["--output-dir"])).is_err());
    }
}
