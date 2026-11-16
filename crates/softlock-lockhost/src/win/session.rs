//! Dev-launch session. The window thread owns the lock controller. The hook
//! thread only stores events.

use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use softlock_core::{
    Event, GuardHealth, HeartbeatReport, LockController, LockSource, LockState, MethodId, Overlay,
    Secret,
};
use zeroize::Zeroize;

use super::failclosed;
use super::guard;
use super::overlay;
use super::platform::WinPlatform;
use super::queue::{EventQueue, KIND_CLICK, KIND_KEY, KIND_WHEEL, RawEvent};

/// Colour the overlay fills with. Captures must not contain it.
pub const OVERLAY_MARKER: (u8, u8, u8) = (1, 255, 2);

pub(crate) static SESSION: AtomicPtr<Shared> = AtomicPtr::new(std::ptr::null_mut());

const PIN_CHAR_LIMIT: u32 = 128;

pub struct DevConfig {
    pub pin_hash: String,
    /// Runs after the guard is installed and before the overlay is shown.
    pub while_arming: Option<Box<dyn FnOnce() + Send>>,
}

pub struct DevError {
    message: String,
}

impl DevError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for DevError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::fmt::Debug for DevError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DevError")
            .field("message", &self.message)
            .finish()
    }
}

impl std::error::Error for DevError {}

pub struct DevLock {
    shared: Arc<Shared>,
    ui: Option<JoinHandle<()>>,
    elapsed: Duration,
}

impl DevLock {
    pub fn arm(config: DevConfig) -> Result<Self, DevError> {
        let _ = crate::dev_launch_marker_bytes();
        let shared = Shared::new(config.pin_hash, config.while_arming);
        let worker = Arc::clone(&shared);
        let (tx, rx) = std::sync::mpsc::channel();
        let ui = thread::spawn(move || ui_main(worker, tx));
        match rx.recv_timeout(Duration::from_secs(8)) {
            Ok(Ok(elapsed)) => Ok(Self {
                shared,
                ui: Some(ui),
                elapsed,
            }),
            Ok(Err(error)) => {
                let _ = ui.join();
                release_session(&shared);
                Err(error)
            }
            Err(_) => {
                shared.stop.store(true, Ordering::Release);
                let thread_id = shared.ui_thread_id.load(Ordering::Acquire);
                if thread_id != 0 {
                    unsafe {
                        let _ = windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                            thread_id,
                            windows::Win32::UI::WindowsAndMessaging::WM_QUIT,
                            windows::Win32::Foundation::WPARAM(0),
                            windows::Win32::Foundation::LPARAM(0),
                        );
                    }
                }
                let _ = ui.join();
                release_session(&shared);
                Err(DevError::new("lock host did not finish arming"))
            }
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub fn is_locked(&self) -> bool {
        self.shared.locked.load(Ordering::Acquire)
    }

    pub fn prompt_open(&self) -> bool {
        self.shared.prompt_open.load(Ordering::Acquire)
    }

    pub fn pin_chars(&self) -> u32 {
        self.shared.pin_chars.load(Ordering::Acquire)
    }

    pub fn overlay_windows(&self) -> Vec<isize> {
        self.shared
            .windows
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

impl Drop for DevLock {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let thread_id = self.shared.ui_thread_id.load(Ordering::Acquire);
        if thread_id != 0 {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    thread_id,
                    windows::Win32::UI::WindowsAndMessaging::WM_QUIT,
                    windows::Win32::Foundation::WPARAM(0),
                    windows::Win32::Foundation::LPARAM(0),
                );
            }
        }
        if let Some(ui) = self.ui.take() {
            let _ = ui.join();
        }
        release_session(&self.shared);
    }
}

