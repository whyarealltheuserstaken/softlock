//! The service is the lock authority.
//!
//! A public request cannot unlock. The only unlock is `Verified` carrying the
//! nonce this service drew for the Lock Host, compared with `eq_ct` and then
//! dropped. Host exit, missed heartbeats, and a guard-down report retry the
//! OS lock. When that lock is held the session is `OsLocked` and a replacement
//! host is not armed. When it is not, the guard stays up. Neither path enters
//! Unlocked.

use std::path::{Path, PathBuf};

use std::time::{Duration, Instant};
use zeroize::Zeroize;

use softlock_core::{
    CONFIG_ERRORS_NAME, ConfigOffer, Event, GuardHealth, HeartbeatReport, HostToService,
    LaunchNonce, LockController, LockSource, LockState, MethodId, OverlayHealth, RequestKind,
    ResponseKind, Role, Secret, ServiceToHost, StateName, Verdict, decode_request_frame,
    hash_softlock_pin, verify_softlock_pin,
};

use crate::log::{audit, source_name};
use crate::platform::{ARM_READY_LIMIT, ArmWait, ServicePlatform};

const SILENCE: Duration = Duration::from_millis(750);
const OS_LOCK_RETRY: Duration = Duration::from_secs(1);

/// True when `current` is a fall-back the subscriber has not been shown.
/// Starting `previous` at `None` replays a fall-back that already happened.
pub fn fallback_is_new(
    previous: Option<softlock_core::Fallback>,
    current: Option<softlock_core::Fallback>,
) -> bool {
    current.is_some() && current != previous
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostLoss {
    Exited,
    Silent,
    GuardDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostAction {
    Hold,
    /// `arm` is set only while the session is still locked. After a normal
    /// unlock the replacement host stays idle.
    Respawn {
        arm: bool,
    },
    RetryLock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostFault {
    ForgedNonce,
}

pub struct Authority {
    controller: LockController<ServicePlatform>,
    launch: Option<LaunchNonce>,
    pin_hash: Option<String>,
    dev_pin: bool,
    note_len: Option<usize>,
    host_ready: bool,
    last_beat: Option<Instant>,
    next_os_retry: Option<Instant>,
    /// `Lock` to `HostReady`. Cleared once the host is ready or lost.
    arm_deadline: Option<Instant>,
    /// The hung host is still alive and has to be replaced.
    arm_replace: bool,
    config_dir: Option<PathBuf>,
    offer: ConfigOffer,
}

impl Authority {
    pub fn new(platform: ServicePlatform, dev_pin: bool) -> Self {
        Self {
            controller: LockController::new(platform),
            launch: None,
            pin_hash: None,
            dev_pin,
            note_len: None,
            host_ready: false,
            last_beat: None,
            next_os_retry: None,
            arm_deadline: None,
            arm_replace: false,
            config_dir: None,
            offer: ConfigOffer::new(""),
        }
    }

    pub fn set_config_dir(&mut self, dir: PathBuf) {
        self.config_dir = Some(dir);
    }

    pub fn lock_screen_message(&self) -> Option<String> {
        let message = self.controller.config_message();
        if message.is_empty() {
            None
        } else {
            Some(message.to_string())
        }
    }

    pub fn note_config_on_disk(&mut self, disk: &str) {
        if self.offer.pending().is_none() {
            self.offer = ConfigOffer::new(self.controller.config_toml());
        }
        let locked = !matches!(self.controller.state(), LockState::Unlocked)
            || self.controller.session_os_locked();
        self.offer.note_disk(locked, disk);
    }

    pub fn config_offer(&self) -> Option<&str> {
        self.offer.pending()
    }

    pub fn accept_config_offer(&mut self) -> ResponseKind {
        let Some(toml) = self.offer.pending().map(str::to_string) else {
            return ResponseKind::Conflict;
        };
        let revision = self.controller.config_revision();
        let response = self.handle_public(
            Role::App,
            RequestKind::PutConfig {
                toml,
                base_revision: revision,
            },
        );
        if matches!(response, ResponseKind::Applied) {
            self.offer.accept();
            self.offer.replace_effective(self.controller.config_toml());
        }
        response
    }

    pub fn decline_config_offer(&mut self) {
        self.offer.decline();
    }

    pub fn platform_mut(&mut self) -> &mut ServicePlatform {
        self.controller.platform_mut()
    }

    pub fn arm_wait(&self) -> Option<ArmWait> {
        self.controller.platform().arm_wait()
    }

    pub fn notify(&mut self, event: Event) {
        self.controller.handle(event);
    }

    pub fn session_os_locked(&self) -> bool {
        self.controller.session_os_locked()
    }

    pub fn state(&self) -> &LockState {
        self.controller.state()
    }

    pub fn state_name(&self) -> StateName {
        self.public_state(self.controller.state().name())
    }

    /// The controller enters `Locked` as soon as the arm slot is marked, so a
    /// lost host still fails closed. The hooks are not in until `HostReady`.
    /// Until then the public name stays `Arming`. A platform with no arm slot
    /// has no desktop guard to wait for and reports `Locked` immediately.
    fn public_state(&self, name: StateName) -> StateName {
        if name == StateName::Locked && self.controller.platform().awaits_host() && !self.host_ready
        {
            StateName::Arming
        } else {
            name
        }
    }

    pub fn last_fallback(&self) -> Option<softlock_core::Fallback> {
        self.controller.notifications().back().copied()
    }

    pub fn recovery_due(&self) -> bool {
        self.controller.recovery_due()
    }

    pub fn os_lock_attempts(&self) -> u32 {
        self.controller.os_lock_attempts()
    }

    pub fn os_lock_unavailable(&self) -> bool {
        self.controller.os_lock_unavailable()
    }

    pub fn returned_to_unlocked(&self) -> bool {
        self.controller.returned_to_unlocked()
    }

    pub fn pin_hash(&self) -> Option<&str> {
        self.pin_hash.as_deref()
    }

    pub fn install_pin_hash(&mut self, hash: String) {
        self.pin_hash = Some(hash);
    }

    pub fn note_len(&self) -> Option<usize> {
        self.note_len
    }

    /// Test observation. Production code reads the field directly.
    #[cfg(any(test, feature = "simulated-physical"))]
    pub fn host_ready(&self) -> bool {
        self.host_ready
    }

    /// The replacement Lock Host arms only while this service is still the
    /// thing holding the session. `OsLocked`, and `FailingClosed` after the
    /// OS lock has succeeded, do not get another armed host. After unlock the
    /// replacement waits for the next Lock.
    pub fn session_needs_a_host(&self) -> bool {
        match self.controller.state() {
            LockState::Locked { .. } | LockState::Arming { .. } => true,
            LockState::FailingClosed { .. } => self.os_lock_unavailable(),
            _ => false,
        }
    }

    /// Records a heartbeat time. The desktop service does this when a frame arrives.
    pub fn record_beat(&mut self, at: Instant) {
        self.last_beat = Some(at);
        self.host_ready = true;
    }

    /// Draws the nonce the Lock Host must present. The previous one is dropped.
    pub fn issue_launch_nonce(&mut self) -> Option<LaunchNonce> {
        let nonce = LaunchNonce::generate().ok()?;
        self.launch = Some(LaunchNonce::from_bytes(*nonce.as_bytes()));
        self.host_ready = false;
        self.last_beat = None;
        Some(nonce)
    }

    pub fn handle_public(&mut self, role: Role, kind: RequestKind) -> ResponseKind {
        match kind {
            RequestKind::SetPin { pin } => self.set_pin(role, &pin),
            RequestKind::ClearPin => self.clear_pin(role),
            RequestKind::Lock { source, note } => {
                self.remember_lock(source, note.as_deref());
                drop(note);
                self.controller.handle(Event::Public {
                    role,
                    kind: RequestKind::Lock { source, note: None },
                });
                let response = self.response();
                if matches!(response, ResponseKind::Accepted) {
                    self.note_lock_armed();
                }
                response
            }
            other => {
                self.controller.handle(Event::Public { role, kind: other });
                let response = self.response();
                self.record_config_result(&response);
                response
            }
        }
    }
}

fn write_errors(path: &Path, errors: &[String]) -> std::io::Result<()> {
    let body = errors.join("\n");
    #[cfg(windows)]
    {
        crate::config_file::write_bytes(path, body.as_bytes())
    }
    #[cfg(not(windows))]
    {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Ok(meta) = std::fs::symlink_metadata(path)
            && meta.file_type().is_symlink()
        {
            return Err(std::io::Error::other("reparse"));
        }
        std::fs::write(path, body)
    }
}

impl Authority {
    fn record_config_result(&self, response: &ResponseKind) {
        let Some(dir) = &self.config_dir else {
            return;
        };
        let path = dir.join(CONFIG_ERRORS_NAME);
        match response {
            ResponseKind::Invalid { errors } => {
                let _ = write_errors(&path, errors);
            }
            ResponseKind::Applied => {
                #[cfg(windows)]
                {
                    let _ = crate::config_file::remove_file(&path);
                }
                #[cfg(not(windows))]
                {
                    if let Ok(meta) = std::fs::symlink_metadata(&path)
                        && meta.file_type().is_symlink()
                    {
                        return;
                    }
                    let _ = std::fs::remove_file(&path);
                }
            }
            _ => {}
        }
    }

    pub fn handle_public_frame(&mut self, role: Role, bytes: &[u8]) -> ResponseKind {
        match decode_request_frame(bytes) {
            Ok(decoded) => self.handle_public(role, decoded.kind),
            Err(_) => {
                audit("rejected", &[("reason", "malformed")]);
                ResponseKind::Unauthorized
            }
        }
    }

    pub fn accept_host(
        &mut self,
        message: HostToService,
    ) -> Result<Option<ServiceToHost>, HostFault> {
        if !self.matches_launch(message.nonce()) {
            audit("rejected", &[("reason", "nonce")]);
            return Err(HostFault::ForgedNonce);
        }
        match message {
            HostToService::HostHello { .. } => Ok(None),
            HostToService::HostReady { monitors, .. } => {
                self.host_ready = true;
                self.arm_deadline = None;
                self.arm_replace = false;
                self.last_beat = Some(Instant::now());
                self.controller.platform_mut().set_report(
                    GuardHealth {
                        hook_alive: true,
                        last_physical_input_ms: None,
                    },
                    OverlayHealth {
                        covered_monitors: monitors.max(1),
                        capture_excluded: true,
                        topmost: true,
                    },
                );
                Ok(None)
            }
            HostToService::Heartbeat {
                seq,
                guard,
                overlay,
                ..
            } => {
                self.last_beat = Some(Instant::now());
                self.host_ready = guard.hook_alive;
                self.controller.platform_mut().set_report(guard, overlay);
                self.controller.handle(Event::Heartbeat(HeartbeatReport {
                    seq,
                    guard,
                    overlay,
                }));
                Ok(None)
            }
            HostToService::GuardDown { .. } => {
                self.note_host_lost(HostLoss::GuardDown);
                Ok(None)
            }
            HostToService::HardLock { .. } => {
                self.controller.handle(Event::Public {
                    role: Role::LockHost,
                    kind: RequestKind::HardLock,
                });
                Ok(None)
            }
            HostToService::SubmitSecret { method, secret, .. } => {
                Ok(Some(self.decide_secret(&method, secret)))
            }
        }
    }

    pub fn note_os_lock_held(&mut self) {
        self.controller.note_os_lock_held();
    }

    /// A new subscriber must start from the fall-back already recorded, or the
    /// stream treats that old value as a new event.
    pub fn fallback_stream_start(&self) -> Option<softlock_core::Fallback> {
        self.last_fallback()
    }

    pub fn note_host_lost(&mut self, loss: HostLoss) {
        self.host_ready = false;
        self.last_beat = None;
        self.arm_deadline = None;
        if let Some(arm) = self.arm_wait() {
            arm.failed();
        }
        #[cfg(windows)]
        if matches!(loss, HostLoss::Exited | HostLoss::Silent) {
            crate::windows::mark_host_unavailable(loss);
        }
        let (event, kind) = match loss {
            HostLoss::Exited => (Event::HostExited, "exited"),
            HostLoss::Silent => (Event::HostSilent, "silent"),
            HostLoss::GuardDown => (Event::GuardDown, "guard-down"),
        };
        audit("host-lost", &[("kind", kind)]);
        if matches!(
            self.controller.state(),
            LockState::Locked { .. } | LockState::Arming { .. } | LockState::FailingClosed { .. }
        ) {
            self.controller.handle(event);
        }
    }

    /// The arm slot is still waiting and `HostReady` did not arrive. Fails
    /// closed, and asks the supervisor to replace a host that is still alive.
    pub fn expire_arm_wait(&mut self) {
        if self.host_ready
            || !matches!(
                self.controller.state(),
                LockState::Locked { .. } | LockState::Arming { .. }
            )
        {
            return;
        }
        self.arm_replace = true;
        self.note_host_lost(HostLoss::Silent);
    }

    /// Respawns when the host is gone. Retries the OS lock while it is down.
    /// Neither result is Unlocked.
    pub fn poll_host(&mut self, now: Instant, process_alive: bool) -> HostAction {
        let stale = self
            .last_beat
            .is_some_and(|last| now.saturating_duration_since(last) >= SILENCE);
        // Death while `Locked` fails closed even when `HostReady` never
        // arrived. The hooks are not in, and a respawn loop must not leave
        // the public name on `Arming`.
        if !process_alive && matches!(self.controller.state(), LockState::Locked { .. }) {
            self.note_host_lost(HostLoss::Exited);
        } else if self.host_ready && (!process_alive || stale) {
            self.note_host_lost(if process_alive {
                HostLoss::Silent
            } else {
                HostLoss::Exited
            });
        }
        if !self.host_ready
            && self.arm_deadline.is_some_and(|due| now >= due)
            && matches!(
                self.controller.state(),
                LockState::Locked { .. }
                    | LockState::Arming { .. }
                    | LockState::FailingClosed { .. }
            )
        {
            self.arm_replace = true;
            self.note_host_lost(HostLoss::Silent);
        }
        // A live host waits out the OS-lock chain. A dead host, and a host that
        // missed HostReady, still fail closed.
        #[cfg(windows)]
        if process_alive && !self.arm_replace && crate::windows::chain_running() {
            return HostAction::Hold;
        }
        // A dead Lock Host is replaced. While the session is still locked this
        // is the emergency recovery. After a normal unlock the replacement
        // stays idle until the next Lock.
        if !process_alive
            || self.arm_replace
            || (self.controller.recovery_due() && !self.host_ready)
        {
            return self.respawn(now);
        }
        if self.os_retry_due(now) {
            self.retry_os_lock();
            return HostAction::RetryLock;
        }
        HostAction::Hold
    }

    fn note_lock_armed(&mut self) {
        if self.host_ready || !self.controller.platform().awaits_host() {
            return;
        }
        if !matches!(
            self.controller.state(),
            LockState::Locked { .. } | LockState::Arming { .. }
        ) {
            return;
        }
        self.arm_deadline = Some(Instant::now() + ARM_READY_LIMIT);
    }

    fn respawn(&mut self, now: Instant) -> HostAction {
        self.arm_replace = false;
        if self.controller.recovery_due() {
            self.controller.acknowledge_recovery();
        }
        let arm = self.session_needs_a_host();
        if arm && self.controller.platform().awaits_host() {
            self.arm_deadline = Some(now + ARM_READY_LIMIT);
        }
        HostAction::Respawn { arm }
    }

    fn os_retry_due(&mut self, now: Instant) -> bool {
        if !self.controller.os_lock_unavailable()
            || !matches!(self.controller.state(), LockState::FailingClosed { .. })
        {
            return false;
        }
        match self.next_os_retry {
            None => {
                self.next_os_retry = Some(now + OS_LOCK_RETRY);
                false
            }
            Some(due) if now >= due => {
                self.next_os_retry = Some(now + OS_LOCK_RETRY);
                true
            }
            Some(_) => false,
        }
    }

    fn retry_os_lock(&mut self) {
        let ready = self.host_ready;
        self.controller.handle(Event::HostSilent);
        if ready {
            self.host_ready = true;
            self.controller.acknowledge_recovery();
        }
    }

    fn decide_secret(&mut self, method: &str, secret: Secret) -> ServiceToHost {
        let method_id = MethodId::new(method);
        let backoff = self.controller.backoff_delay();
        if !backoff.is_zero() {
            self.controller.handle(Event::SubmitSecret {
                method: method_id,
                secret,
            });
            return ServiceToHost::PromptResult {
                accepted: false,
                backoff_ms: u64::try_from(backoff.as_millis()).unwrap_or(u64::MAX),
            };
        }
        let matched = self
            .pin_hash
            .as_deref()
            .is_some_and(|hash| verify_softlock_pin(&secret, hash) == Verdict::Verified);
        if matched && self.grant_allowed() {
            drop(secret);
            let Some(stored) = self.launch.take() else {
                return ServiceToHost::PromptResult {
                    accepted: false,
                    backoff_ms: 0,
                };
            };
            let mut bytes = *stored.as_bytes();
            // AdoptLaunchNonce followed by Verified always matches. Both
            // events carry these same bytes, the nonce just taken from
            // `self.launch`. That pair is not the gate. The gate already
            // ran: `matches_launch` compared every private message with
            // `eq_ct`, and Argon2 accepted the PIN before this nonce was
            // adopted.
            self.controller.handle(Event::AdoptLaunchNonce {
                nonce: LaunchNonce::from_bytes(bytes),
            });
            self.controller.handle(Event::Verified {
                nonce: LaunchNonce::from_bytes(bytes),
            });
            bytes.zeroize();
            audit("unlock", &[("method", "softlock_pin")]);
            // The host is still up until it reads Disarm and removes its hooks.
            // Clearing the launch flag here must not look like a lost host, or
            // the supervisor kills the process before that Disarm is applied.
            self.controller.acknowledge_recovery();
            self.host_ready = false;
            return ServiceToHost::Disarm;
        }
        if matched {
            drop(secret);
            return ServiceToHost::PromptResult {
                accepted: false,
                backoff_ms: 0,
            };
        }
        self.controller.handle(Event::SubmitSecret {
            method: method_id,
            secret,
        });
        let backoff = self.controller.backoff_delay();
        ServiceToHost::PromptResult {
            accepted: false,
            backoff_ms: u64::try_from(backoff.as_millis()).unwrap_or(u64::MAX),
        }
    }

    fn grant_allowed(&self) -> bool {
        matches!(self.controller.state(), LockState::Locked { .. })
            || (self.controller.os_lock_unavailable()
                && matches!(self.controller.state(), LockState::FailingClosed { .. }))
    }

    fn matches_launch(&self, presented: &LaunchNonce) -> bool {
        self.launch
            .as_ref()
            .is_some_and(|expected| expected.eq_ct(presented))
    }

    fn set_pin(&mut self, role: Role, pin: &Secret) -> ResponseKind {
        if role != Role::App {
            audit("pin-set", &[("result", "unauthorized")]);
            return ResponseKind::Unauthorized;
        }
        if !self.dev_pin {
            audit("pin-set", &[("result", "needs-confirmation")]);
            return ResponseKind::NeedsConfirmation {
                fields: vec!["unlock".to_string()],
            };
        }
        if !matches!(self.controller.state(), LockState::Unlocked) {
            audit("pin-set", &[("result", "locked")]);
            return ResponseKind::Locked;
        }
        match hash_softlock_pin(pin) {
            Ok(hash) => {
                if let Err(error) = crate::secrets::PinStore::in_dir(&pin_directory()).save(&hash) {
                    let reason = if error.to_string().contains("LocalSystem") {
                        "localsystem"
                    } else {
                        "io"
                    };
                    audit("pin-set", &[("result", "rejected"), ("reason", reason)]);
                    return ResponseKind::Unsupported;
                }
                self.pin_hash = Some(hash);
                audit("pin-set", &[("result", "stored")]);
                ResponseKind::Ok
            }
            Err(_) => {
                audit("pin-set", &[("result", "rejected")]);
                ResponseKind::Unsupported
            }
        }
    }

    fn clear_pin(&mut self, role: Role) -> ResponseKind {
        if role != Role::App {
            return ResponseKind::Unauthorized;
        }
        if !self.dev_pin {
            return ResponseKind::NeedsConfirmation {
                fields: vec!["unlock".to_string()],
            };
        }
        if !matches!(self.controller.state(), LockState::Unlocked) {
            return ResponseKind::Locked;
        }
        match crate::secrets::PinStore::in_dir(&pin_directory()).remove() {
            Ok(()) => {
                self.pin_hash = None;
                audit("pin-cleared", &[("result", "stored")]);
                ResponseKind::Ok
            }
            Err(error) => {
                let reason = if error.to_string().contains("LocalSystem") {
                    "localsystem"
                } else {
                    "io"
                };
                audit("pin-cleared", &[("result", "rejected"), ("reason", reason)]);
                ResponseKind::Unsupported
            }
        }
    }

    fn remember_lock(&mut self, source: LockSource, note: Option<&str>) {
        let noted = if note.is_some() { "yes" } else { "no" };
        let len = note.map(str::len).unwrap_or(0);
        self.note_len = note.map(str::len);
        let len_text = len.to_string();
        audit(
            "lock",
            &[
                ("source", source_name(source)),
                ("noted", noted),
                ("noted_len", &len_text),
            ],
        );
    }

    fn response(&self) -> ResponseKind {
        let response = self
            .controller
            .last_response()
            .cloned()
            .unwrap_or(ResponseKind::Unsupported);
        match response {
            ResponseKind::Status {
                state,
                lock_source,
                guard,
                overlay,
                last_fallback,
            } => ResponseKind::Status {
                state: self.public_state(state),
                lock_source,
                guard,
                overlay,
                last_fallback,
            },
            ResponseKind::Hello {
                service_version,
                state,
                capabilities,
            } => ResponseKind::Hello {
                service_version,
                state: self.public_state(state),
                capabilities,
            },
            other => other,
        }
    }
}

/// A client outside the active console session is not a Lock Client.
pub fn admit_client(client_session: u32, console_session: u32) -> bool {
    client_session != 0 && client_session != u32::MAX && client_session == console_session
}

pub fn image_is_inside(image: &Path, install_dir: &Path) -> bool {
    let image: Vec<_> = image.components().collect();
    let install: Vec<_> = install_dir.components().collect();
    image.len() > install.len() && image[..install.len()] == install[..]
}

pub fn role_for_image(image: &Path, install_dir: &Path) -> Role {
    if image_is_inside(image, install_dir) {
        Role::App
    } else {
        Role::LockClient
    }
}

pub fn dev_pin_allowed() -> bool {
    cfg!(debug_assertions)
}

pub fn pin_directory() -> PathBuf {
    if let Some(dir) = test_path("SOFTLOCK_PIN_DIR") {
        return dir;
    }
    service_data_directory()
}

/// The LocalSystem profile directory. A standard user cannot create it.
/// Ignores the test override so service start verifies this directory before
/// any log write.
pub fn service_data_directory() -> PathBuf {
    default_pin_directory()
}

#[cfg(windows)]
fn default_pin_directory() -> PathBuf {
    system_directory()
        .join("config")
        .join("systemprofile")
        .join("AppData")
        .join("Local")
        .join(softlock_core::SERVICE_NAME)
}

#[cfg(windows)]
fn system_directory() -> PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buffer = [0u16; 260];
    let length = unsafe { GetSystemDirectoryW(Some(&mut buffer[..])) } as usize;
    if length == 0 || length >= buffer.len() {
        PathBuf::from(r"C:\Windows\System32")
    } else {
        PathBuf::from(OsString::from_wide(&buffer[..length]))
    }
}

#[cfg(not(windows))]
fn default_pin_directory() -> PathBuf {
    PathBuf::from("/tmp").join(softlock_core::SERVICE_NAME)
}

pub fn log_path() -> PathBuf {
    if let Some(path) = test_path("SOFTLOCK_LOG") {
        return path;
    }
    pin_directory().join("service.log")
}

pub fn pipe_path() -> String {
    if let Some(path) = test_path("SOFTLOCK_PIPE") {
        return path.to_string_lossy().to_string();
    }
    softlock_core::pipe_name()
}

pub(crate) fn test_path(name: &str) -> Option<PathBuf> {
    #[cfg(feature = "simulated-physical")]
    {
        if let Ok(path) = std::env::var(name) {
            return Some(PathBuf::from(path));
        }
    }
    #[cfg(all(
        windows,
        feature = "live-fail-closed",
        not(feature = "simulated-physical")
    ))]
    if crate::windows::live_hooks_allowed()
        && let Ok(path) = std::env::var(name)
    {
        return Some(PathBuf::from(path));
    }
    let _ = name;
    None
}

