//! Which top-level windows are shell surfaces. They are dismissed with Escape, never closed with a posted message.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellSurface {
    Start,
    TaskView,
    NotificationCenter,
    QuickSettings,
    Project,
}

/// A window is on screen only when Windows says it is visible and DWM has not
/// cloaked it. The hidden Start menu reports visible while it is cloaked.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn surface_is_open(visible: bool, cloaked: bool) -> bool {
    visible && !cloaked
}

/// A real shell host owns this window. The spike never posts a close message
/// to one of these. It dismisses the surface with Escape instead.
/// StartMenuExperienceHost is the Start menu even when its title is blank.
pub fn identify(class: &str, title: &str, exe: &str) -> Option<ShellSurface> {
    let class_l = class.trim().to_ascii_lowercase();
    let title_l = title.trim().to_ascii_lowercase();
    let exe_l = file_name(exe).to_ascii_lowercase();
    if exe_l == "startmenuexperiencehost.exe" && is_shell_class(&class_l) {
        return Some(ShellSurface::Start);
    }
    let kind = kind_of(&class_l, &title_l)?;
    if exe_allowed(&exe_l, kind) {
        Some(kind)
    } else {
        None
    }
}

fn kind_of(class: &str, title: &str) -> Option<ShellSurface> {
    if class == "multitaskingviewframe" {
        return Some(ShellSurface::TaskView);
    }
    if !is_shell_class(class) {
        return None;
    }
    match title {
        "start" | "start menu" | "search" => Some(ShellSurface::Start),
        "task view" => Some(ShellSurface::TaskView),
        "notification center" | "action center" | "action centre" => {
            Some(ShellSurface::NotificationCenter)
        }
        "quick settings" => Some(ShellSurface::QuickSettings),
        "project" => Some(ShellSurface::Project),
        _ => None,
    }
}

fn is_shell_class(class: &str) -> bool {
    matches!(
        class,
        "windows.ui.core.corewindow"
            | "xamlexplorerhostislandwindow"
            | "windows.ui.composition.desktopwindowcontentbridge"
            | "multitaskingviewframe"
    )
}

fn exe_allowed(exe: &str, kind: ShellSurface) -> bool {
    matches!(
        exe,
        "explorer.exe"
            | "startmenuexperiencehost.exe"
            | "shellexperiencehost.exe"
            | "shellhost.exe"
    ) || (exe == "searchhost.exe" && kind == ShellSurface::Start)
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_shell_windows_are_classified() {
        let cases = [
            (
                "Windows.UI.Core.CoreWindow",
                "Start",
                r"C:\Windows\SystemApps\StartMenuExperienceHost.exe",
                ShellSurface::Start,
            ),
            (
                "XamlExplorerHostIslandWindow",
                "Start",
                r"C:\Windows\SystemApps\StartMenuExperienceHost.exe",
                ShellSurface::Start,
            ),
            (
                "XamlExplorerHostIslandWindow",
                "Task View",
                r"C:\Windows\explorer.exe",
                ShellSurface::TaskView,
            ),
            (
                "MultitaskingViewFrame",
                "",
                r"C:\Windows\explorer.exe",
                ShellSurface::TaskView,
            ),
            (
                "Windows.UI.Core.CoreWindow",
                "Notification Center",
                r"C:\Windows\SystemApps\ShellExperienceHost.exe",
                ShellSurface::NotificationCenter,
            ),
            (
                "Windows.UI.Core.CoreWindow",
                "Action center",
                r"C:\Windows\SystemApps\ShellExperienceHost.exe",
                ShellSurface::NotificationCenter,
            ),
            (
                "Windows.UI.Core.CoreWindow",
                "Quick settings",
                r"C:\Windows\SystemApps\ShellExperienceHost.exe",
                ShellSurface::QuickSettings,
            ),
            (
                "XamlExplorerHostIslandWindow",
                "Project",
                r"C:\Windows\explorer.exe",
                ShellSurface::Project,
            ),
            (
                "Windows.UI.Core.CoreWindow",
                "Start",
                r"C:\Windows\SystemApps\SearchHost.exe",
                ShellSurface::Start,
            ),
        ];
        for (class, title, exe, expected) in cases {
            assert_eq!(
                identify(class, title, exe),
                Some(expected),
                "{class} / {title} / {exe}"
            );
        }
    }

    #[test]
    fn ordinary_windows_and_the_overlay_itself_are_left_alone() {
        assert_eq!(
            identify("Notepad", "Start", r"C:\Windows\notepad.exe"),
            None
        );
        assert_eq!(
            identify(
                "spike-overlay-window",
                "overlay",
                r"C:\work\spike-overlay.exe"
            ),
            None
        );
        assert_eq!(
            identify(
                "Windows.UI.Core.CoreWindow",
                "Start",
                r"C:\work\spike-overlay.exe"
            ),
            None
        );
        assert_eq!(
            identify("Chrome_WidgetWin_1", "Inbox", r"C:\chrome.exe"),
            None
        );
    }

    #[test]
    fn a_cloaked_window_is_not_an_open_surface() {
        assert!(surface_is_open(true, false));
        assert!(!surface_is_open(true, true));
        assert!(!surface_is_open(false, false));
        assert!(!surface_is_open(false, true));
    }

    #[test]
    fn a_blank_start_host_window_is_still_the_start_menu() {
        assert_eq!(
            identify(
                "Windows.UI.Core.CoreWindow",
                "",
                r"C:\Windows\SystemApps\StartMenuExperienceHost.exe"
            ),
            Some(ShellSurface::Start)
        );
    }
}