pub(crate) struct Shared {
    pub queue: EventQueue,
    pub last_event: AtomicU64,
    pub last_physical_ms: AtomicU64,
    pub saw_physical: AtomicBool,
    pub marker_seen: AtomicU64,
    pub wake: AtomicU64,
    pub prompt_open: AtomicBool,
    pub pin_chars: AtomicU32,
    pub locked: AtomicBool,
    pub stop: AtomicBool,
    pub fail: AtomicBool,
    pub hook_thread_id: AtomicU32,
    pub ui_thread_id: AtomicU32,
    pub monitors: AtomicU32,
    pub capture_excluded: AtomicBool,
    pub topmost: AtomicBool,
    pub execution: AtomicBool,
    pub display_changed: AtomicBool,
    pub windows: Mutex<Vec<isize>>,
    pub hook: Mutex<Option<JoinHandle<()>>>,
    pub liveness: Mutex<Option<JoinHandle<()>>>,
    pub pin_hash: String,
    pub while_arming: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    events_tx: Mutex<std::sync::mpsc::Sender<softlock_core::SessionEvent>>,
    events_rx: Mutex<Option<std::sync::mpsc::Receiver<softlock_core::SessionEvent>>>,
}

impl Shared {
    fn new(pin_hash: String, while_arming: Option<Box<dyn FnOnce() + Send>>) -> Arc<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        Arc::new(Self {
            queue: EventQueue::new(),
            last_event: AtomicU64::new(0),
            last_physical_ms: AtomicU64::new(0),
            saw_physical: AtomicBool::new(false),
            marker_seen: AtomicU64::new(0),
            wake: AtomicU64::new(0),
            prompt_open: AtomicBool::new(false),
            pin_chars: AtomicU32::new(0),
            locked: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            fail: AtomicBool::new(false),
            hook_thread_id: AtomicU32::new(0),
            ui_thread_id: AtomicU32::new(0),
            monitors: AtomicU32::new(0),
            capture_excluded: AtomicBool::new(false),
            topmost: AtomicBool::new(false),
            execution: AtomicBool::new(false),
            display_changed: AtomicBool::new(false),
            windows: Mutex::new(Vec::new()),
            hook: Mutex::new(None),
            liveness: Mutex::new(None),
            pin_hash,
            while_arming: Mutex::new(while_arming),
            events_tx: Mutex::new(tx),
            events_rx: Mutex::new(Some(rx)),
        })
    }

    pub(crate) fn take_events(&self) -> std::sync::mpsc::Receiver<softlock_core::SessionEvent> {
        self.events_rx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
            .unwrap_or_else(|| {
                let (tx, rx) = std::sync::mpsc::channel();
                *self
                    .events_tx
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = tx;
                rx
            })
    }

    pub(crate) fn physical_ms(&self) -> Option<u64> {
        if self.saw_physical.load(Ordering::Acquire) {
            Some(self.last_physical_ms.load(Ordering::Acquire))
        } else {
            None
        }
    }
}

fn ui_main(shared: Arc<Shared>, ready: std::sync::mpsc::Sender<Result<Duration, DevError>>) {
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        shared.ui_thread_id.store(
            windows::Win32::System::Threading::GetCurrentThreadId(),
            Ordering::Release,
        );
    }
    let mut controller = LockController::new(WinPlatform::new(Arc::clone(&shared)));
    let started = Instant::now();
    controller.handle(Event::LockRequested {
        source: LockSource::LocalApi,
    });
    let elapsed = started.elapsed();
    if !matches!(controller.state(), LockState::Locked { .. }) {
        shutdown(&shared);
        let _ = ready.send(Err(DevError::new("arming did not reach the locked state")));
        return;
    }
    shared.locked.store(true, Ordering::Release);
    if ready.send(Ok(elapsed)).is_err() {
        shutdown(&shared);
        return;
    }
    {
        let liveness_shared = Arc::clone(&shared);
        let worker = thread::spawn(move || guard::liveness(liveness_shared));
        *shared
            .liveness
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(worker);
    }
    let mut entry = PinEntry::default();
    let mut last_minute = overlay::minute_now();
    loop {
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        if shared.fail.swap(false, Ordering::AcqRel) {
            report_guard_down(&mut controller, &shared);
            break;
        }
        if shared.display_changed.swap(false, Ordering::AcqRel) {
            overlay::destroy(&shared);
            if overlay::create(&shared).is_err() {
                report_guard_down(&mut controller, &shared);
                break;
            }
        }
        let minute = overlay::minute_now();
        if minute != last_minute {
            last_minute = minute;
            overlay::invalidate(&shared);
        }
        drain(&mut controller, &shared, &mut entry);
        if !shared.locked.load(Ordering::Acquire) {
            break;
        }
        if !pump_message() && shared.stop.load(Ordering::Acquire) {
            break;
        }
    }
    entry.bytes.zeroize();
    shutdown(&shared);
}

