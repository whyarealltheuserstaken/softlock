//! Windows proof window. Click-through, capture-excluded, and never a hook.

mod fullscreen;
mod input;
mod registry;
mod shell;
mod toast;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, Once};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmIsCompositionEnabled;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CLR_INVALID,
    CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
    DEFAULT_GUI_FONT, DT_WORDBREAK, DeleteDC, DeleteObject, DrawTextW, EndPaint,
    EnumDisplayMonitors, FF_DONTCARE, FW_SEMIBOLD, FillRect, GetDC, GetPixel, GetStockObject,
    HBRUSH, HDC, HFONT, HGDIOBJ, InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT, ReleaseDC,
    SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey, VK_F13, VK_Q,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GW_HWNDFIRST, GW_HWNDNEXT,
    GWL_EXSTYLE, GetForegroundWindow, GetMessageW, GetWindow, GetWindowDisplayAffinity,
    GetWindowLongPtrW, GetWindowRect, HTTRANSPARENT, HWND_NOTOPMOST, HWND_TOPMOST, IDC_ARROW,
    IsWindow, LWA_ALPHA, LoadCursorW, MSG, PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage,
    PostThreadMessageW, RegisterClassW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SetForegroundWindow, SetLayeredWindowAttributes, SetWindowDisplayAffinity,
    SetWindowPos, ShowWindow, TranslateMessage, WDA_EXCLUDEFROMCAPTURE, WINDOWPOS, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN, WM_NCHITTEST, WM_PAINT,
    WM_QUIT, WM_TIMER, WM_WINDOWPOSCHANGED, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_VISIBLE,
};
use windows::core::PCWSTR;

use crate::config::{self, Command, Config, HOTKEY_LABEL, REASSERT_INTERVAL_MS};
use crate::coverage::{MonitorRect, windows_for};
use crate::safety::{CAPTURE_EXCLUDED, EX_OVERLAY, HIT_TRANSPARENT, overlay_lines};
use crate::shell::ShellSurface;
use crate::tour::{self, Action, Step, SurfaceId};

const HOTKEY_ID: i32 = 1;
const TIMER_ID: usize = 1;
const CLASS_NAME: &str = "spike-overlay-window";
const PROBE_CLASS: &str = "spike-overlay-probe";
const MARKER: u32 = 0x00FF_00FF;

static GUI_THREAD: AtomicU32 = AtomicU32::new(0);
static FINISHED: AtomicBool = AtomicBool::new(false);
static REASSERTING: AtomicBool = AtomicBool::new(false);
static PROBE_SAW_F13: AtomicBool = AtomicBool::new(false);
static FULLSCREEN_CHILD: Mutex<Option<std::process::Child>> = Mutex::new(None);
static CLEANUP_ONCE: Once = Once::new();
static STATUS: Mutex<OverlayNote> = Mutex::new(OverlayNote::Intro);
static HWNDS: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static TOUR_ERROR: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone, Copy)]
enum OverlayNote {
    Intro,
    Step {
        number: usize,
        title: &'static str,
        look_for: &'static str,
    },
    ClickNo,
}

thread_local! {
    static GUI: RefCell<Option<Gui>> = const { RefCell::new(None) };
}

struct Gui {
    windows: Vec<LiveWindow>,
    lines: Vec<String>,
    brush: HBRUSH,
    font: HFONT,
    font_owned: bool,
    class_name: Vec<u16>,
    reassert_timer: bool,
    reassert_pos: bool,
    close_shell: bool,
    quitting: bool,
    alpha: u8,
}

struct LiveWindow {
    hwnd: HWND,
    rect: MonitorRect,
}

struct Cleanup;

impl Drop for Cleanup {
    fn drop(&mut self) {
        finish();
    }
}

