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
    Event, GuardHealth, HeartbeatReport, HostToService, LaunchNonce, LockController, LockSource,
    LockState, MethodId, OverlayHealth, RequestKind, ResponseKind, Role, Secret, ServiceToHost,
    StateName, Verdict, decode_request_frame, hash_softlock_pin, verify_softlock_pin,
};

use crate::log::{audit, source_name};
use crate::platform::{ArmWait, ServicePlatform};

const SILENCE: Duration = Duration::from_millis(750);
const OS_LOCK_RETRY: Duration = Duration::from_secs(1);

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
        }
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

    pub fn state(&self) -> &LockState {
        self.controller.state()
    }

    pub fn state_name(&self) -> StateName {
        self.controller.state().name()
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
                self.response()
            }
            other => {
                self.controller.handle(Event::Public { role, kind: other });
                self.response()
            }
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
            HostToService::SubmitSecret { method, secret, .. } => {
                Ok(Some(self.decide_secret(&method, secret)))
            }
        }
    }

    pub fn note_host_lost(&mut self, loss: HostLoss) {
        self.host_ready = false;
        self.last_beat = None;
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

    /// Respawns when the host is gone. Retries the OS lock while it is down.
    /// Neither result is Unlocked.
    pub fn poll_host(&mut self, now: Instant, process_alive: bool) -> HostAction {
        let stale = self
            .last_beat
            .is_some_and(|last| now.saturating_duration_since(last) >= SILENCE);
        if self.host_ready && (!process_alive || stale) {
            self.note_host_lost(if process_alive {
                HostLoss::Silent
            } else {
                HostLoss::Exited
            });
        }
        // A dead Lock Host is replaced. While the session is still locked this
        // is the emergency recovery. After a normal unlock the replacement
        // stays idle until the next Lock.
        if !process_alive {
            if self.controller.recovery_due() {
                self.controller.acknowledge_recovery();
            }
            return HostAction::Respawn {
                arm: self.session_needs_a_host(),
            };
        }
        if self.controller.recovery_due() && !self.host_ready {
            self.controller.acknowledge_recovery();
            return HostAction::Respawn {
                arm: self.session_needs_a_host(),
            };
        }
        if self.os_retry_due(now) {
            self.retry_os_lock();
            return HostAction::RetryLock;
        }
        HostAction::Hold
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
                    let code = error.raw_os_error().unwrap_or(0).to_string();
                    audit("pin-set", &[("result", "rejected"), ("os", &code)]);
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
        self.pin_hash = None;
        audit("pin-cleared", &[("result", "stored")]);
        ResponseKind::Ok
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
        self.controller
            .last_response()
            .cloned()
            .unwrap_or(ResponseKind::Unsupported)
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
    #[cfg(feature = "simulated-physical")]
    if let Ok(dir) = std::env::var("SOFTLOCK_PIN_DIR") {
        return PathBuf::from(dir);
    }
    default_pin_directory()
}

#[cfg(windows)]
fn default_pin_directory() -> PathBuf {
    let base = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    PathBuf::from(base).join(softlock_core::SERVICE_NAME)
}

#[cfg(not(windows))]
fn default_pin_directory() -> PathBuf {
    PathBuf::from("/tmp").join(softlock_core::SERVICE_NAME)
}

pub fn log_path() -> PathBuf {
    #[cfg(feature = "simulated-physical")]
    if let Ok(path) = std::env::var("SOFTLOCK_LOG") {
        return PathBuf::from(path);
    }
    pin_directory().join("service.log")
}

pub fn pipe_path() -> String {
    #[cfg(feature = "simulated-physical")]
    if let Ok(name) = std::env::var("SOFTLOCK_PIPE") {
        return name;
    }
    softlock_core::pipe_name()
}

pub fn stub_os_lock() -> bool {
    #[cfg(feature = "simulated-physical")]
    {
        std::env::var("SOFTLOCK_STUB_OS_LOCK").ok().as_deref() == Some("1")
    }
    #[cfg(not(feature = "simulated-physical"))]
    {
        false
    }
}
