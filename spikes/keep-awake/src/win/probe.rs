//! One small UI Automation call per tick. It does not send keys or move the pointer,
//! because a key would stop the run and a moved pointer would bother the person at the laptop.

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};
use windows::Win32::UI::WindowsAndMessaging::{
    GetDesktopWindow, GetForegroundWindow, GetWindowTextLengthW,
};

pub struct Probe {
    automation: Option<IUIAutomation>,
    uninit: bool,
}

impl Probe {
    pub fn start() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr.is_err() {
            return Self {
                automation: None,
                uninit: false,
            };
        }
        let automation = unsafe {
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
        };
        Self {
            automation,
            uninit: true,
        }
    }

    pub fn mode(&self) -> &'static str {
        if self.automation.is_some() {
            "uia"
        } else {
            "foreground"
        }
    }

    /// Returns `(kind, ok)`.
    pub fn sample(&self) -> (&'static str, bool) {
        if let Some(automation) = &self.automation {
            let desktop = unsafe { GetDesktopWindow() };
            if !desktop.0.is_null() {
                let element = unsafe { automation.ElementFromHandle(desktop) };
                if let Ok(element) = element
                    && let Ok(rect) = unsafe { element.CurrentBoundingRectangle() }
                {
                    let _ = rect.left;
                    return ("uia", true);
                }
            }
        }
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return ("unavailable", false);
        }
        let _ = unsafe { GetWindowTextLengthW(hwnd) };
        ("foreground", true)
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.automation = None;
        if self.uninit {
            unsafe { CoUninitialize() };
        }
    }
}
