use std::io::BufReader;
use std::net::TcpStream;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, HBRUSH, WHITE_BRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThread, GetCurrentThreadId, OpenThread, SetThreadPriority,
    THREAD_PRIORITY_NORMAL, THREAD_PRIORITY_TIME_CRITICAL, THREAD_SET_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_F24;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, BringWindowToTop, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GetForegroundWindow, GetMessageExtraInfo, GetMessageW, GetSystemMetrics,
    GetWindowRect, GetWindowThreadProcessId, HWND_TOPMOST, LSFW_UNLOCK, LockSetForegroundWindow,
    MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, RegisterClassExW, SM_CXSCREEN, SM_CYSCREEN,
    SPI_GETFOREGROUNDLOCKTIMEOUT, SPI_SETFOREGROUNDLOCKTIMEOUT, SW_SHOW, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SetForegroundWindow, SetWindowPos, SetWindowsHookExW, ShowWindow,
    SystemParametersInfoW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP,
    WM_KEYDOWN, WM_QUIT, WNDCLASSEXW, WS_EX_APPWINDOW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::protocol::{self, Command, Hello, ROLE_ELEVATED, ROLE_SYSTEM, Rect as ProtoRect, Reply};
use crate::verdict::{EXTRA_MARKER, EXTRA_VIS};

use super::args::Args;
use super::callback;
use super::identity;
use super::input;
use super::token;
use super::util::{self, explain, log_line, wide};

enum UiCmd {
    Foreground(Sender<Result<ProtoRect, String>>),
}

struct HookSession {
    thread_id: u32,
    hwnd: u64,
    commands: Sender<UiCmd>,
    join: JoinHandle<()>,
}

pub fn run(args: &Args, role: &str) -> i32 {
    let log_path = args.output_dir.join(format!("{role}.log"));
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| run_role(args, role, &log_path)));
    match outcome {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => {
            log_line(&log_path, &error);
            eprintln!("{role}: {error}");
            1
        }
        Err(_) => {
            log_line(&log_path, "hook process panicked");
            eprintln!("{role}: panicked");
            1
        }
    }
}