pub fn stub_os_lock() -> bool {
    #[cfg(feature = "simulated-physical")]
    {
        // `1` and `fail` make the host report failure. `succeed` makes the
        // host report a confirmed lock. None of them may call LockWorkStation
        // or disconnect the console from this process.
        matches!(
            std::env::var("SOFTLOCK_STUB_OS_LOCK").ok().as_deref(),
            Some("1" | "fail" | "succeed")
        )
    }
    #[cfg(not(feature = "simulated-physical"))]
    {
        false
    }
}

#[cfg(test)]
mod host_ready_tests {
    use softlock_core::{
        HostToService, LaunchNonce, LockSource, LockState, RequestKind, ResponseKind, Role,
        StateName,
    };

    use std::time::Instant;

    use super::{Authority, HostAction, HostLoss};
    use crate::platform::{ARM_READY_LIMIT, ArmWait, OsLock, ServicePlatform};

    fn lock_hotkey(authority: &mut Authority) -> ResponseKind {
        authority.handle_public(
            Role::App,
            RequestKind::Lock {
                source: LockSource::Hotkey,
                note: None,
            },
        )
    }

    fn status_state(authority: &mut Authority) -> StateName {
        match authority.handle_public(Role::App, RequestKind::GetStatus) {
            ResponseKind::Status { state, .. } => state,
            other => panic!("GetStatus returned {other:?}"),
        }
    }

