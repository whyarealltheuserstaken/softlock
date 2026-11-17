//! The injected-input sequence. Opening a surface never confirms UAC.

use std::time::Duration;

use crate::config::FULLSCREEN_MAX_SECS;

/// Windows 11 starts at this build. Older builds use the Action Center chord.
pub const WINDOWS_11_BUILD: u32 = 22_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceId {
    Start,
    TaskView,
    NotificationCenter,
    Toast,
    Uac,
    FullscreenExclusive,
    MonitorPlugIn,
}

impl SurfaceId {
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "start" => Some(Self::Start),
            "taskview" | "task-view" | "task view" => Some(Self::TaskView),
            "notificationcenter" | "notification-center" | "notification" | "notifications" => {
                Some(Self::NotificationCenter)
            }
            "toast" => Some(Self::Toast),
            "uac" => Some(Self::Uac),
            "fullscreen" | "full-screen" => Some(Self::FullscreenExclusive),
            "monitor" | "project" => Some(Self::MonitorPlugIn),
            _ => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Start => "Start menu",
            Self::TaskView => "Task View",
            Self::NotificationCenter => "Notification center",
            Self::Toast => "Toast banner",
            Self::Uac => "UAC prompt",
            Self::FullscreenExclusive => "Full-screen exclusive window",
            Self::MonitorPlugIn => "Monitor plug-in prompt",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Tab,
    N,
    A,
    P,
}

