use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, HMODULE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK,
    MOUSEINPUT, SendInput, VK_F13,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG,
    MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN, WM_QUIT,
};

use crate::report::HookReport;

use windows::core::PCWSTR;

use super::util::{explain, last_error};

struct Seen {
    key_vk: Option<u32>,
    key_injected: bool,
    click_message: Option<u32>,
    click_injected: bool,
}

static SEEN: Mutex<Seen> = Mutex::new(Seen {
    key_vk: None,
    key_injected: false,
    click_message: None,
    click_injected: false,
});

pub fn prove() -> HookReport {
    reset();
    let (ready_tx, ready_rx) = mpsc::channel();
    let worker = thread::spawn(move || hook_thread(ready_tx));
    let ready = ready_rx.recv_timeout(Duration::from_secs(5));
    let Ok(ready) = ready else {
        return failed("hook thread did not start");
    };
    let thread_id = match ready {
        Ok(thread_id) => thread_id,
        Err(error) => return failed(&error),
    };
    thread::sleep(Duration::from_millis(50));
    let inserted = match send_probe() {
        Ok(inserted) => inserted,
        Err(error) => {
            let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
            let _ = worker.join();
            return failed(&error);
        }
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut seen = snapshot();
    while Instant::now() < deadline && !(seen.key_vk.is_some() && seen.click_message.is_some()) {
        thread::sleep(Duration::from_millis(10));
        seen = snapshot();
    }
    let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    let join = worker.join();
    let mut report = HookReport {
        installed: true,
        key_seen: seen.key_vk == Some(u32::from(VK_F13.0)),
        key_injected: seen.key_injected && seen.key_vk == Some(u32::from(VK_F13.0)),
        key_vk: seen.key_vk,
        click_seen: seen.click_message == Some(WM_LBUTTONDOWN),
        click_injected: seen.click_injected && seen.click_message == Some(WM_LBUTTONDOWN),
        click_message: seen.click_message,
        inserted,
        error: None,
    };
    if inserted != 5 {
        report.error = Some(format!("SendInput inserted {inserted} of 5 events"));
    }
    if let Err(panic) = join {
        report.error = Some(format!("hook thread panicked: {}", panic_text(panic)));
    }
    report
}

fn failed(error: &str) -> HookReport {
    HookReport {
        installed: false,
        key_seen: false,
        key_injected: false,
        key_vk: None,
        click_seen: false,
        click_injected: false,
        click_message: None,
        inserted: 0,
        error: Some(error.to_string()),
    }
}

fn reset() {
    let mut seen = SEEN.lock().unwrap_or_else(|poison| poison.into_inner());
    *seen = Seen {
        key_vk: None,
        key_injected: false,
        click_message: None,
        click_injected: false,
    };
}

fn snapshot() -> Seen {
    let seen = SEEN.lock().unwrap_or_else(|poison| poison.into_inner());
    Seen {
        key_vk: seen.key_vk,
        key_injected: seen.key_injected,
        click_message: seen.click_message,
        click_injected: seen.click_injected,
    }
}

fn hook_thread(ready: mpsc::Sender<Result<u32, String>>) {
    let keyboard = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_proc),
            Some(module_instance()),
            0,
        )
    };
    let keyboard = match keyboard {
        Ok(hook) => hook,
        Err(error) => {
            let _ = ready.send(Err(explain(error, "keyboard hook")));
            return;
        }
    };
    let mouse =
        unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(module_instance()), 0) };
    let mouse = match mouse {
        Ok(hook) => hook,
        Err(error) => {
            unsafe {
                let _ = UnhookWindowsHookEx(keyboard);
            }
            let _ = ready.send(Err(explain(error, "mouse hook")));
            return;
        }
    };
    let mut message = MSG::default();
    unsafe {
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
    }
    let thread_id = unsafe { GetCurrentThreadId() };
    let _ = ready.send(Ok(thread_id));
    loop {
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 == 0 || status.0 == -1 {
            break;
        }
    }
    unsafe {
        let _ = UnhookWindowsHookEx(keyboard);
        let _ = UnhookWindowsHookEx(mouse);
    }
}

fn module_instance() -> HINSTANCE {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.unwrap_or(HMODULE::default());
    HINSTANCE(module.0)
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 && wparam.0 == WM_KEYDOWN as usize {
            let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            if info.vkCode == u32::from(VK_F13.0) {
                let mut seen = SEEN.lock().unwrap_or_else(|poison| poison.into_inner());
                seen.key_vk = Some(info.vkCode);
                seen.key_injected = info.flags.contains(LLKHF_INJECTED);
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 && wparam.0 == WM_LBUTTONDOWN as usize {
            let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            let mut seen = SEEN.lock().unwrap_or_else(|poison| poison.into_inner());
            seen.click_message = Some(WM_LBUTTONDOWN);
            seen.click_injected = info.flags & LLMHF_INJECTED != 0;
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

fn send_probe() -> Result<u32, String> {
    let (x, y) = absolute_center();
    let inputs = [
        key_input(false),
        key_input(true),
        mouse_input(
            x,
            y,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        ),
        mouse_input(
            x,
            y,
            MOUSEEVENTF_LEFTDOWN | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        ),
        mouse_input(
            x,
            y,
            MOUSEEVENTF_LEFTUP | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        ),
    ];
    let inserted = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if inserted == 0 {
        Err(last_error("SendInput"))
    } else {
        Ok(inserted)
    }
}

fn key_input(up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_F13,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_input(
    x: i32,
    y: i32,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: x,
                dy: y,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn absolute_center() -> (i32, i32) {
    // MOUSEEVENTF_VIRTUALDESK maps 0..65535 across the virtual desktop.
    (32_767, 32_767)
}

fn panic_text(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_string()))
        .unwrap_or_else(|| "unknown panic".to_string())
}