    fn hello_state(authority: &mut Authority) -> StateName {
        match authority.handle_public(
            Role::App,
            RequestKind::Hello {
                client_version: "0".to_string(),
            },
        ) {
            ResponseKind::Hello { state, .. } => state,
            other => panic!("Hello returned {other:?}"),
        }
    }

    #[test]
    fn forged_host_ready_leaves_the_host_not_ready() {
        let mut authority = Authority::new(ServicePlatform::new(OsLock::Fails), true);
        let nonce = authority.issue_launch_nonce().expect("nonce");
        let response = authority.handle_public(
            Role::App,
            RequestKind::Lock {
                source: LockSource::App,
                note: None,
            },
        );
        assert_eq!(response, ResponseKind::Accepted);
        assert!(
            !authority.host_ready(),
            "the host was ready before HostReady"
        );
        let mut bytes = *nonce.as_bytes();
        bytes[0] ^= 0xff;
        let forged = LaunchNonce::from_bytes(bytes);
        let error = authority
            .accept_host(HostToService::HostReady {
                monitors: 2,
                nonce: forged,
            })
            .expect_err("forged HostReady was accepted");
        assert_eq!(error, super::HostFault::ForgedNonce);
        assert!(!authority.host_ready(), "a forged HostReady armed the host");
        assert!(matches!(authority.state(), LockState::Locked { .. }));
        authority
            .accept_host(HostToService::HostReady { monitors: 2, nonce })
            .expect("the real HostReady still arms");
        assert!(authority.host_ready());
    }