impl Key {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tab => "Tab",
            Self::N => "N",
            Self::A => "A",
            Self::P => "P",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub win: bool,
    pub key: Option<Key>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// The script must not send Enter, Alt+Y, or any other accept key.
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Chord(Chord),
    Toast,
    Uac {
        file: &'static str,
        params: &'static str,
        confirm: Confirm,
    },
    Fullscreen {
        max_secs: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dismiss {
    RepeatChord,
    /// UAC lives on the secure desktop. The person at the keyboard clicks No.
    ClickNo,
    ExitsItself,
    Expires,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub id: SurfaceId,
    pub action: Action,
    pub dismiss: Dismiss,
    pub look_for: &'static str,
}

pub fn notification_chord(build: u32) -> Chord {
    if build >= WINDOWS_11_BUILD {
        Chord {
            win: true,
            key: Some(Key::N),
        }
    } else {
        Chord {
            win: true,
            key: Some(Key::A),
        }
    }
}

pub fn steps(build: u32, only: Option<SurfaceId>, fullscreen_secs: u64) -> Vec<Step> {
    let fullscreen_secs = fullscreen_secs.clamp(1, FULLSCREEN_MAX_SECS);
    let all = [
        Step {
            id: SurfaceId::Start,
            action: Action::Chord(Chord {
                win: true,
                key: None,
            }),
            dismiss: Dismiss::RepeatChord,
            look_for: "The Start menu. Is it pink, or is it drawn above the pink?",
        },
        Step {
            id: SurfaceId::TaskView,
            action: Action::Chord(Chord {
                win: true,
                key: Some(Key::Tab),
            }),
            dismiss: Dismiss::RepeatChord,
            look_for: "Task View. Is it pink, or is it drawn above the pink?",
        },
        Step {
            id: SurfaceId::NotificationCenter,
            action: Action::Chord(notification_chord(build)),
            dismiss: Dismiss::RepeatChord,
            look_for: "The notification center. On Windows 10 this is Action Center.",
        },
        Step {
            id: SurfaceId::Toast,
            action: Action::Toast,
            dismiss: Dismiss::Expires,
            look_for: "A banner, usually at the bottom right or the top right. Photograph it with your phone.",
        },
        Step {
            id: SurfaceId::Uac,
            action: Action::Uac {
                file: "cmd.exe",
                params: "/c exit",
                confirm: Confirm::Never,
            },
            dismiss: Dismiss::ClickNo,
            look_for: "The dimmed User Account Control screen. Click No after the photo. Never click Yes.",
        },
        Step {
            id: SurfaceId::FullscreenExclusive,
            action: Action::Fullscreen {
                max_secs: fullscreen_secs,
            },
            dismiss: Dismiss::ExitsItself,
            look_for: "A green full-screen window. Press Esc if you want it gone sooner. It also closes itself.",
        },
        Step {
            id: SurfaceId::MonitorPlugIn,
            action: Action::Chord(Chord {
                win: true,
                key: Some(Key::P),
            }),
            dismiss: Dismiss::RepeatChord,
            look_for: "The Project menu from the injected Win+P chord. On the school PC, also plug a monitor in while the pink layer is up and photograph the new screen.",
        },
    ];
    all.into_iter()
        .filter(|step| only.is_none_or(|id| id == step.id))
        .collect()
}

pub fn step_wait(pause: Duration, id: SurfaceId, fullscreen_secs: u64) -> Duration {
    if id == SurfaceId::FullscreenExclusive {
        pause.min(Duration::from_secs(
            fullscreen_secs.min(FULLSCREEN_MAX_SECS),
        ))
    } else {
        pause
    }
}

/// How long the overlay must stay up so the tour can finish, plus a small buffer.
pub fn tour_duration(pause: Duration, only: Option<SurfaceId>, fullscreen_secs: u64) -> Duration {
    let waits = steps(WINDOWS_11_BUILD, only, fullscreen_secs)
        .into_iter()
        .map(|step| step_wait(pause, step.id, fullscreen_secs))
        .sum::<Duration>();
    waits + Duration::from_secs(15)
}

pub fn plan_text() -> String {
    let mut lines = Vec::new();
    lines.push("The tour injects input. It does not block the mouse or the keyboard.".to_string());
    for (index, step) in steps(WINDOWS_11_BUILD, None, FULLSCREEN_MAX_SECS)
        .iter()
        .enumerate()
    {
        lines.push(format!(
            "{}. {} — {}",
            index + 1,
            step.id.title(),
            describe(step)
        ));
    }
    lines.push("Windows 10 uses Win+A for Action Center instead of Win+N.".to_string());
    lines.push(
        "The real monitor plug-in is physical. The injected chord only opens Project.".to_string(),
    );
    lines.join("\n") + "\n"
}

fn describe(step: &Step) -> String {
    match step.action {
        Action::Chord(Chord {
            win: true,
            key: None,
        }) => "injected Win key".into(),
        Action::Chord(Chord {
            win: true,
            key: Some(key),
        }) => format!("injected Win+{}", key.label()),
        Action::Chord(Chord { win: false, key }) => format!("injected {:?}", key),
        Action::Toast => "a toast banner from this program".into(),
        Action::Uac {
            file,
            params,
            confirm: Confirm::Never,
        } => format!("consent prompt for \"{file} {params}\" (click No, never Yes)"),
        Action::Fullscreen { max_secs } => {
            format!("green window, exits on Esc or within {max_secs} seconds")
        }
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn toast_xml(product: &str) -> String {
    format!(
        "<toast duration=\"short\"><visual><binding template=\"ToastGeneric\"><text>{}</text><text>Photograph this banner with your phone. Does it sit above the pink layer?</text></binding></visual></toast>",
        xml_escape(product)
    )
}

#[cfg_attr(not(windows), allow(dead_code))]
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tour_opens_the_seven_surfaces_in_order() {
        let ids: Vec<_> = steps(WINDOWS_11_BUILD, None, 20)
            .into_iter()
            .map(|step| step.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                SurfaceId::Start,
                SurfaceId::TaskView,
                SurfaceId::NotificationCenter,
                SurfaceId::Toast,
                SurfaceId::Uac,
                SurfaceId::FullscreenExclusive,
                SurfaceId::MonitorPlugIn,
            ]
        );
    }

    #[test]
    fn notification_center_uses_win_n_on_windows_11_and_win_a_on_windows_10() {
        assert_eq!(
            notification_chord(22_000),
            Chord {
                win: true,
                key: Some(Key::N)
            }
        );
        assert_eq!(
            notification_chord(19_045),
            Chord {
                win: true,
                key: Some(Key::A)
            }
        );
    }

    #[test]
    fn uac_is_a_harmless_command_and_is_never_confirmed() {
        let step = steps(WINDOWS_11_BUILD, Some(SurfaceId::Uac), 20)
            .into_iter()
            .next()
            .unwrap();
        match step.action {
            Action::Uac {
                file,
                params,
                confirm,
            } => {
                assert_eq!(file, "cmd.exe");
                assert_eq!(params, "/c exit");
                assert_eq!(confirm, Confirm::Never);
            }
            other => panic!("expected UAC, got {other:?}"),
        }
        assert_eq!(step.dismiss, Dismiss::ClickNo);
        let text = plan_text();
        assert!(!text.to_ascii_lowercase().contains("click yes"));
        assert!(text.contains("never Yes"));
    }

    #[test]
    fn fullscreen_wait_is_capped_at_thirty_seconds() {
        let wait = step_wait(
            Duration::from_secs(120),
            SurfaceId::FullscreenExclusive,
            600,
        );
        assert!(wait <= Duration::from_secs(FULLSCREEN_MAX_SECS));
        let step = steps(WINDOWS_11_BUILD, Some(SurfaceId::FullscreenExclusive), 600)
            .into_iter()
            .next()
            .unwrap();
        match step.action {
            Action::Fullscreen { max_secs } => assert_eq!(max_secs, FULLSCREEN_MAX_SECS),
            other => panic!("expected fullscreen, got {other:?}"),
        }
    }

    #[test]
    fn toast_text_uses_the_product_name_constant() {
        let xml = toast_xml(softlock_core::PRODUCT_DISPLAY_NAME);
        assert!(xml.contains(softlock_core::PRODUCT_DISPLAY_NAME));
        assert!(xml.contains("phone"));
        assert_eq!(
            toast_xml("A<B&C"),
            "<toast duration=\"short\"><visual><binding template=\"ToastGeneric\"><text>A&lt;B&amp;C</text><text>Photograph this banner with your phone. Does it sit above the pink layer?</text></binding></visual></toast>"
        );
    }

    #[test]
    fn only_one_surface_filters_the_tour() {
        let steps = steps(WINDOWS_11_BUILD, Some(SurfaceId::Toast), 20);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].id, SurfaceId::Toast);
    }
}