pub fn run(config: Config) -> Result<(), String> {
    if config.command == Command::RestoreToasts {
        toast::remove_shortcut();
        println!("{}", registry::restore()?);
        return Ok(());
    }
    if config.command == Command::Fullscreen {
        return fullscreen::run(config.fullscreen_secs);
    }
    if config.command == Command::Tour {
        let elevated = unsafe { windows::Win32::UI::Shell::IsUserAnAdmin() }.as_bool();
        config::tour_allowed(elevated, config.allow_admin)?;
    }
    config.ensure_timeout_covers_tour()?;
    if config.command == Command::Tour
        && config.prompt
        && std::io::IsTerminal::is_terminal(&std::io::stdin())
    {
        print!("{}", tour::plan_text());
        println!("Press Enter to show the pink overlay.");
        println!("Exit: {HOTKEY_LABEL}. It also closes itself.");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }

    let _cleanup = Cleanup;
    toast::remove_shortcut();
    let _ = registry::restore();
    if config.suppress_toasts {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        registry::suppress(&exe.to_string_lossy())?;
        println!(
            "Toast banners are off until this program exits. If a banner still appears, the setting may apply after you sign out."
        );
    }
    let exit_code = if config.command == Command::Smoke {
        1
    } else {
        0
    };
    arm_timeout(config.timeout, exit_code);
    install_ctrl_c();
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let ready = if config.command == Command::Tour {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let tour_config = config.clone();
        thread::spawn(move || {
            if ready_rx.recv().is_err() {
                return;
            }
            if let Err(error) = run_tour(&tour_config)
                && let Ok(mut slot) = TOUR_ERROR.lock()
            {
                *slot = Some(error);
            }
            request_quit();
        });
        Some(ready_tx)
    } else {
        None
    };

    let result = show_and_pump(&config, ready);
    if let Some(error) = TOUR_ERROR.lock().ok().and_then(|slot| slot.clone()) {
        return Err(error);
    }
    result
}

fn show_and_pump(
    config: &Config,
    ready: Option<std::sync::mpsc::Sender<()>>,
) -> Result<(), String> {
    GUI_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
    let monitors = enum_monitors()?;
    let rects = windows_for(&monitors)?;
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| error.to_string())?;
    let instance = HINSTANCE::from(module);
    let class_name = wide(CLASS_NAME);
    let brush = unsafe { CreateSolidBrush(COLORREF(MARKER)) };
    if brush.0.is_null() {
        return Err("could not create the pink brush".into());
    }
    let font_name = wide("Segoe UI");
    let font = unsafe {
        CreateFontW(
            -22,
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            FF_DONTCARE.0 as u32,
            PCWSTR(font_name.as_ptr()),
        )
    };
    let (font, font_owned) = if font.0.is_null() {
        let stock = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
        (HFONT(stock.0), false)
    } else {
        (font, true)
    };
    unsafe {
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            hCursor: cursor,
            hbrBackground: brush,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err("could not register the overlay window class".into());
        }
    }
    let lines = overlay_lines(config);
    GUI.with(|slot| {
        *slot.borrow_mut() = Some(Gui {
            windows: Vec::new(),
            lines,
            brush,
            font,
            font_owned,
            class_name,
            reassert_timer: config.reassert_timer,
            reassert_pos: config.reassert_on_pos_change,
            close_shell: config.close_shell_surfaces,
            quitting: false,
            alpha: config.alpha,
        });
    });
    let mut created = Vec::new();
    for rect in &rects {
        created.push(create_overlay(instance, rect, config.alpha)?);
    }
    GUI.with(|slot| {
        if let Some(gui) = slot.borrow_mut().as_mut() {
            gui.windows = created;
        }
    });
    let timer_hwnd = GUI.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|gui| gui.windows.first().map(|window| window.hwnd))
    });
    if let Some(hwnd) = timer_hwnd {
        if let Err(error) = unsafe {
            RegisterHotKey(
                Some(hwnd),
                HOTKEY_ID,
                MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
                VK_Q.0 as u32,
            )
        } {
            eprintln!(
                "Hotkey {HOTKEY_LABEL} is unavailable ({error}). Ctrl+C in this window still closes it, and it still closes itself."
            );
        }
        if config.reassert_timer || config.close_shell_surfaces {
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::SetTimer(
                    Some(hwnd),
                    TIMER_ID,
                    REASSERT_INTERVAL_MS,
                    None,
                );
            }
        }
    }
    remember_hwnds();
    set_note(OverlayNote::Intro);
    println!(
        "Pink overlay is up on {} monitor(s). Exit: {HOTKEY_LABEL}.",
        rects.len()
    );
    if let Some(ready) = ready {
        let _ = ready.send(());
    }
    if config.command == Command::Smoke {
        let report = smoke_report(config);
        println!("{}", report.json());
        let ok = report.checks_ok;
        request_quit();
        pump_until_quit();
        destroy_windows();
        if ok {
            Ok(())
        } else {
            Err("smoke checks failed".into())
        }
    } else {
        pump_until_quit();
        destroy_windows();
        Ok(())
    }
}