    /// A desktop lock marks the arm slot and the controller enters `Locked`
    /// so a lost host still fails closed. Callers are told `Arming` until
    /// `HostReady`, which the Lock Host sends only after its hooks are in.
    #[test]
    fn locked_is_not_reported_until_the_host_hooks_are_in() {
        let mut bare = Authority::new(ServicePlatform::new(OsLock::Fails), true);
        let _ = bare.issue_launch_nonce().expect("nonce");
        assert_eq!(lock_hotkey(&mut bare), ResponseKind::Accepted);
        assert_eq!(bare.state_name(), StateName::Locked);
        assert_eq!(status_state(&mut bare), StateName::Locked);

        let mut platform = ServicePlatform::new(OsLock::Fails);
        platform.set_arm_wait(ArmWait::new());
        let mut authority = Authority::new(platform, true);
        let nonce = authority.issue_launch_nonce().expect("nonce");
        assert_eq!(lock_hotkey(&mut authority), ResponseKind::Accepted);
        assert!(
            matches!(authority.state(), LockState::Locked { .. }),
            "the controller left Locked before the host was ready"
        );
        assert!(authority.session_needs_a_host());
        assert_eq!(authority.state_name(), StateName::Arming);
        assert_eq!(status_state(&mut authority), StateName::Arming);
        assert_eq!(hello_state(&mut authority), StateName::Arming);
        authority
            .accept_host(HostToService::HostReady { monitors: 1, nonce })
            .expect("host ready");
        assert!(authority.host_ready());
        assert_eq!(authority.state_name(), StateName::Locked);
        assert_eq!(status_state(&mut authority), StateName::Locked);
        assert_eq!(hello_state(&mut authority), StateName::Locked);
    }

