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
    Event, FailReason, GuardHealth, HEARTBEAT_INTERVAL, HeartbeatReport, LaunchNonce,
    LockController, LockSource, LockState, MethodId, Overlay, OverlayHealth, PinAuthority,
    PinDecision, Secret,
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
    /// Decides a PIN. The host does not hash it and does not draw the nonce.
    pub authority: Box<dyn PinAuthority>,
    /// Runs after the guard is installed and before the overlay is shown.
    pub while_arming: Option<Box<dyn FnOnce() + Send>>,
}

pub(crate) enum PipeCommand {
    Arm { message: Option<String> },
    Disarm,
    OsLock,
    Quit,
}

pub(crate) enum PipeReport {
    Ready {
        monitors: u32,
    },
    Heartbeat {
        seq: u64,
        guard: GuardHealth,
        overlay: OverlayHealth,
    },
    Secret(Secret),
    GuardDown,
}

struct NeverGrant;

impl PinAuthority for NeverGrant {
    fn decide(&mut self, _secret: &Secret, _allow_grant: bool) -> PinDecision {
        PinDecision::Deny
    }
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
        #[cfg(feature = "dev-launch")]
        let _ = crate::dev_launch_marker_bytes();
        let shared = Shared::new(config.authority, config.while_arming);
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
                if on_panic.pipe_mode.load(Ordering::Acquire) {
                    publish(&on_panic, PipeReport::GuardDown);
                    std::process::exit(1);
                }
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

    /// Wakes the hook thread so its message loop panics. A panic inside the
    /// callback is caught there and does not use this flag.
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

    /// The next hook callback panics. The catch swallows that event and asks
    /// the window thread to try the OS lock.
    #[cfg(feature = "simulated-physical")]
    pub fn panic_in_callback(&self) {
        self.shared.panic_in_callback.store(true, Ordering::Release);
    }

    /// Installs the service panic hook. A panic on the hook thread tries the
    /// OS lock and then releases the guard without joining that thread.
    /// Panics on any other thread are forwarded to the hook that was current.
    #[cfg(feature = "simulated-physical")]
    pub fn install_service_panic_hook(&self) {
        let previous = std::panic::take_hook();
        let shared = Arc::clone(&self.shared);
        std::panic::set_hook(Box::new(move |info| {
            if guard::current_thread_is_hook(&shared) {
                host_panic_hook(&shared, info);
            } else {
                previous(info);
            }
        }));
    }

    /// The service panic hook ran on the hook thread. The callback-panic test
    /// uses this to prove the self-join path returned.
    #[cfg(feature = "simulated-physical")]
    pub fn panic_hook_ran_on_hook_thread(&self) -> bool {
        self.shared
            .panic_hook_on_hook_thread
            .load(Ordering::Acquire)
    }