fn create_overlay(
    instance: HINSTANCE,
    rect: &MonitorRect,
    alpha: u8,
) -> Result<LiveWindow, String> {
    let class = GUI.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|gui| gui.class_name.as_ptr())
            .ok_or_else(|| "overlay state is missing".to_string())
    })?;
    let title = wide("overlay spike");
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            PCWSTR(class),
            PCWSTR(title.as_ptr()),
            WS_POPUP,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|error| error.to_string())?;
    unsafe {
        SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA)
            .map_err(|error| error.to_string())?;
        SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)
            .map_err(|error| error.to_string())?;
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .map_err(|error| error.to_string())?;
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Ok(LiveWindow { hwnd, rect: *rect })
}

fn pump_until_quit() {
    unsafe {
        let mut msg = MSG::default();
        loop {
            let status = GetMessageW(&mut msg, None, 0, 0);
            if status.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    unsafe {
        while Instant::now() < deadline {
            let mut msg = MSG::default();
            if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            } else {
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn destroy_windows() {
    let hwnds = GUI.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|gui| {
                gui.windows
                    .iter()
                    .map(|window| window.hwnd)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    unsafe {
        for hwnd in hwnds {
            let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
            let _ = DestroyWindow(hwnd);
        }
    }
    if let Ok(mut slot) = HWNDS.lock() {
        slot.clear();
    }
    GUI.with(|slot| {
        if let Some(gui) = slot.borrow_mut().take() {
            unsafe {
                if gui.font_owned {
                    let _ = DeleteObject(HGDIOBJ(gui.font.0));
                }
                let _ = DeleteObject(HGDIOBJ(gui.brush.0));
            }
        }
    });
}

fn set_note(note: OverlayNote) {
    if let Ok(mut slot) = STATUS.lock() {
        *slot = note;
    }
    let hwnds = HWNDS.lock().map(|slot| slot.clone()).unwrap_or_default();
    for bits in hwnds {
        unsafe {
            let _ = InvalidateRect(Some(HWND(bits as *mut core::ffi::c_void)), None, false);
        }
    }
}

fn remember_hwnds() {
    let hwnds = GUI.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|gui| {
                gui.windows
                    .iter()
                    .map(|window| window.hwnd.0 as isize)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    if let Ok(mut slot) = HWNDS.lock() {
        *slot = hwnds;
    }
}

fn request_quit() {
    GUI.with(|slot| {
        if let Some(gui) = slot.borrow_mut().as_mut() {
            gui.quitting = true;
        }
    });
    let thread = GUI_THREAD.load(Ordering::SeqCst);
    if thread != 0 {
        unsafe {
            let _ = PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

fn arm_timeout(timeout: Duration, exit_code: i32) {
    thread::spawn(move || {
        thread::sleep(timeout);
        if FINISHED.load(Ordering::SeqCst) {
            return;
        }
        cleanup_all();
        request_quit();
        thread::sleep(Duration::from_secs(2));
        std::process::exit(exit_code);
    });
}

fn cleanup_all() {
    CLEANUP_ONCE.call_once(|| {
        FINISHED.store(true, Ordering::SeqCst);
        let _ = registry::restore();
        kill_fullscreen_child();
        toast::remove_shortcut();
    });
}

fn finish() {
    cleanup_all();
}

fn install_ctrl_c() {
    unsafe extern "system" fn handler(ctrl: u32) -> windows::core::BOOL {
        if ctrl == CTRL_C_EVENT || ctrl == CTRL_BREAK_EVENT {
            cleanup_all();
            std::process::exit(0);
        }
        windows::Win32::Foundation::FALSE
    }
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(handler), true);
    }
}

fn run_tour(config: &Config) -> Result<(), String> {
    let build = registry::windows_build().unwrap_or(0);
    println!("Windows build {build}.");
    if unsafe { windows::Win32::UI::Shell::IsUserAnAdmin() }.as_bool() {
        println!(
            "This window is running as admin, so Windows may not show a User Account Control prompt. That is expected when you passed --allow-admin."
        );
    }
    let fullscreen_secs = config.pause.as_secs().min(config.fullscreen_secs);
    for (index, step) in tour::steps(build, config.only, fullscreen_secs)
        .iter()
        .enumerate()
    {
        println!();
        println!("=== Step {}: {} ===", index + 1, step.id.title());
        println!("{}", step.look_for);
        println!("Take a phone photo now.");
        set_note(OverlayNote::Step {
            number: index + 1,
            title: step.id.title(),
            look_for: step.look_for,
        });
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let before = shell::visible_surfaces();
        let opened = match perform(step, build) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("This surface did not open: {error}");
                eprintln!("Write that in the notes. The tour continues.");
                false
            }
        };
        confirm_overlay(step, &before, opened)?;
        if !matches!(step.action, Action::Fullscreen { .. }) {
            thread::sleep(tour::step_wait(config.pause, step.id, fullscreen_secs));
            dismiss(step, config);
        }
    }
    println!();
    println!("Tour finished. The pink overlay is closing.");
    Ok(())
}

fn perform(step: &Step, _build: u32) -> Result<(), String> {
    match step.action {
        Action::Chord(chord) => input::chord(chord),
        Action::Toast => toast::show(),
        Action::Uac { file, params, .. } => {
            let file = file.to_string();
            let params = params.to_string();
            thread::spawn(move || {
                if let Err(error) = input::launch_uac(&file, &params) {
                    eprintln!("UAC did not open: {error}");
                }
            });
            Ok(())
        }
        Action::Fullscreen { max_secs } => launch_fullscreen(max_secs),
    }
}

fn confirm_overlay(step: &Step, before: &[ShellSurface], opened: bool) -> Result<(), String> {
    if !overlay_is_up() {
        println!("surface_opened_while_overlay: overlay-closed");
        return Err("the tour ran after the overlay closed".into());
    }
    let Some(kind) = shell_surface_for(step.id) else {
        if opened && step.id == SurfaceId::FullscreenExclusive {
            println!("surface_opened_while_overlay: {}", step.id.title());
        }
        return Ok(());
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut found = false;
    loop {
        if !overlay_is_up() {
            println!("surface_opened_while_overlay: overlay-closed");
            return Err("the tour ran after the overlay closed".into());
        }
        let now = shell::visible_surfaces();
        if now.contains(&kind) && !before.contains(&kind) {
            found = true;
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    if found {
        println!("surface_opened_while_overlay: {}", step.id.title());
    } else {
        println!(
            "surface_opened_while_overlay: not-found {}",
            step.id.title()
        );
    }
    Ok(())
}

fn shell_surface_for(id: SurfaceId) -> Option<ShellSurface> {
    match id {
        SurfaceId::Start => Some(ShellSurface::Start),
        SurfaceId::TaskView => Some(ShellSurface::TaskView),
        SurfaceId::NotificationCenter => Some(ShellSurface::NotificationCenter),
        SurfaceId::MonitorPlugIn => Some(ShellSurface::Project),
        _ => None,
    }
}

fn overlay_is_up() -> bool {
    let hwnds = HWNDS.lock().map(|slot| slot.clone()).unwrap_or_default();
    hwnds
        .iter()
        .any(|bits| unsafe { IsWindow(Some(HWND(*bits as *mut core::ffi::c_void))).as_bool() })
}

fn dismiss(step: &Step, config: &Config) {
    match step.dismiss {
        tour::Dismiss::RepeatChord => {
            if config.close_shell_surfaces {
                let _ = input::escape();
            } else if let Action::Chord(chord) = step.action {
                let _ = input::chord(chord);
            }
        }
        tour::Dismiss::ClickNo => {
            println!("If the User Account Control prompt is still up, click No. Do not click Yes.");
            set_note(OverlayNote::ClickNo);
        }
        tour::Dismiss::ExitsItself | tour::Dismiss::Expires => {}
    }
}

fn launch_fullscreen(seconds: u64) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let child = std::process::Command::new(exe)
        .args(["fullscreen", "--seconds", &seconds.to_string()])
        .spawn()
        .map_err(|error| error.to_string())?;
    if let Ok(mut slot) = FULLSCREEN_CHILD.lock() {
        *slot = Some(child);
    }
    let start = Instant::now();
    loop {
        let done = FULLSCREEN_CHILD
            .lock()
            .ok()
            .and_then(|mut slot| {
                slot.as_mut()
                    .and_then(|child| child.try_wait().ok().flatten())
            })
            .is_some();
        if done {
            if let Ok(mut slot) = FULLSCREEN_CHILD.lock() {
                *slot = None;
            }
            return Ok(());
        }
        if start.elapsed() >= Duration::from_secs(seconds.saturating_add(3)) {
            kill_fullscreen_child();
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn kill_fullscreen_child() {
    if let Ok(mut slot) = FULLSCREEN_CHILD.lock()
        && let Some(mut child) = slot.take()
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn enum_monitors() -> Result<Vec<MonitorRect>, String> {
    let mut found = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(monitor_enum),
            LPARAM(&mut found as *mut Vec<MonitorRect> as isize),
        )
        .ok()
        .map_err(|error| error.to_string())?;
    }
    if found.is_empty() {
        return Err("no monitors were reported".into());
    }
    Ok(found)
}

unsafe extern "system" fn monitor_enum(
    _monitor: windows::Win32::Graphics::Gdi::HMONITOR,
    _hdc: HDC,
    rect: *mut RECT,
    data: LPARAM,
) -> windows::core::BOOL {
    unsafe {
        let list = &mut *(data.0 as *mut Vec<MonitorRect>);
        let rect = &*rect;
        list.push(MonitorRect {
            x: rect.left,
            y: rect.top,
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
        });
        windows::Win32::Foundation::TRUE
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_NCHITTEST => LRESULT(HIT_TRANSPARENT),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_WINDOWPOSCHANGED => {
                reassert_on_pos(hwnd, lparam);
                LRESULT(0)
            }
            WM_TIMER => {
                on_timer();
                LRESULT(0)
            }
            WM_HOTKEY => {
                cleanup_all();
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                recover_monitors();
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if wparam.0 == VK_F13.0 as usize {
                    PROBE_SAW_F13.store(true, Ordering::SeqCst);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_DESTROY => LRESULT(0),
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn paint(hwnd: HWND) {
    let Some((lines, brush, font, rect)) = GUI.with(|slot| {
        let gui = slot.borrow();
        let gui = gui.as_ref()?;
        let rect = gui
            .windows
            .iter()
            .find(|window| window.hwnd == hwnd)
            .map(|window| window.rect);
        Some((gui.lines.clone(), gui.brush, gui.font, rect))
    }) else {
        return;
    };
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut paint);
        if hdc.0.is_null() {
            return;
        }
        let mut client = RECT::default();
        if GetWindowRect(hwnd, &mut client).is_ok() {
            client = RECT {
                left: 0,
                top: 0,
                right: client.right - client.left,
                bottom: client.bottom - client.top,
            };
        }
        let _ = FillRect(hdc, &client, brush);
        let previous = SelectObject(hdc, HGDIOBJ(font.0));
        SetBkMode(hdc, TRANSPARENT);
        let _ = SetTextColor(hdc, COLORREF(0x00FF_FFFF));
        let mut text_rect = RECT {
            left: client.left + 48,
            top: client.top + 48,
            right: client.right - 48,
            bottom: client.bottom - 48,
        };
        let mut block = String::new();
        let note = STATUS
            .lock()
            .map(|slot| *slot)
            .unwrap_or(OverlayNote::Intro);
        block.push_str(&note_text(note));
        block.push_str("\n\n");
        for line in &lines {
            block.push_str(line);
            block.push('\n');
        }
        if let Some(rect) = rect {
            block.push_str(&format!(
                "Covering {},{} {}x{}.\n",
                rect.x, rect.y, rect.width, rect.height
            ));
        }
        let mut wide_text = wide(&block);
        if wide_text.len() > 1 {
            wide_text.pop();
        }
        let _ = DrawTextW(hdc, &mut wide_text, &mut text_rect, DT_WORDBREAK);
        if !previous.0.is_null() {
            SelectObject(hdc, previous);
        }
        let _ = EndPaint(hwnd, &paint);
    }
}

fn reassert_on_pos(hwnd: HWND, lparam: LPARAM) {
    let enabled = GUI.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|gui| gui.reassert_pos && !gui.quitting)
    });
    if !enabled {
        return;
    }
    let insert_after_topmost = unsafe {
        let pos = &*(lparam.0 as *const WINDOWPOS);
        pos.flags
            .contains(windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER)
            || pos.hwndInsertAfter == HWND_TOPMOST
    };
    if insert_after_topmost || REASSERTING.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
    REASSERTING.store(false, Ordering::SeqCst);
}

fn note_text(note: OverlayNote) -> String {
    match note {
        OverlayNote::Intro => {
            "The pink layer is up. Clicking and typing still work. The current step is written here."
                .into()
        }
        OverlayNote::Step {
            number,
            title,
            look_for,
        } => format!("Step {number}: {title}.\n{look_for}"),
        OverlayNote::ClickNo => {
            "If User Account Control is still up, click No. Never click Yes.".into()
        }
    }
}

fn on_timer() {
    let (reassert, close) = GUI
        .with(|slot| {
            slot.borrow().as_ref().map(|gui| {
                (
                    gui.reassert_timer && !gui.quitting,
                    gui.close_shell && !gui.quitting,
                )
            })
        })
        .unwrap_or((false, false));
    if reassert {
        let hwnds = GUI.with(|slot| {
            slot.borrow()
                .as_ref()
                .map(|gui| {
                    gui.windows
                        .iter()
                        .map(|window| window.hwnd)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        });
        unsafe {
            for hwnd in hwnds {
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }
    if close {
        shell::dismiss_visible();
    }
}

fn recover_monitors() {
    let Ok(monitors) = enum_monitors() else {
        return;
    };
    let Ok(rects) = windows_for(&monitors) else {
        return;
    };
    let Some((instance, alpha)) = GUI.with(|slot| {
        let gui = slot.borrow();
        let gui = gui.as_ref()?;
        let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.ok()?;
        Some((HINSTANCE::from(module), gui.alpha))
    }) else {
        return;
    };
    let current = GUI.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|gui| {
                gui.windows
                    .iter()
                    .map(|window| window.rect)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    let plan = crate::coverage::reconcile(&current, &rects);
    for rect in plan.close {
        let hwnd = GUI.with(|slot| {
            let mut gui = slot.borrow_mut();
            let gui = gui.as_mut()?;
            let index = gui.windows.iter().position(|window| window.rect == rect)?;
            Some(gui.windows.remove(index).hwnd)
        });
        if let Some(hwnd) = hwnd {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
    for rect in plan.create {
        if let Ok(window) = create_overlay(instance, &rect, alpha) {
            GUI.with(|slot| {
                if let Some(gui) = slot.borrow_mut().as_mut() {
                    gui.windows.push(window);
                }
            });
        }
    }
    remember_hwnds();
}

struct SmokeReport {
    checks_ok: bool,
    capture_excluded: bool,
    click_through: bool,
    topmost: bool,
    blocks_input: bool,
    monitors: usize,
    covered: bool,
    capture_sample: &'static str,
    input_probe: &'static str,
    reassert_pos: &'static str,
    reassert_timer: &'static str,
    shell_fixture: &'static str,
}

impl SmokeReport {
    fn json(&self) -> String {
        format!(
            r#"{{"checks_ok":{},"capture_excluded":{},"click_through":{},"topmost":{},"blocks_input":{},"monitors":{},"covered":{},"capture_sample":"{}","input_probe":"{}","reassert_pos":"{}","reassert_timer":"{}","shell_fixture":"{}","exited":"{}"}}"#,
            self.checks_ok,
            self.capture_excluded,
            self.click_through,
            self.topmost,
            self.blocks_input,
            self.monitors,
            self.covered,
            self.capture_sample,
            self.input_probe,
            self.reassert_pos,
            self.reassert_timer,
            self.shell_fixture,
            if self.checks_ok {
                "checks-passed"
            } else {
                "checks-failed"
            },
        )
    }
}

fn smoke_report(config: &Config) -> SmokeReport {
    let windows = GUI.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|gui| {
                gui.windows
                    .iter()
                    .map(|window| LiveWindow {
                        hwnd: window.hwnd,
                        rect: window.rect,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    let monitors = enum_monitors().unwrap_or_default();
    let mut capture_excluded = !windows.is_empty();
    let mut click_through = !windows.is_empty();
    let mut topmost = !windows.is_empty();
    let mut covered = windows.len() == monitors.len() && !monitors.is_empty();
    for window in &windows {
        unsafe {
            let mut affinity = 0u32;
            if GetWindowDisplayAffinity(window.hwnd, &mut affinity).is_err()
                || affinity != CAPTURE_EXCLUDED
            {
                capture_excluded = false;
            }
            let style = GetWindowLongPtrW(window.hwnd, GWL_EXSTYLE) as u32;
            if style & EX_OVERLAY != EX_OVERLAY {
                topmost = false;
                click_through = false;
            }
            let packed = pack_point(
                window.rect.x + window.rect.width / 2,
                window.rect.y + window.rect.height / 2,
            );
            let hit = windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                window.hwnd,
                WM_NCHITTEST,
                Some(WPARAM(0)),
                Some(packed),
            );
            if hit.0 != HTTRANSPARENT as isize {
                click_through = false;
            }
            let mut got = RECT::default();
            if GetWindowRect(window.hwnd, &mut got).is_err()
                || got.left != window.rect.x
                || got.top != window.rect.y
                || got.right - got.left != window.rect.width
                || got.bottom - got.top != window.rect.height
            {
                covered = false;
            }
        }
    }
    for monitor in &monitors {
        if !windows.iter().any(|window| window.rect == *monitor) {
            covered = false;
        }
    }
    let capture_sample = capture_sample(&windows);
    let input_probe = input_probe();
    let blocks_input = input_probe == "blocked";
    let reassert_pos = if config.reassert_on_pos_change {
        if reassert_pos_check(&windows) {
            "restored"
        } else {
            "failed"
        }
    } else {
        "not-requested"
    };
    let reassert_timer = if config.reassert_timer {
        if reassert_timer_check(&windows) {
            "restored"
        } else {
            "failed"
        }
    } else {
        "not-requested"
    };
    let shell_fixture = if config.shell_fixture && config.close_shell_surfaces {
        if shell_fixture_check() {
            "left-open"
        } else {
            "failed"
        }
    } else {
        "not-requested"
    };
    let checks_ok = capture_excluded
        && click_through
        && topmost
        && !blocks_input
        && covered
        && !monitors.is_empty()
        && capture_sample != "visible"
        && reassert_pos != "failed"
        && reassert_timer != "failed"
        && shell_fixture != "failed";
    SmokeReport {
        checks_ok,
        capture_excluded,
        click_through,
        topmost,
        blocks_input,
        monitors: monitors.len(),
        covered,
        capture_sample,
        input_probe,
        reassert_pos,
        reassert_timer,
        shell_fixture,
    }
}

fn capture_sample(windows: &[LiveWindow]) -> &'static str {
    let composed = unsafe {
        DwmIsCompositionEnabled()
            .ok()
            .is_some_and(|enabled| enabled.as_bool())
    };
    if !composed {
        return "skipped-no-dwm";
    }
    for window in windows {
        let x = window.rect.x + window.rect.width / 2;
        let y = window.rect.y + window.rect.height / 2;
        match gdi_capture_pixel(x, y) {
            Some(color) if is_marker(color) => return "visible",
            Some(_) => {}
            None => return "skipped-no-dwm",
        }
    }
    "excluded"
}

fn gdi_capture_pixel(x: i32, y: i32) -> Option<u32> {
    unsafe {
        let screen = GetDC(None);
        if screen.0.is_null() {
            return None;
        }
        let memory = CreateCompatibleDC(Some(screen));
        if memory.0.is_null() {
            let _ = ReleaseDC(None, screen);
            return None;
        }
        let bitmap = CreateCompatibleBitmap(screen, 1, 1);
        if bitmap.0.is_null() {
            let _ = DeleteDC(memory);
            let _ = ReleaseDC(None, screen);
            return None;
        }
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let copied = BitBlt(memory, 0, 0, 1, 1, Some(screen), x, y, SRCCOPY);
        let color = if copied.is_ok() {
            let pixel = GetPixel(memory, 0, 0);
            if pixel.0 == CLR_INVALID {
                None
            } else {
                Some(pixel.0)
            }
        } else {
            None
        };
        if !previous.0.is_null() {
            SelectObject(memory, previous);
        }
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        let _ = ReleaseDC(None, screen);
        color
    }
}

fn is_marker(color: u32) -> bool {
    let red = color & 0xff;
    let green = (color >> 8) & 0xff;
    let blue = (color >> 16) & 0xff;
    red > 200 && green < 40 && blue > 200
}

fn input_probe() -> &'static str {
    let probe = match create_probe() {
        Ok(hwnd) => hwnd,
        Err(_) => return "skipped-no-foreground",
    };
    unsafe {
        let foreground = GetForegroundWindow();
        let foreground_thread =
            windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(foreground, None);
        let ours = GetCurrentThreadId();
        if foreground_thread != 0 && foreground_thread != ours {
            let _ = AttachThreadInput(foreground_thread, ours, true);
            let _ = SetForegroundWindow(probe);
            let _ = AttachThreadInput(foreground_thread, ours, false);
        } else {
            let _ = SetForegroundWindow(probe);
        }
    }
    pump_for(Duration::from_millis(50));
    if unsafe { GetForegroundWindow() } != probe {
        unsafe {
            let _ = DestroyWindow(probe);
        }
        return "skipped-no-foreground";
    }
    PROBE_SAW_F13.store(false, Ordering::SeqCst);
    if input::key(VK_F13, false, false).is_err() || input::key(VK_F13, true, false).is_err() {
        unsafe {
            let _ = DestroyWindow(probe);
        }
        return "skipped-no-foreground";
    }
    pump_for(Duration::from_millis(100));
    let saw = PROBE_SAW_F13.load(Ordering::SeqCst);
    unsafe {
        let _ = DestroyWindow(probe);
    }
    if saw { "reached-probe" } else { "blocked" }
}

fn create_probe() -> Result<HWND, String> {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| error.to_string())?;
    let instance = HINSTANCE::from(module);
    let class_name = wide(PROBE_CLASS);
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
        CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(wide("probe").as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            40,
            40,
            200,
            120,
            None,
            None,
            Some(instance),
            None,
        )
        .map_err(|error| error.to_string())
    }
}

fn reassert_pos_check(windows: &[LiveWindow]) -> bool {
    let Some(window) = windows.first() else {
        return false;
    };
    unsafe {
        if SetWindowPos(
            window.hwnd,
            Some(HWND_NOTOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
        .is_err()
        {
            return false;
        }
        let style = GetWindowLongPtrW(window.hwnd, GWL_EXSTYLE) as u32;
        style & WS_EX_TOPMOST.0 == WS_EX_TOPMOST.0
    }
}

fn reassert_timer_check(windows: &[LiveWindow]) -> bool {
    let Some(window) = windows.first() else {
        return false;
    };
    let competitor = match create_competitor() {
        Ok(hwnd) => hwnd,
        Err(_) => return false,
    };
    unsafe {
        let _ = PostMessageW(Some(window.hwnd), WM_TIMER, WPARAM(TIMER_ID), LPARAM(0));
    }
    pump_for(Duration::from_millis(200));
    let above = z_above(window.hwnd, competitor);
    unsafe {
        let _ = DestroyWindow(competitor);
    }
    above
}

fn create_competitor() -> Result<HWND, String> {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| error.to_string())?;
    let instance = HINSTANCE::from(module);
    let class_name = wide("spike-overlay-competitor");
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(competitor_proc),
            hInstance: instance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_TOPMOST,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(wide("competitor").as_ptr()),
            WS_POPUP | WS_VISIBLE,
            80,
            80,
            40,
            40,
            None,
            None,
            Some(instance),
            None,
        )
        .map_err(|error| error.to_string())
    }
}

unsafe extern "system" fn competitor_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn z_above(above: HWND, below: HWND) -> bool {
    let mut current = unsafe { GetWindow(above, GW_HWNDFIRST).ok() };
    let mut seen_above = false;
    while let Some(hwnd) = current {
        if hwnd == above {
            seen_above = true;
        }
        if hwnd == below {
            return seen_above;
        }
        current = unsafe { GetWindow(hwnd, GW_HWNDNEXT).ok() };
    }
    false
}

fn shell_fixture_check() -> bool {
    let Ok(hwnd) = create_fixture() else {
        return false;
    };
    shell::dismiss_visible();
    pump_for(Duration::from_millis(200));
    let open = unsafe { IsWindow(Some(hwnd)).as_bool() };
    if open {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
    open
}

fn create_fixture() -> Result<HWND, String> {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| error.to_string())?;
    let instance = HINSTANCE::from(module);
    let class_name = wide("Windows.UI.Core.CoreWindow");
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(competitor_proc),
            hInstance: instance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
        CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(wide("Start").as_ptr()),
            WS_POPUP | WS_VISIBLE,
            120,
            120,
            80,
            80,
            None,
            None,
            Some(instance),
            None,
        )
        .map_err(|error| error.to_string())
    }
}

fn pack_point(x: i32, y: i32) -> LPARAM {
    LPARAM((((y as u16 as u32) << 16) | (x as u16 as u32)) as isize)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_bits_match_the_sdk() {
        let sdk = (WS_EX_TOPMOST
            | WS_EX_TRANSPARENT
            | WS_EX_TOOLWINDOW
            | WS_EX_LAYERED
            | WS_EX_NOACTIVATE)
            .0;
        assert_eq!(sdk, EX_OVERLAY);
        assert_eq!(WDA_EXCLUDEFROMCAPTURE.0, CAPTURE_EXCLUDED);
        assert_eq!(HTTRANSPARENT as isize, HIT_TRANSPARENT);
    }

    #[test]
    fn magenta_matches_the_marker_and_desktop_blue_does_not() {
        assert!(is_marker(MARKER));
        assert!(!is_marker(0x00FF_0000));
    }
}