    #[test]
    fn host_loss_before_ready_still_fails_closed() {
        let mut platform = ServicePlatform::new(OsLock::Fails);
        platform.set_arm_wait(ArmWait::new());
        let mut authority = Authority::new(platform, true);
        let _ = authority.issue_launch_nonce().expect("nonce");
        assert_eq!(lock_hotkey(&mut authority), ResponseKind::Accepted);
        assert_eq!(authority.state_name(), StateName::Arming);
        authority.note_host_lost(HostLoss::GuardDown);
        assert!(matches!(authority.state(), LockState::FailingClosed { .. }));
        assert_ne!(authority.state_name(), StateName::Unlocked);
        assert_ne!(authority.state_name(), StateName::Locked);
    }

    /// The host process is gone before `HostReady`. `poll_host` has to fail
    /// closed on that death, not only after the hooks were reported in.
    #[test]
    fn a_dead_host_before_ready_fails_closed() {
        let mut platform = ServicePlatform::new(OsLock::Fails);
        let wait = ArmWait::new();
        platform.set_arm_wait(wait.clone());
        let mut authority = Authority::new(platform, true);
        let _ = authority.issue_launch_nonce().expect("nonce");
        assert_eq!(lock_hotkey(&mut authority), ResponseKind::Accepted);
        assert_eq!(authority.state_name(), StateName::Arming);
        assert!(wait.is_waiting());
        let action = authority.poll_host(Instant::now(), false);
        assert!(
            matches!(authority.state(), LockState::FailingClosed { .. }),
            "a dead host before HostReady stayed {:?}",
            authority.state()
        );
        assert_ne!(authority.state_name(), StateName::Arming);
        assert_ne!(authority.state_name(), StateName::Unlocked);
        assert!(
            authority.os_lock_attempts() >= 1,
            "the OS lock was not requested"
        );
        assert!(!wait.is_waiting(), "the arm slot stayed waiting");
        assert_eq!(action, HostAction::Respawn { arm: true });
    }

