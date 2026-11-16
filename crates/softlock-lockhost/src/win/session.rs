//! Dev-launch session. The window thread owns the lock controller. The hook
//! thread only stores events.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicPtr, AtomicU8, AtomicU32, AtomicU64, Ordering,
};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use softlock_core::{
    Event, FailReason, GuardHealth, HEARTBEAT_INTERVAL, HeartbeatReport, LockController,
    LockSource, LockState, MethodId, Overlay, Secret, Verdict, verify_softlock_pin,
};
use zeroize::Zeroize;

use super::failclosed;
use super::guard;
use super::overlay;
use super::platform::WinPlatform;
use super::queue::{EventQueue, KIND_CLICK, KIND_KEY, KIND_WHEEL, RawEvent};
use crate::real_lock::RealLock;

/// How long to wait before asking Windows to lock again after both calls failed.
const OS_LOCK_RETRY: Duration = Duration::from_secs(1);

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
        let ui = thread::spawn(move || {
            let on_panic = Arc::clone(&worker);
            let panicked = std::panic::catch_unwind(AssertUnwindSafe(|| ui_main(worker, tx)));
            if panicked.is_err() {
                // The window thread is gone, so nothing is left to draw the
                // overlay or take a PIN. Try the OS lock, then drop the hooks.
                // Leaving them installed would swallow input with no UI.
                emergency(&on_panic);
            }
        });
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

    /// Makes `lock_or_disconnect` return `decide(false, false)` without
    /// calling `LockWorkStation` or `WTSDisconnectSession`.
    #[cfg(feature = "simulated-physical")]
    pub fn stub_os_lock_unavailable(&self) {
        self.shared.os_lock_stub.store(1, Ordering::Release);
    }

    pub fn os_lock_attempts(&self) -> u32 {
        self.shared.os_lock_attempts.load(Ordering::Acquire)
    }

    pub fn os_lock_error(&self) -> bool {
        self.shared.os_lock_error.load(Ordering::Acquire)
    }

    pub fn overlay_notice(&self) -> Option<&'static str> {
        if self.os_lock_error() {
            Some(overlay::OS_LOCK_NOTICE)
        } else {
            None
        }
    }

    /// Same flag the liveness thread sets when the marker is missed.
    pub fn signal_guard_down(&self) {
        self.shared.fail.store(true, Ordering::Release);
    }

    #[cfg(feature = "simulated-physical")]
    pub fn ignore_liveness_marker(&self) {
        self.shared
            .ignore_liveness_marker
            .store(true, Ordering::Release);
    }

    #[cfg(feature = "simulated-physical")]
    pub fn hold_heartbeats(&self) {
        self.shared.hold_heartbeats.store(true, Ordering::Release);
    }

    pub fn heartbeat_missed(&self) -> bool {
        self.shared.heartbeat_missed.load(Ordering::Acquire)
    }

    #[cfg(feature = "simulated-physical")]
    pub fn panic_window_thread(&self) {
        self.shared.panic_window.store(true, Ordering::Release);
    }

    /// Wakes the hook thread so its loop panics. The callback itself is not
    /// this path: a panic there aborts the process.
    #[cfg(feature = "simulated-physical")]
    pub fn panic_hook_thread(&self) {
        self.shared.panic_hook.store(true, Ordering::Release);
        let thread_id = self.shared.hook_thread_id.load(Ordering::Acquire);
        if thread_id != 0 {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    thread_id,
                    0,
                    windows::Win32::Foundation::WPARAM(0),
                    windows::Win32::Foundation::LPARAM(0),
                );
            }
        }
    }

    #[cfg(feature = "simulated-physical")]
    pub fn panic_liveness_thread(&self) {
        self.shared.panic_liveness.store(true, Ordering::Release);
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
    /// Cursor position last accepted from an injected move. A swallowed
    /// physical move has already shifted the cursor by the time the hook
    /// runs; `snap_gen` asks the window thread to put it back.
    pub cursor_x: AtomicI32,
    pub cursor_y: AtomicI32,
    pub cursor_gen: AtomicU64,
    pub snap_gen: AtomicU64,
    /// `0` calls Windows. Anything else makes both OS lock paths fail.
    pub os_lock_stub: AtomicU8,
    pub os_lock_attempts: AtomicU32,
    pub os_lock_unavailable: AtomicBool,
    pub os_lock_error: AtomicBool,
    pub ignore_liveness_marker: AtomicBool,
    pub hold_heartbeats: AtomicBool,
    pub panic_window: AtomicBool,
    pub panic_hook: AtomicBool,
    pub panic_liveness: AtomicBool,
    pub failing_closed: AtomicBool,
    pub heartbeat_missed: AtomicBool,
    pub emergency_ran: AtomicBool,
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
            cursor_x: AtomicI32::new(0),
            cursor_y: AtomicI32::new(0),
            cursor_gen: AtomicU64::new(0),
            snap_gen: AtomicU64::new(0),
            os_lock_stub: AtomicU8::new(0),
            os_lock_attempts: AtomicU32::new(0),
            os_lock_unavailable: AtomicBool::new(false),
            os_lock_error: AtomicBool::new(false),
            ignore_liveness_marker: AtomicBool::new(false),
            hold_heartbeats: AtomicBool::new(false),
            panic_window: AtomicBool::new(false),
            panic_hook: AtomicBool::new(false),
            panic_liveness: AtomicBool::new(false),
            failing_closed: AtomicBool::new(false),
            heartbeat_missed: AtomicBool::new(false),
            emergency_ran: AtomicBool::new(false),
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
    let mut clock = Instant::now();
    let mut os_retry_at: Option<Instant> = None;
    loop {
        if shared.panic_window.load(Ordering::Acquire) {
            panic!("lock host window thread");
        }
        advance_clock(&mut controller, &shared, &mut clock);
        note_controller(&controller, &shared);
        guard::restore_cursor(&shared);
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        if shared.fail.swap(false, Ordering::AcqRel) {
            report_guard_down(&mut controller, &shared);
            note_controller(&controller, &shared);
        }
        retry_os_lock(&mut controller, &shared, &mut os_retry_at);
        if shared.display_changed.swap(false, Ordering::AcqRel) {
            overlay::destroy(&shared);
            if overlay::create(&shared).is_err() {
                report_guard_down(&mut controller, &shared);
                note_controller(&controller, &shared);
            }
        }
        // OS lock confirmed: the controller already disarmed. Leave.
        // Unavailable: stay. The guard, the overlay, and the PIN remain.
        if matches!(controller.state(), LockState::OsLocked { .. }) {
            break;
        }
        if matches!(controller.state(), LockState::FailingClosed { .. })
            && !shared.os_lock_unavailable.load(Ordering::Acquire)
        {
            break;
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

/// The controller clock does not move unless told. Wall time is fed in here
/// so a backoff can expire and so a missed heartbeat can fail closed. While
/// the hook is healthy the advance is split into one interval at a time and
/// a heartbeat is sent after each step. A verify can block this thread for
/// longer than three intervals; one jump of that size would look like a
/// dead hook. Holding heartbeats lets the watchdog run for real.
fn advance_clock(
    controller: &mut LockController<WinPlatform>,
    shared: &Shared,
    clock: &mut Instant,
) {
    let now = Instant::now();
    let delta = now.saturating_duration_since(*clock);
    *clock = now;
    if delta.is_zero() {
        return;
    }
    let healthy = !shared.hold_heartbeats.load(Ordering::Acquire)
        && !shared.fail.load(Ordering::Acquire)
        && matches!(controller.state(), LockState::Locked { .. });
    if !healthy {
        controller.handle(Event::AdvanceTime(delta));
        return;
    }
    let mut left = delta;
    while !left.is_zero() {
        let step = left.min(HEARTBEAT_INTERVAL);
        controller.handle(Event::AdvanceTime(step));
        left = left.saturating_sub(step);
        if !matches!(controller.state(), LockState::Locked { .. }) {
            break;
        }
        beat(controller, shared);
    }
}

fn beat(controller: &mut LockController<WinPlatform>, shared: &Shared) {
    if shared.hold_heartbeats.load(Ordering::Acquire) {
        return;
    }
    if !matches!(controller.state(), LockState::Locked { .. }) {
        return;
    }
    let seq = controller.next_heartbeat_seq();
    controller.handle(Event::Heartbeat(HeartbeatReport {
        seq,
        guard: GuardHealth {
            hook_alive: true,
            last_physical_input_ms: shared.physical_ms(),
        },
        overlay: Overlay::health(controller.platform()),
    }));
}

fn note_controller(controller: &LockController<WinPlatform>, shared: &Shared) {
    if let LockState::FailingClosed { reason } = controller.state() {
        shared.failing_closed.store(true, Ordering::Release);
        if matches!(reason, FailReason::HeartbeatMissed) {
            shared.heartbeat_missed.store(true, Ordering::Release);
        }
    }
}

fn retry_os_lock(
    controller: &mut LockController<WinPlatform>,
    shared: &Shared,
    os_retry_at: &mut Option<Instant>,
) {
    if !shared.os_lock_unavailable.load(Ordering::Acquire) {
        return;
    }
    let due = os_retry_at.get_or_insert_with(|| Instant::now() + OS_LOCK_RETRY);
    if Instant::now() < *due {
        return;
    }
    *due = Instant::now() + OS_LOCK_RETRY;
    match failclosed::lock_or_disconnect(failclosed::console_session()) {
        RealLock::Unavailable => {
            shared.os_lock_error.store(true, Ordering::Release);
            overlay::invalidate(shared);
        }
        RealLock::Workstation | RealLock::Disconnected => {
            shared.os_lock_unavailable.store(false, Ordering::Release);
            shared.os_lock_error.store(false, Ordering::Release);
            controller.handle(Event::OsLockConfirmed);
        }
    }
}

/// OS lock, then hooks. Used when a lock-host thread panics. Does not join
/// the caller: the hook and liveness threads can be the caller.
pub(crate) fn worker_panic(shared: &Shared) {
    let _ = failclosed::lock_or_disconnect(failclosed::console_session());
    shared.fail.store(true, Ordering::Release);
}

fn emergency(shared: &Shared) {
    if shared.emergency_ran.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = failclosed::lock_or_disconnect(failclosed::console_session());
    release_session(shared);
    shared.locked.store(false, Ordering::Release);
}

pub(crate) fn installed_session() -> Option<&'static Shared> {
    let pointer = SESSION.load(Ordering::Acquire);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { &*pointer })
    }
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

pub(crate) fn release_session(shared: &Shared) {
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
            // UTF-8 is at most four bytes per character. Pre-sizing means a
            // full PIN cannot reallocate and leave an old copy behind.
            bytes: Vec::with_capacity(PIN_CHAR_LIMIT as usize * 4),
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
    let secret = Secret::new(&entry.bytes);
    // `Zeroize` for `Vec` clears the length and keeps the allocation, so the
    // next PIN reuses this buffer instead of abandoning a copy.
    entry.bytes.zeroize();
    entry.chars = 0;
    // `pin_chars` stays at the submitted length until the hash and the
    // controller have finished. Publishing zero first lets the next PIN's
    // keys queue during Argon2 and then look lost to a short wait.
    let finish = |shared: &Shared| {
        shared.pin_chars.store(0, Ordering::Release);
        overlay::invalidate(shared);
    };
    // Any PIN, right or wrong, is refused while the controller's backoff is
    // running. `SubmitSecret` is how that refusal is recorded: the controller
    // drops the secret and does not start an unlock.
    if !controller.backoff_delay().is_zero() {
        controller.handle(Event::SubmitSecret {
            method: MethodId::softlock_pin(),
            secret,
        });
        finish(shared);
        return;
    }
    // Argon2id compare, constant time, spec parameters. `Verdict::Verified`
    // here is the credential verdict and carries no nonce.
    let matched = verify_softlock_pin(&secret, &shared.pin_hash) == Verdict::Verified;
    if matched && matches!(controller.state(), LockState::Locked { .. }) {
        drop(secret);
        unlock_with_broker_nonce(controller, shared);
        finish(shared);
        return;
    }
    if matched && shared.os_lock_unavailable.load(Ordering::Acquire) {
        // The controller is `FailingClosed`, so `RequestHello` is refused.
        // A verified PIN is the only release while both OS locks keep failing.
        // Backoff is clear here: a non-zero delay returned above.
        drop(secret);
        release_session(shared);
        shared.prompt_open.store(false, Ordering::Release);
        shared.locked.store(false, Ordering::Release);
        finish(shared);
        return;
    }
    // Wrong PIN, or a PIN the controller will not accept. `verify_secret`
    // stays `Rejected`, so a `Locked` or `FailingClosed` controller records
    // the failure and starts backoff.
    controller.handle(Event::SubmitSecret {
        method: MethodId::softlock_pin(),
        secret,
    });
    finish(shared);
}

fn unlock_with_broker_nonce(controller: &mut LockController<WinPlatform>, shared: &Shared) {
    // Dev launch only. `RequestHello` makes this process draw the nonce with
    // `LaunchNonce::generate` (32 bytes from `getrandom`) and store it here.
    // Ticket 13 must not do that. The Broker owns the nonce, and the lock
    // host has to receive that nonce across the process boundary before
    // `Event::Verified`. Production code does not call `LaunchNonce::from_bytes`.
    controller.handle(Event::RequestHello);
    let Some(nonce) = controller.broker_nonce() else {
        overlay::invalidate(shared);
        return;
    };
    controller.handle(Event::Verified { nonce });
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