    /// The hook thread has been joined. Its id is 0 only after `remove`.
    /// Tests wait for this before sending a key that must reach the probe.
    /// The product path tries the OS lock first and drops the hooks only
    /// when that lock is unavailable.
    #[cfg(feature = "simulated-physical")]
    pub fn hooks_removed(&self) -> bool {
        self.shared.hook_thread_id.load(Ordering::Acquire) == 0
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
    /// `0` calls Windows. `1` makes both OS lock paths fail. `2` reports a
    /// confirmed workstation lock without calling Windows.
    pub os_lock_stub: AtomicU8,
    pub os_lock_attempts: AtomicU32,
    pub os_lock_unavailable: AtomicBool,
    pub os_lock_error: AtomicBool,
    pub ignore_liveness_marker: AtomicBool,
    pub hold_heartbeats: AtomicBool,
    pub panic_window: AtomicBool,
    pub panic_hook: AtomicBool,
    pub panic_liveness: AtomicBool,
    pub hook_poisoned: AtomicBool,
    #[cfg(feature = "simulated-physical")]
    pub panic_in_callback: AtomicBool,
    /// Panic only on F13. The liveness marker and stray mouse input stay
    /// quiet, so a service test can fire `emergency()` after the host is ready.
    #[cfg(feature = "simulated-physical")]
    pub panic_callback_on_f13: AtomicBool,
    #[cfg(feature = "simulated-physical")]
    pub panic_hook_on_hook_thread: AtomicBool,
    pub failing_closed: AtomicBool,
    pub heartbeat_missed: AtomicBool,
    pub emergency_ran: AtomicBool,
    pub windows: Mutex<Vec<isize>>,
    pub hook: Mutex<Option<JoinHandle<()>>>,
    pub liveness: Mutex<Option<JoinHandle<()>>>,
    pub authority: Mutex<Box<dyn PinAuthority>>,
    pub pipe_mode: AtomicBool,
    pub reports: Mutex<Option<std::sync::mpsc::Sender<PipeReport>>>,
    pub commands: Mutex<Option<std::sync::mpsc::Receiver<PipeCommand>>>,
    pub while_arming: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    events_tx: Mutex<std::sync::mpsc::Sender<softlock_core::SessionEvent>>,
    events_rx: Mutex<Option<std::sync::mpsc::Receiver<softlock_core::SessionEvent>>>,
}

impl Shared {
    fn new(
        authority: Box<dyn PinAuthority>,
        while_arming: Option<Box<dyn FnOnce() + Send>>,
    ) -> Arc<Self> {
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
            hook_poisoned: AtomicBool::new(false),
            #[cfg(feature = "simulated-physical")]
            panic_in_callback: AtomicBool::new(false),
            #[cfg(feature = "simulated-physical")]
            panic_callback_on_f13: AtomicBool::new(false),
            #[cfg(feature = "simulated-physical")]
            panic_hook_on_hook_thread: AtomicBool::new(false),
            failing_closed: AtomicBool::new(false),
            heartbeat_missed: AtomicBool::new(false),
            emergency_ran: AtomicBool::new(false),
            windows: Mutex::new(Vec::new()),
            hook: Mutex::new(None),
            liveness: Mutex::new(None),
            authority: Mutex::new(authority),
            pipe_mode: AtomicBool::new(false),
            reports: Mutex::new(None),
            commands: Mutex::new(None),
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
    apply_simulated_environment(&shared);
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        shared.ui_thread_id.store(
            windows::Win32::System::Threading::GetCurrentThreadId(),
            Ordering::Release,
        );
    }
    if shared.pipe_mode.load(Ordering::Acquire) && !wait_for_arm(&shared) {
        let _ = ready.send(Err(DevError::new("lock host stopped before arming")));
        return;
    }
    #[cfg(feature = "live-fail-closed")]
    if host_ready_fault() == Some("exit") {
        let _ = ready.send(Err(DevError::new("lock host exited before HostReady")));
        std::process::exit(3);
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
    failclosed::publish_armed_host();
    #[cfg(feature = "live-fail-closed")]
    if host_ready_fault() == Some("hold") {
        while !shared.stop.load(Ordering::Acquire) {
            if !pump_message() && shared.stop.load(Ordering::Acquire) {
                break;
            }
        }
        shutdown(&shared);
        let _ = ready.send(Err(DevError::new("lock host held HostReady")));
        return;
    }
    // Before Ready. Lock returns only after Ready, and the test then sends
    // F13. Liveness and mouse input do not trip this flag.
    #[cfg(feature = "simulated-physical")]
    if std::env::var("SOFTLOCK_PANIC").ok().as_deref() == Some("callback") {
        shared.panic_callback_on_f13.store(true, Ordering::Release);
    }
    publish(
        &shared,
        PipeReport::Ready {
            monitors: shared.monitors.load(Ordering::Acquire),
        },
    );
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
    let mut service_gone = false;
    let mut quit_tried = false;
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
        match next_command(&shared) {
            Some(PipeCommand::Disarm) => break,
            Some(PipeCommand::OsLock) => failclosed::schedule_workstation_lock(),
            Some(PipeCommand::Quit) => service_gone = true,
            Some(PipeCommand::Arm { message }) => overlay::set_message(message.as_deref()),
            None => {}
        }
        if service_gone {
            arm_quit_lock(&mut quit_tried, controller.state());
        }
        if let Some(result) = take_quit_os_lock()
            && apply_quit_os_lock(&shared, &mut controller, result)
        {
            break;
        }
        if service_gone && failclosed::replacement_host_armed() {
            break;
        }
        if shared.fail.swap(false, Ordering::AcqRel) {
            report_guard_down(&mut controller, &shared);
            note_controller(&controller, &shared);
            // A poisoned callback cannot take a PIN. If the OS lock did not
            // engage, drop the hooks the same way a window-thread panic does.
            if shared.hook_poisoned.load(Ordering::Acquire)
                && shared.os_lock_unavailable.load(Ordering::Acquire)
            {
                emergency(&shared);
                break;
            }
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

/// `hold` never sends `HostReady`. `exit` ends the process before the hooks.
/// Both are ignored unless this binary was built with `live-fail-closed` and
/// the machine is a hosted runner that is allowed to lock.
#[cfg(feature = "live-fail-closed")]
fn host_ready_fault() -> Option<&'static str> {
    if softlock_windows::runner_may_lock().is_err() {
        return None;
    }
    match std::env::var("SOFTLOCK_HOST_READY").ok().as_deref() {
        Some("hold") => Some("hold"),
        Some("exit") => Some("exit"),
        _ => None,
    }
}

fn force_unhealthy_guard() -> bool {
    #[cfg(feature = "live-fail-closed")]
    {
        if softlock_windows::runner_may_lock().is_err() {
            return false;
        }
        match std::env::var("SOFTLOCK_FORCE_GUARD_DOWN") {
            Ok(value) if value == "1" => true,
            Ok(value) => std::path::Path::new(&value).is_file(),
            Err(_) => false,
        }
    }
    #[cfg(not(feature = "live-fail-closed"))]
    {
        false
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
    let guard = GuardHealth {
        hook_alive: !force_unhealthy_guard(),
        last_physical_input_ms: shared.physical_ms(),
    };
    let overlay = Overlay::health(controller.platform());
    controller.handle(Event::Heartbeat(HeartbeatReport {
        seq,
        guard,
        overlay,
    }));
    publish(
        shared,
        PipeReport::Heartbeat {
            seq,
            guard,
            overlay,
        },
    );
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

/// OS lock, then hooks. The hook thread must not join itself; `remove` skips
/// that join. The OS lock attempt stays first.
fn host_panic_hook(shared: &Shared, info: &std::panic::PanicHookInfo) {
    eprintln!("lock host panic: {info}");
    #[cfg(feature = "simulated-physical")]
    if guard::current_thread_is_hook(shared) {
        shared
            .panic_hook_on_hook_thread
            .store(true, Ordering::Release);
    }
    // OS lock before any join. `remove` joins the hook thread, and this hook
    // is that thread when a callback panics. Unhooking from inside the
    // callback also leaves SendInput broken, so the callback path only asks
    // the hook thread to exit after it returns.
    let _ = failclosed::lock_or_disconnect(failclosed::console_session());
    if guard::in_hook_callback() {
        shared.stop.store(true, Ordering::Release);
        let thread_id = shared.hook_thread_id.load(Ordering::Acquire);
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
        if shared.pipe_mode.load(Ordering::Acquire) {
            publish(shared, PipeReport::GuardDown);
            std::process::exit(1);
        }
        return;
    }
    emergency(shared);
    if shared.pipe_mode.load(Ordering::Acquire) {
        publish(shared, PipeReport::GuardDown);
        std::process::exit(1);
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
    failclosed::clear_armed_host();
    shared.locked.store(false, Ordering::Release);
}

const QUIT_IDLE: u8 = 0;
const QUIT_RUNNING: u8 = 1;
const QUIT_WORKSTATION: u8 = 2;
const QUIT_DISCONNECTED: u8 = 3;
const QUIT_UNAVAILABLE: u8 = 4;
const QUIT_DONE: u8 = 5;

/// The quit path's OS lock. The window thread only reads the result.
static QUIT_OS: AtomicU8 = AtomicU8::new(QUIT_IDLE);

/// The quit tests share `QUIT_OS` and the held OS-lock flag.
#[cfg(all(test, feature = "simulated-physical"))]
static QUIT_TEST: Mutex<()> = Mutex::new(());

/// Quit is the service pipe closing. The OS lock can take the whole
/// confirmation budget, so it runs off the window thread. The hooks stay
/// until the result says the session is locked, or another host has armed.
/// A service crash locks the workstation only while the session is guarded.
/// Unlocked, unlocking, recovering, and an already OS-locked session stay as they are.
fn service_quit_locks(state: &LockState) -> bool {
    matches!(
        state,
        LockState::Locked { .. } | LockState::Arming { .. } | LockState::FailingClosed { .. }
    )
}

/// `quit_tried` stays false while the session is unguarded, so a crash during
/// unlock or recovery still locks once the session is Locked or FailingClosed.
fn arm_quit_lock(quit_tried: &mut bool, state: &LockState) -> bool {
    if *quit_tried || !service_quit_locks(state) {
        return false;
    }
    *quit_tried = true;
    schedule_quit_os_lock();
    true
}

fn schedule_quit_os_lock() {
    if QUIT_OS
        .compare_exchange(QUIT_IDLE, QUIT_RUNNING, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    thread::spawn(|| {
        let result = failclosed::lock_or_disconnect(failclosed::console_session());
        let code = match result {
            RealLock::Workstation => QUIT_WORKSTATION,
            RealLock::Disconnected => QUIT_DISCONNECTED,
            RealLock::Unavailable => QUIT_UNAVAILABLE,
        };
        QUIT_OS.store(code, Ordering::Release);
        #[cfg(all(test, feature = "simulated-physical"))]
        failclosed::note_quit_os_finished();
    });
}

fn take_quit_os_lock() -> Option<RealLock> {
    let code = QUIT_OS.load(Ordering::Acquire);
    let result = match code {
        QUIT_WORKSTATION => RealLock::Workstation,
        QUIT_DISCONNECTED => RealLock::Disconnected,
        QUIT_UNAVAILABLE => RealLock::Unavailable,
        _ => return None,
    };
    if QUIT_OS
        .compare_exchange(code, QUIT_DONE, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return None;
    }
    Some(result)
}

fn apply_quit_os_lock(
    shared: &Shared,
    controller: &mut LockController<WinPlatform>,
    result: RealLock,
) -> bool {
    let guarded = matches!(
        controller.state(),
        LockState::Locked { .. } | LockState::Arming { .. } | LockState::FailingClosed { .. }
    );
    if !guarded {
        return false;
    }
    match result {
        RealLock::Workstation | RealLock::Disconnected => {
            if matches!(
                controller.state(),
                LockState::Locked { .. } | LockState::Arming { .. }
            ) {
                report_guard_down(controller, shared);
            }
            if !matches!(controller.state(), LockState::OsLocked { .. }) {
                controller.handle(Event::OsLockConfirmed);
            }
            matches!(controller.state(), LockState::OsLocked { .. })
        }
        RealLock::Unavailable => {
            shared.os_lock_unavailable.store(true, Ordering::Release);
            false
        }
    }
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
    if shared.pipe_mode.load(Ordering::Acquire) {
        publish(shared, PipeReport::Secret(secret));
        finish(shared);
        return;
    }
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
    // The service crate decides. A grant carries its nonce. This process does
    // not hash the PIN and does not call `RequestHello`.
    let releasing = matches!(controller.state(), LockState::Locked { .. })
        || (matches!(controller.state(), LockState::FailingClosed { .. })
            && shared.os_lock_unavailable.load(Ordering::Acquire));
    let decision = shared
        .authority
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .decide(&secret, releasing);
    match decision {
        PinDecision::Grant(nonce) => {
            drop(secret);
            apply_granted_nonce(controller, shared, nonce);
            finish(shared);
        }
        PinDecision::Hold => {
            drop(secret);
            finish(shared);
        }
        PinDecision::Deny => {
            controller.handle(Event::SubmitSecret {
                method: MethodId::softlock_pin(),
                secret,
            });
            finish(shared);
        }
    }
}

fn apply_granted_nonce(
    controller: &mut LockController<WinPlatform>,
    shared: &Shared,
    nonce: LaunchNonce,
) {
    let bytes = *nonce.as_bytes();
    controller.handle(Event::AdoptLaunchNonce {
        nonce: LaunchNonce::from_bytes(bytes),
    });
    controller.handle(Event::Verified {
        nonce: LaunchNonce::from_bytes(bytes),
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

fn publish(shared: &Shared, report: PipeReport) {
    if let Some(sender) = shared
        .reports
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .as_ref()
    {
        let _ = sender.send(report);
    }
}

fn next_command(shared: &Shared) -> Option<PipeCommand> {
    shared
        .commands
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .as_mut()
        .and_then(|commands| commands.try_recv().ok())
}

fn wait_for_arm(shared: &Shared) -> bool {
    loop {
        if shared.stop.load(Ordering::Acquire) {
            return false;
        }
        match next_command(shared) {
            Some(PipeCommand::Arm { message }) => {
                overlay::set_message(message.as_deref());
                return true;
            }
            Some(PipeCommand::Disarm) => return false,
            Some(PipeCommand::OsLock) => failclosed::schedule_workstation_lock(),
            // Hooks are installed only after Arm. Quit before that is not
            // Locked, Arming, or FailingClosed, so it must not call the OS lock.
            Some(PipeCommand::Quit) => return false,
            None => {
                if !pump_message() && shared.stop.load(Ordering::Acquire) {
                    return false;
                }
            }
        }
    }
}

fn apply_simulated_environment(shared: &Shared) {
    #[cfg(feature = "simulated-physical")]
    {
        match std::env::var("SOFTLOCK_STUB_OS_LOCK").ok().as_deref() {
            Some("succeed") => shared.os_lock_stub.store(2, Ordering::Release),
            // This feature is the test build. A missing stub must not call
            // LockWorkStation; production builds do not compile this branch.
            _ => shared.os_lock_stub.store(1, Ordering::Release),
        }
        if std::env::var("SOFTLOCK_PANIC").ok().as_deref() == Some("after-arm") {
            shared.panic_window.store(true, Ordering::Release);
        }
        if let Ok(text) = std::env::var("SOFTLOCK_RUN_NONCE")
            && let Ok(raw) = text.parse::<u64>()
            && let Some(nonce) = crate::RunNonce::from_extra(raw)
        {
            crate::set_run_nonce(nonce);
        }
    }
    #[cfg(not(feature = "simulated-physical"))]
    {
        let _ = shared;
    }
}

pub fn run_service_host() -> i32 {
    use std::os::windows::io::FromRawHandle;
    // The journal sign-in case asks this build to count injected keys as
    // physical. A release build does not accept the argument. The PIN itself
    // is injected by the liveness thread, which is the thread the hook sees.
    #[cfg(feature = "live-fail-closed")]
    if std::env::args().any(|arg| arg == "--present-physical") {
        crate::classify::arm_present_physical();
    }

    let mut read_handle = None;
    let mut write_handle = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--read" => read_handle = args.next().and_then(|value| value.parse::<usize>().ok()),
            "--write" => write_handle = args.next().and_then(|value| value.parse::<usize>().ok()),
            _ => {}
        }
    }
    let (Some(read_handle), Some(write_handle)) = (read_handle, write_handle) else {
        eprintln!("usage: softlock-lockhost --service-host --read HANDLE --write HANDLE");
        return 2;
    };
    let mut read_file =
        unsafe { std::fs::File::from_raw_handle(read_handle as *mut std::ffi::c_void) };
    let write_file =
        unsafe { std::fs::File::from_raw_handle(write_handle as *mut std::ffi::c_void) };
    let mut welcome = match softlock_core::read_prefixed_frame(&mut read_file) {
        Ok(frame) => frame,
        Err(_) => return 1,
    };
    let nonce = match softlock_core::decode_service_frame(&welcome) {
        Ok(softlock_core::ServiceToHost::Welcome { nonce }) => nonce.to_nonce().ok(),
        _ => None,
    };
    welcome.fill(0);
    let Some(nonce) = nonce else {
        return 1;
    };
    let (report_tx, report_rx) = std::sync::mpsc::channel();
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    let shared = Shared::new(Box::new(NeverGrant), None);
    shared.pipe_mode.store(true, Ordering::Release);
    *shared
        .reports
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = Some(report_tx);
    *shared
        .commands
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = Some(command_rx);
    thread::spawn(move || host_write_loop(write_file, report_rx, nonce));
    thread::spawn(move || host_read_loop(read_file, command_tx));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    // A panic while unwinding can stall in a hook-thread join. Report the
    // guard down and exit before that unwind, so the service sees host-lost.
    let hook_shared = Arc::clone(&shared);
    std::panic::set_hook(Box::new(move |info| {
        host_panic_hook(&hook_shared, info);
    }));
    ui_main(Arc::clone(&shared), ready_tx);
    drop(ready_rx);
    shutdown(&shared);
    0
}

fn host_read_loop(mut file: std::fs::File, commands: std::sync::mpsc::Sender<PipeCommand>) {
    while let Ok(frame) = softlock_core::read_prefixed_frame(&mut file) {
        let command = match softlock_core::decode_service_frame(&frame) {
            Ok(softlock_core::ServiceToHost::Arm { message }) => Some(PipeCommand::Arm { message }),
            Ok(softlock_core::ServiceToHost::Disarm) => Some(PipeCommand::Disarm),
            Ok(softlock_core::ServiceToHost::OsLock) => Some(PipeCommand::OsLock),
            Ok(softlock_core::ServiceToHost::PromptResult { .. }) => None,
            _ => None,
        };
        if let Some(command) = command
            && commands.send(command).is_err()
        {
            break;
        }
    }
    let _ = commands.send(PipeCommand::Quit);
}

fn host_write_loop(
    mut file: std::fs::File,
    reports: std::sync::mpsc::Receiver<PipeReport>,
    nonce: LaunchNonce,
) {
    use softlock_core::{
        host_guard_down_frame, host_heartbeat_frame, host_hello_frame, host_ready_frame,
        host_secret_frame, write_prefixed_frame,
    };
    let mut id = 1u64;
    if let Ok(mut frame) = host_hello_frame(id, &nonce) {
        let _ = write_prefixed_frame(&mut file, &frame);
        frame.fill(0);
    }
    while let Ok(report) = reports.recv() {
        id = id.saturating_add(1);
        let frame = match &report {
            PipeReport::Ready { monitors } => host_ready_frame(id, *monitors, &nonce).ok(),
            PipeReport::Heartbeat {
                seq,
                guard,
                overlay,
            } => host_heartbeat_frame(id, *seq, *guard, *overlay, &nonce).ok(),
            PipeReport::Secret(secret) => {
                host_secret_frame(id, MethodId::softlock_pin().as_str(), secret, &nonce).ok()
            }
            PipeReport::GuardDown => host_guard_down_frame(id, &nonce).ok(),
        };
        if let Some(mut frame) = frame {
            let _ = write_prefixed_frame(&mut file, &frame);
            frame.fill(0);
        }
        drop(report);
    }
}

#[cfg(test)]
mod quit_guard {
    use softlock_core::{ArmStep, DisarmStep, FailReason, LockSource, LockState, PromptState};

    #[test]
    fn a_guarded_session_still_locks_when_the_service_quits() {
        let locked = LockState::Locked {
            source: LockSource::Tray,
            prompt: PromptState::Idle,
        };
        let arming = LockState::Arming {
            source: LockSource::App,
            step: ArmStep::Guard,
        };
        let failing = LockState::FailingClosed {
            reason: FailReason::HostExited,
        };
        assert!(super::service_quit_locks(&locked));
        assert!(super::service_quit_locks(&arming));
        assert!(super::service_quit_locks(&failing));
    }

    #[test]
    fn an_unguarded_session_does_not_lock_when_the_service_quits() {
        assert!(!super::service_quit_locks(&LockState::Unlocked));
        assert!(!super::service_quit_locks(&LockState::Unlocking {
            step: DisarmStep::DisarmGuard,
        }));
        assert!(!super::service_quit_locks(&LockState::OsLocked {
            after_fail: None,
        }));
        assert!(!super::service_quit_locks(&LockState::Recovering));
    }

    #[cfg(feature = "simulated-physical")]
    #[test]
    fn a_quit_while_unlocking_schedules_once_the_session_is_locked() {
        super::with_quit_hold(|| {
            let mut tried = false;
            let unlocking = LockState::Unlocking {
                step: DisarmStep::DisarmGuard,
            };
            assert!(!super::arm_quit_lock(&mut tried, &unlocking));
            assert!(!tried);
            assert_eq!(
                super::QUIT_OS.load(std::sync::atomic::Ordering::Acquire),
                super::QUIT_IDLE
            );
            let locked = LockState::Locked {
                source: LockSource::Tray,
                prompt: PromptState::Idle,
            };
            let held = super::failclosed::os_lock_held_count();
            assert!(super::arm_quit_lock(&mut tried, &locked));
            assert!(tried);
            assert_eq!(
                super::QUIT_OS.load(std::sync::atomic::Ordering::Acquire),
                super::QUIT_RUNNING
            );
            assert!(!super::arm_quit_lock(&mut tried, &locked));
            super::failclosed::wait_until_os_lock_held(held);
            super::failclosed::block_os_lock_for_test(false);
            super::failclosed::wait_for_os_lock_signal(super::take_quit_os_lock);
        });
    }
}

#[cfg(all(test, feature = "simulated-physical"))]
fn with_quit_hold(body: impl FnOnce()) {
    let _order = QUIT_TEST
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            failclosed::release_os_lock_for_test(|| {
                QUIT_OS.load(Ordering::Acquire) == QUIT_RUNNING
            });
        }
    }
    let _release = Release;
    QUIT_OS.store(QUIT_IDLE, std::sync::atomic::Ordering::Release);
    failclosed::block_os_lock_for_test(true);
    body();
}

#[cfg(all(test, feature = "simulated-physical"))]
mod quit_thread {
    use super::{QUIT_OS, QUIT_RUNNING, schedule_quit_os_lock, take_quit_os_lock, with_quit_hold};
    use crate::real_lock::RealLock;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    #[test]
    fn quit_os_lock_returns_before_the_call_finishes() {
        with_quit_hold(|| {
            let held = super::failclosed::os_lock_held_count();
            let started = Instant::now();
            schedule_quit_os_lock();
            let queued = started.elapsed();
            assert!(
                queued < Duration::from_millis(200),
                "scheduling the quit lock blocked the caller for {} ms",
                queued.as_millis()
            );
            assert_eq!(QUIT_OS.load(Ordering::Acquire), QUIT_RUNNING);
            super::failclosed::wait_until_os_lock_held(held);
            assert_eq!(
                QUIT_OS.load(Ordering::Acquire),
                QUIT_RUNNING,
                "the OS lock finished on the caller instead of staying on the worker"
            );
            super::failclosed::block_os_lock_for_test(false);
            let result = super::failclosed::wait_for_os_lock_signal(take_quit_os_lock);
            assert_eq!(result, RealLock::Unavailable);
        });
    }
}