fn run_role(args: &Args, role: &str, log_path: &std::path::Path) -> Result<(), String> {
    if role == ROLE_SYSTEM {
        callback::SWALLOW_MARKERS.store(true, Ordering::SeqCst);
    }
    let id = identity::current();
    let il = match token::current_query_token() {
        Ok(token) => token::integrity_rid(token.handle()).unwrap_or(0),
        Err(_) => 0,
    };
    log_line(
        log_path,
        &format!(
            "role={role} pid={} il={il} user={} session={} station={} desktop={} timeout={} raw={}",
            id.pid, id.user, id.session, id.station, id.desktop, id.timeout_ms, id.timeout_raw
        ),
    );
    let hook = start_hook(role != ROLE_SYSTEM, role, log_path)?;
    log_line(log_path, &format!("hook thread {}", hook.thread_id));
    let stream = connect(args.port)?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(180)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let mut writer = stream
        .try_clone()
        .map_err(|error| format!("clone socket: {error}"))?;
    let mut reader = BufReader::new(stream);
    protocol::write_msg(
        &mut writer,
        &Hello {
            role: role.to_string(),
            pid: id.pid,
            il,
            user: id.user,
            session: id.session,
            station: id.station,
            desktop: id.desktop,
            timeout_ms: id.timeout_ms,
            timeout_raw: id.timeout_raw,
            hwnd: hook.hwnd,
        },
    )?;
    let result = command_loop(role, &hook, &mut reader, &mut writer, log_path);
    let _ = unsafe { PostThreadMessageW(hook.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    let _ = hook.join.join();
    result
}

fn command_loop(
    role: &str,
    hook: &HookSession,
    reader: &mut BufReader<TcpStream>,
    writer: &mut TcpStream,
    log_path: &std::path::Path,
) -> Result<(), String> {
    loop {
        let command: Command = protocol::read_msg(reader)?;
        log_line(log_path, &format!("command {command:?}"));
        match command {
            Command::Foreground => {
                let reply = match request_foreground(hook) {
                    Ok(rect) => Reply::Foreground {
                        ok: true,
                        rect: Some(rect),
                        error: None,
                    },
                    Err(error) => Reply::Foreground {
                        ok: false,
                        rect: None,
                        error: Some(error),
                    },
                };
                protocol::write_msg(writer, &reply)?;
            }
            Command::Reset => {
                callback::reset_counts();
                protocol::write_msg(writer, &Reply::Reset)?;
            }
            Command::Counts => {
                let (vis_seen, vis_injected, load_seen, marker_seen, window_keys) =
                    callback::counts();
                let (slow_entered, slow_returned) = callback::slow_state();
                protocol::write_msg(
                    writer,
                    &Reply::Counts {
                        vis_seen,
                        vis_injected,
                        load_seen,
                        marker_seen,
                        window_keys,
                        slow_entered,
                        slow_returned,
                    },
                )?;
            }
            Command::Slow { timeout_ms } => {
                let reply = if role == ROLE_SYSTEM {
                    arm_slow(timeout_ms, hook.thread_id)
                } else {
                    Reply::Slow {
                        healthy_before: false,
                        sleep_ms: 0,
                        marker_seen: 0,
                        error: Some("slow probe runs in the system hook".to_string()),
                    }
                };
                protocol::write_msg(writer, &reply)?;
            }
            Command::Quit => {
                protocol::write_msg(writer, &Reply::Bye)?;
                return Ok(());
            }
            Command::Disarm => {
                callback::disarm();
                protocol::write_msg(writer, &Reply::Disarm)?;
            }
        }
    }
}

fn request_foreground(hook: &HookSession) -> Result<ProtoRect, String> {
    let (tx, rx) = mpsc::channel();
    hook.commands
        .send(UiCmd::Foreground(tx))
        .map_err(|_| "hook thread is gone".to_string())?;
    unsafe {
        PostThreadMessageW(hook.thread_id, WM_APP, WPARAM(0), LPARAM(0))
            .map_err(|error| explain(error, "PostThreadMessageW"))?;
    }
    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| "foreground timed out".to_string())?
}

fn connect(port: u16) -> Result<TcpStream, String> {
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut last = "not attempted".to_string();
    while Instant::now() < deadline {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                last = error.to_string();
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Err(format!("connect to supervisor: {last}"))
}

fn arm_slow(timeout_ms: u64, thread_id: u32) -> Reply {
    callback::reset_counts();
    if !marker_roundtrip() {
        return Reply::Slow {
            healthy_before: false,
            sleep_ms: 0,
            marker_seen: 0,
            error: Some("marker was not seen before the slow callback".to_string()),
        };
    }
    // Time-critical priority can skip the desktop timeout. The load test is
    // already finished, so the removal probe runs at normal priority.
    lower_hook_priority(thread_id);
    // Past the timeout, with the remainder small enough that detecting removal
    // after the callback returns still fits in 500 ms.
    let sleep_ms = timeout_ms.saturating_add(600).min(1_200);
    callback::SLOW_RETURNED.store(0, Ordering::Relaxed);
    callback::SLOW_ENTERED.store(0, Ordering::Relaxed);
    callback::SLOW_MS.store(sleep_ms as u32, Ordering::Relaxed);
    callback::SLOW_ARM.store(true, Ordering::Release);
    Reply::Slow {
        healthy_before: true,
        sleep_ms,
        marker_seen: callback::MARKER_SEEN.load(Ordering::Relaxed),
        error: None,
    }
}

fn lower_hook_priority(thread_id: u32) {
    unsafe {
        if let Ok(thread) = OpenThread(THREAD_SET_INFORMATION, false, thread_id) {
            let _ = SetThreadPriority(thread, THREAD_PRIORITY_NORMAL);
            let _ = CloseHandle(thread);
        }
    }
}

fn marker_roundtrip() -> bool {
    let before = callback::MARKER_SEEN.load(Ordering::Relaxed);
    if input::send_key_pairs(VK_F24, EXTRA_MARKER, 1).is_err() {
        return false;
    }
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        if callback::MARKER_SEEN.load(Ordering::Relaxed) > before {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

fn start_hook(
    with_window: bool,
    role: &str,
    log_path: &std::path::Path,
) -> Result<HookSession, String> {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let role = role.to_string();
    let log_path = log_path.to_path_buf();
    let join = thread::spawn(move || hook_main(with_window, role, log_path, cmd_rx, ready_tx));
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok((thread_id, hwnd))) => Ok(HookSession {
            thread_id,
            hwnd,
            commands: cmd_tx,
            join,
        }),
        Ok(Err(error)) => {
            let _ = join.join();
            Err(error)
        }
        Err(_) => Err("hook thread did not start".to_string()),
    }
}

fn hook_main(
    with_window: bool,
    role: String,
    log_path: std::path::PathBuf,
    commands: Receiver<UiCmd>,
    ready: Sender<Result<(u32, u64), String>>,
) {
    let raised = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) };
    if let Err(error) = raised {
        let _ = ready.send(Err(explain(error, "SetThreadPriority")));
        return;
    }
    let hwnd = if with_window {
        match create_window(&role) {
            Ok(hwnd) => Some(hwnd),
            Err(error) => {
                let _ = ready.send(Err(error));
                return;
            }
        }
    } else {
        None
    };
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(callback::keyboard_proc),
            Some(module_instance()),
            0,
        )
    };
    let hook = match hook {
        Ok(hook) => hook,
        Err(error) => {
            if let Some(hwnd) = hwnd {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
            let _ = ready.send(Err(explain(error, "keyboard hook")));
            return;
        }
    };
    let mut message = MSG::default();
    unsafe {
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
    }
    let thread_id = unsafe { GetCurrentThreadId() };
    let hwnd_bits = hwnd.as_ref().map(|window| window.0 as u64).unwrap_or(0);
    log_line(
        &log_path,
        &format!(
            "hook installed thread={thread_id} window={}",
            hwnd.is_some()
        ),
    );
    let _ = ready.send(Ok((thread_id, hwnd_bits)));
    loop {
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 == 0 || status.0 == -1 {
            break;
        }
        if message.message == WM_APP {
            while let Ok(command) = commands.try_recv() {
                match command {
                    UiCmd::Foreground(reply) => {
                        let result = match hwnd {
                            Some(hwnd) => become_foreground(hwnd),
                            None => Err("this process has no probe window".to_string()),
                        };
                        let _ = reply.send(result);
                    }
                }
            }
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            let _ = DispatchMessageW(&message);
        }
    }
    unsafe {
        let _ = UnhookWindowsHookEx(hook);
        if let Some(hwnd) = hwnd {
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn become_foreground(hwnd: HWND) -> Result<ProtoRect, String> {
    unsafe {
        let _ = LockSetForegroundWindow(LSFW_UNLOCK);
        let _ = AllowSetForegroundWindow(0xFFFF_FFFF);
        input::send_alt_keyup();
        let foreground = GetForegroundWindow();
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let ours = GetCurrentThreadId();
        let attached = foreground_thread != 0
            && foreground_thread != ours
            && AttachThreadInput(foreground_thread, ours, true).as_bool();
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = BringWindowToTop(hwnd);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(foreground_thread, ours, false);
        }
        if GetForegroundWindow() != hwnd {
            relax_foreground_lock(hwnd);
        }
        if GetForegroundWindow() != hwnd {
            let mut pid = 0u32;
            let other = GetForegroundWindow();
            let _ = GetWindowThreadProcessId(other, Some(&mut pid));
            return Err(format!(
                "foreground is pid {pid} hwnd {:x}, wanted {:x}",
                other.0 as usize, hwnd.0 as usize
            ));
        }
        window_rect(hwnd)
    }
}

fn relax_foreground_lock(hwnd: HWND) {
    unsafe {
        let mut previous = 0u32;
        let _ = SystemParametersInfoW(
            SPI_GETFOREGROUNDLOCKTIMEOUT,
            0,
            Some((&mut previous as *mut u32).cast()),
            Default::default(),
        );
        let mut zero = 0u32;
        let _ = SystemParametersInfoW(
            SPI_SETFOREGROUNDLOCKTIMEOUT,
            0,
            Some((&mut zero as *mut u32).cast()),
            Default::default(),
        );
        input::send_alt_keyup();
        let _ = SetForegroundWindow(hwnd);
        let _ = SystemParametersInfoW(
            SPI_SETFOREGROUNDLOCKTIMEOUT,
            0,
            Some((&mut previous as *mut u32).cast()),
            Default::default(),
        );
    }
}

fn window_rect(hwnd: HWND) -> Result<ProtoRect, String> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect).map_err(|error| explain(error, "GetWindowRect"))? };
    Ok(ProtoRect {
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    })
}

