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
        // SAFETY: this thread owns the message loop. RPC_E_CHANGED_MODE means COM is already
        // initialized, so this function must not call CoUninitialize.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr.is_err() {
            return Self {
                automation: None,
                uninit: false,
            };
        }
        // SAFETY: CUIAutomation is the in-process UI Automation class. Failure falls back to the foreground window.
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
            // SAFETY: the desktop window is a process-wide handle and is not destroyed here.
            let desktop = unsafe { GetDesktopWindow() };
            if !desktop.0.is_null() {
                // SAFETY: the automation object was created on this thread.
                let element = unsafe { automation.ElementFromHandle(desktop) };
                if let Ok(element) = element
                    // SAFETY: reading the bounding rectangle does not change focus or send input.
                    && let Ok(rect) = unsafe { element.CurrentBoundingRectangle() }
                {
                    let _ = rect.left;
                    return ("uia", true);
                }
            }
        }
        // SAFETY: a null foreground window means there is nothing to sample. The title is not read.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return ("unavailable", false);
        }
        // SAFETY: only the title length is read, so the log never stores a window title.
        let _ = unsafe { GetWindowTextLengthW(hwnd) };
        ("foreground", true)
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.automation = None;
        if self.uninit {
            // SAFETY: CoInitializeEx succeeded on this thread, so one CoUninitialize balances it.
            unsafe { CoUninitialize() };
        }
    }
}