    /// The host stays up and never sends `HostReady`. The arm deadline has to
    /// fail closed instead of leaving the public name on `Arming`.
    #[test]
    fn a_host_that_never_readies_fails_closed() {
        let mut platform = ServicePlatform::new(OsLock::Fails);
        let wait = ArmWait::new();
        platform.set_arm_wait(wait.clone());
        let mut authority = Authority::new(platform, true);
        let _ = authority.issue_launch_nonce().expect("nonce");
        assert_eq!(lock_hotkey(&mut authority), ResponseKind::Accepted);
        let started = Instant::now();
        assert_eq!(authority.poll_host(started, true), HostAction::Hold);
        assert_eq!(authority.state_name(), StateName::Arming);
        assert!(wait.is_waiting());
        let action = authority.poll_host(started + ARM_READY_LIMIT, true);
        assert!(
            matches!(
                authority.state(),
                LockState::FailingClosed {
                    reason: softlock_core::FailReason::HeartbeatMissed
                }
            ),
            "a missed HostReady stayed {:?}",
            authority.state()
        );
        assert_ne!(authority.state_name(), StateName::Arming);
        assert_ne!(authority.state_name(), StateName::Unlocked);
        assert!(
            authority.os_lock_attempts() >= 1,
            "the OS lock was not requested"
        );
        assert!(!wait.is_waiting(), "the arm slot stayed waiting");
        assert_eq!(action, HostAction::Respawn { arm: true });
    }
}