fn create_window(role: &str) -> Result<HWND, String> {
    register_class()?;
    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    if screen_w < 200 || screen_h < 200 {
        return Err(format!("screen is {screen_w}x{screen_h}"));
    }
    let width = 320.min(screen_w - 40);
    let height = 200.min(screen_h - 40);
    let (x, y) = if role == ROLE_ELEVATED {
        (screen_w - width - 16, screen_h - height - 16)
    } else {
        (16, screen_h - height - 16)
    };
    let title = wide(&format!("input-hook-{role}"));
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_APPWINDOW,
            w!("InputHookProbe"),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(module_instance()),
            None,
        )
        .map_err(|error| explain(error, "CreateWindowExW"))?
    };
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            width,
            height,
            SWP_SHOWWINDOW,
        );
    }
    Ok(hwnd)
}

fn register_class() -> Result<(), String> {
    use std::sync::OnceLock;
    static CLASS: OnceLock<Result<(), String>> = OnceLock::new();
    CLASS
        .get_or_init(|| {
            let brush = unsafe { GetStockObject(WHITE_BRUSH) };
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: module_instance(),
                hbrBackground: HBRUSH(brush.0),
                lpszClassName: w!("InputHookProbe"),
                ..WNDCLASSEXW::default()
            };
            let atom = unsafe { RegisterClassExW(&class) };
            if atom == 0 {
                Err(util::last_error("RegisterClassExW"))
            } else {
                Ok(())
            }
        })
        .clone()
}

fn module_instance() -> HINSTANCE {
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.unwrap_or_default();
    HINSTANCE(module.0)
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_KEYDOWN
            && wparam.0 == u32::from(windows::Win32::UI::Input::KeyboardAndMouse::VK_F13.0) as usize
        {
            let extra = GetMessageExtraInfo();
            if extra.0 as usize == EXTRA_VIS {
                callback::WINDOW_KEYS.fetch_add(1, Ordering::Relaxed);
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}