fn report_guard_down(controller: &mut LockController<WinPlatform>, shared: &Shared) {
    if !matches!(
        controller.state(),
        LockState::Locked { .. } | LockState::Arming { .. }
    ) {
        return;
    }
    let seq = controller.next_heartbeat_seq();
    controller.handle(Event::Heartbeat(HeartbeatReport {
        seq,
        guard: GuardHealth {
            hook_alive: false,
            last_physical_input_ms: shared.physical_ms(),
        },
        overlay: Overlay::health(controller.platform()),
    }));
}

fn shutdown(shared: &Shared) {
    release_session(shared);
    shared.locked.store(false, Ordering::Release);
}

fn release_session(shared: &Shared) {
    overlay::destroy(shared);
    guard::remove(shared);
    failclosed::clear_execution_state(shared);
}

fn pump_message() -> bool {
    unsafe {
        let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        let pending = windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
            &mut message,
            None,
            0,
            0,
            windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
        );
        if !pending.as_bool() {
            thread::sleep(Duration::from_millis(5));
            return true;
        }
        if message.message == windows::Win32::UI::WindowsAndMessaging::WM_QUIT {
            return false;
        }
        let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&message);
        let _ = windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&message);
        true
    }
}

struct PinEntry {
    bytes: Vec<u8>,
    chars: u32,
    layout: windows::Win32::UI::Input::KeyboardAndMouse::HKL,
}

impl Default for PinEntry {
    fn default() -> Self {
        Self {
            bytes: Vec::new(),
            chars: 0,
            layout: windows::Win32::UI::Input::KeyboardAndMouse::HKL(std::ptr::null_mut()),
        }
    }
}

impl Drop for PinEntry {
    fn drop(&mut self) {
        self.bytes.zeroize();
        self.chars = 0;
    }
}

fn drain(controller: &mut LockController<WinPlatform>, shared: &Shared, entry: &mut PinEntry) {
    while let Some(event) = shared.queue.pop() {
        match event.kind {
            KIND_KEY => on_key(controller, shared, entry, event),
            KIND_CLICK | KIND_WHEEL if !shared.prompt_open.load(Ordering::Acquire) => {
                open_prompt(controller, shared, entry);
            }
            _ => {}
        }
    }
    let wakes = shared.wake.swap(0, Ordering::AcqRel);
    if wakes > 0 && !shared.prompt_open.load(Ordering::Acquire) {
        open_prompt(controller, shared, entry);
    }
}

fn open_prompt(
    controller: &mut LockController<WinPlatform>,
    shared: &Shared,
    entry: &mut PinEntry,
) {
    if shared.prompt_open.swap(true, Ordering::AcqRel) {
        return;
    }
    entry.layout = owner_layout();
    controller.handle(Event::PromptOpened {
        method: MethodId::softlock_pin(),
    });
    overlay::invalidate(shared);
}

