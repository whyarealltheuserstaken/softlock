//! Which top-level windows are shell surfaces the mitigation may close.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellSurface {
    Start,
    TaskView,
    NotificationCenter,
    QuickSettings,
    Project,
}

/// `exe` may be a full path. `extra_exe` is the spike itself, and only the
/// smoke fixture sets it, so a test window can prove the closer without
/// touching the real shell.
pub fn should_close(
    class: &str,
    title: &str,
    exe: &str,
    extra_exe: Option<&str>,
) -> Option<ShellSurface> {
    let class_l = class.trim().to_ascii_lowercase();
    let title_l = title.trim().to_ascii_lowercase();
    let exe_l = file_name(exe).to_ascii_lowercase();
    let kind = kind_of(&class_l, &title_l)?;
    if exe_allowed(&exe_l, kind) {
        return Some(kind);
    }
    if extra_exe.is_some_and(|extra| file_name(extra).eq_ignore_ascii_case(&exe_l)) {
        return Some(kind);
    }
    None
}

fn kind_of(class: &str, title: &str) -> Option<ShellSurface> {
    if class == "multitaskingviewframe" {
        return Some(ShellSurface::TaskView);
    }
    if !is_shell_class(class) {
        return None;
    }
    match title {
        "start" => Some(ShellSurface::Start),
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
                should_close(class, title, exe, None),
                Some(expected),
                "{class} / {title} / {exe}"
            );
        }
    }

    #[test]
    fn ordinary_windows_and_the_overlay_itself_are_left_alone() {
        assert_eq!(
            should_close("Notepad", "Start", r"C:\Windows\notepad.exe", None),
            None
        );
        assert_eq!(
            should_close(
                "spike-overlay-window",
                "overlay",
                r"C:\work\spike-overlay.exe",
                None
            ),
            None
        );
        assert_eq!(
            should_close(
                "Windows.UI.Core.CoreWindow",
                "Start",
                r"C:\work\spike-overlay.exe",
                None
            ),
            None
        );
        assert_eq!(
            should_close("Chrome_WidgetWin_1", "Inbox", r"C:\chrome.exe", None),
            None
        );
    }

    #[test]
    fn the_smoke_fixture_may_close_only_its_own_extra_exe() {
        assert_eq!(
            should_close(
                "Windows.UI.Core.CoreWindow",
                "Start",
                r"C:\work\spike-overlay.exe",
                Some("spike-overlay.exe")
            ),
            Some(ShellSurface::Start)
        );
    }
}