fn on_key(
    controller: &mut LockController<WinPlatform>,
    shared: &Shared,
    entry: &mut PinEntry,
    event: RawEvent,
) {
    if !shared.prompt_open.load(Ordering::Acquire) {
        open_prompt(controller, shared, entry);
        return;
    }
    const VK_BACK: u16 = 0x08;
    const VK_RETURN: u16 = 0x0D;
    if event.vk == VK_RETURN {
        submit(controller, shared, entry);
        return;
    }
    if event.vk == VK_BACK {
        pop_char(entry);
        shared.pin_chars.store(entry.chars, Ordering::Release);
        overlay::invalidate(shared);
        return;
    }
    if entry.chars >= PIN_CHAR_LIMIT {
        return;
    }
    if let Some(mut encoded) = translate(entry.layout, event) {
        entry.bytes.extend_from_slice(&encoded);
        encoded.zeroize();
        entry.chars = entry.chars.saturating_add(1);
        shared.pin_chars.store(entry.chars, Ordering::Release);
        overlay::invalidate(shared);
    }
}

fn submit(controller: &mut LockController<WinPlatform>, shared: &Shared, entry: &mut PinEntry) {
    let mut bytes = std::mem::take(&mut entry.bytes);
    let secret = Secret::new(&bytes);
    bytes.zeroize();
    entry.chars = 0;
    shared.pin_chars.store(0, Ordering::Release);
    controller.handle(Event::SubmitSecret {
        method: MethodId::softlock_pin(),
        secret,
    });
    if matches!(controller.state(), LockState::Unlocked) {
        shared.prompt_open.store(false, Ordering::Release);
        shared.locked.store(false, Ordering::Release);
        return;
    }
    overlay::invalidate(shared);
}

fn pop_char(entry: &mut PinEntry) {
    if entry.chars == 0 {
        return;
    }
    while entry
        .bytes
        .last()
        .is_some_and(|byte| byte & 0b1100_0000 == 0b1000_0000)
    {
        if let Some(slot) = entry.bytes.last_mut() {
            *slot = 0;
        }
        entry.bytes.pop();
    }
    if let Some(slot) = entry.bytes.last_mut() {
        *slot = 0;
    }
    entry.bytes.pop();
    entry.chars -= 1;
}

fn translate(
    layout: windows::Win32::UI::Input::KeyboardAndMouse::HKL,
    event: RawEvent,
) -> Option<Vec<u8>> {
    let mut state = [0u8; 256];
    unsafe {
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardState(&mut state);
    }
    // Ambient Shift or Alt would turn a digit into a symbol. The forwarded
    // key is the one the hook stored, without whatever else is latched.
    for modifier in [0x10usize, 0x11, 0x12, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5] {
        state[modifier] = 0;
    }
    let index = event.vk as usize;
    if index < state.len() {
        state[index] |= 0x80;
    }
    let mut buffer = [0u16; 8];
    let written = unsafe {
        windows::Win32::UI::Input::KeyboardAndMouse::ToUnicodeEx(
            u32::from(event.vk),
            u32::from(event.scan),
            &state,
            &mut buffer,
            0,
            Some(layout),
        )
    };
    let mut produced = Vec::new();
    if written > 0 {
        let count = usize::try_from(written).unwrap_or(0).min(buffer.len());
        for ch in char::decode_utf16(buffer[..count].iter().copied()).filter_map(Result::ok) {
            if ch.is_control() {
                continue;
            }
            let mut encoded = [0u8; 4];
            let text = ch.encode_utf8(&mut encoded);
            produced.extend_from_slice(text.as_bytes());
            encoded.fill(0);
        }
    }
    buffer.fill(0);
    if produced.is_empty() {
        fallback_digit(event.vk).map(|byte| vec![byte])
    } else {
        Some(produced)
    }
}

fn fallback_digit(vk: u16) -> Option<u8> {
    if (u16::from(b'0')..=u16::from(b'9')).contains(&vk) {
        Some(vk as u8)
    } else if (0x60..=0x69).contains(&vk) {
        Some(b'0' + (vk as u8 - 0x60))
    } else {
        None
    }
}

fn owner_layout() -> windows::Win32::UI::Input::KeyboardAndMouse::HKL {
    unsafe {
        let foreground = windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
        let thread =
            windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(foreground, None);
        let layout = windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout(thread);
        if layout.0.is_null() {
            windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout(0)
        } else {
            layout
        }
    }
}
