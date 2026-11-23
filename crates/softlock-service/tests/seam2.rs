//! Seam 2. The service is the lock authority. No desktop is required.

use std::time::{Duration, Instant};

use proptest::prelude::*;
use softlock_core::{
    Event, GuardHealth, HostToService, LaunchNonce, LockSource, LockState, OverlayHealth,
    RequestKind, ResponseKind, Role, Secret, SessionEvent, host_hello_frame, host_secret_frame,
};
use softlock_service::{
    Authority, Capture, HostAction, HostFault, HostLoss, OsLock, ServicePlatform, admit_client,
    role_for_image,
};
#[cfg(not(windows))]
use softlock_service::{PinStore, pin_directory};

const PIN: &str = "z9-pin-4e7c1b";
const NOTE: &str = "Quarterly Report - Notepad";

/// Unix pin saves share `/tmp/<service>/pin.phc`. Hold this across a save and
/// the later assertion so a parallel `SetPin` cannot recreate the file.
#[cfg(not(windows))]
fn pin_disk_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn authority() -> Authority {
    Authority::new(ServicePlatform::new(OsLock::Fails), true)
}

fn fresh() -> Authority {
    authority()
}

fn lock(authority: &mut Authority, source: LockSource) -> LaunchNonce {
    let nonce = authority.issue_launch_nonce().expect("nonce");
    let response = authority.handle_public(Role::App, RequestKind::Lock { source, note: None });
    assert_eq!(response, ResponseKind::Accepted);
    assert!(matches!(authority.state(), LockState::Locked { .. }));
    nonce
}

#[test]
fn public_requests_never_unlock_from_locked() {
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig::with_cases(64));
    let role = prop_oneof![Just(Role::App), Just(Role::LockClient)];
    runner
        .run(
            &(proptest::collection::vec((role, 0u8..6), 1..12)),
            |steps| {
                let mut authority = fresh();
                lock(&mut authority, LockSource::Hotkey);
                for (role, kind) in steps {
                    let _ = authority.handle_public(role, public_kind(kind));
                    prop_assert!(
                        !matches!(authority.state(), LockState::Unlocked),
                        "a public request ended Unlocked"
                    );
                }
                let private = host_hello_frame(9, &LaunchNonce::from_bytes([7; 32])).unwrap();
                let response = authority.handle_public_frame(Role::App, &private);
                prop_assert_eq!(response, ResponseKind::Unauthorized);
                prop_assert!(!matches!(authority.state(), LockState::Unlocked));
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn private_message_on_the_public_pipe_is_rejected() {
    let mut authority = fresh();
    let nonce = lock(&mut authority, LockSource::Tray);
    let private = host_hello_frame(3, &nonce).unwrap();
    let response = authority.handle_public_frame(Role::LockClient, &private);
    assert_eq!(response, ResponseKind::Unauthorized);
    assert!(matches!(authority.state(), LockState::Locked { .. }));
    assert!(!authority.returned_to_unlocked());
}

#[test]
fn forged_host_ready_does_not_arm_the_host() {
    let mut authority = fresh();
    let nonce = lock(&mut authority, LockSource::App);
    let mut bytes = *nonce.as_bytes();
    bytes[0] ^= 0xff;
    let forged = LaunchNonce::from_bytes(bytes);
    let error = authority
        .accept_host(HostToService::HostReady {
            monitors: 2,
            nonce: forged,
        })
        .expect_err("forged HostReady was accepted");
    assert_eq!(error, HostFault::ForgedNonce);
    assert!(matches!(authority.state(), LockState::Locked { .. }));
    authority
        .accept_host(HostToService::HostReady { monitors: 2, nonce })
        .expect("the real HostReady still arms");
}

#[test]
fn forged_launch_nonce_is_rejected() {
    let mut authority = fresh();
    let nonce = lock(&mut authority, LockSource::App);
    let mut bytes = *nonce.as_bytes();
    bytes[31] ^= 0xff;
    let forged = LaunchNonce::from_bytes(bytes);
    let error = authority
        .accept_host(HostToService::HostHello { nonce: forged })
        .expect_err("forged nonce was accepted");
    assert_eq!(error, HostFault::ForgedNonce);
    assert!(matches!(authority.state(), LockState::Locked { .. }));
    authority
        .accept_host(HostToService::HostHello { nonce })
        .expect("the real nonce still works");
}

#[test]
fn verified_nonce_is_single_use() {
    let mut authority = fresh();
    authority.install_pin_hash(hash_pin(PIN));
    let nonce = lock(&mut authority, LockSource::LocalApi);
    let first = authority
        .accept_host(secret_message(&nonce, PIN))
        .expect("first pin");
    assert!(matches!(first, Some(softlock_core::ServiceToHost::Disarm)));
    assert!(matches!(authority.state(), LockState::Unlocked));
    lock(&mut authority, LockSource::LocalApi);
    let replay = authority.accept_host(secret_message(&nonce, PIN));
    assert!(matches!(replay, Err(HostFault::ForgedNonce)));
    assert!(matches!(authority.state(), LockState::Locked { .. }));
}

#[test]
fn a_dead_host_after_unlock_is_replaced_idle() {
    let mut authority = fresh();
    authority.install_pin_hash(hash_pin(PIN));
    let nonce = lock(&mut authority, LockSource::LocalApi);
    authority
        .accept_host(secret_message(&nonce, PIN))
        .expect("unlock");
    assert!(matches!(authority.state(), LockState::Unlocked));
    assert_eq!(
        authority.poll_host(Instant::now(), false),
        HostAction::Respawn { arm: false }
    );
}

#[test]
fn non_console_client_is_rejected() {
    assert!(!admit_client(0, 1));
    assert!(!admit_client(u32::MAX, 4));
    assert!(!admit_client(2, 4));
    assert!(admit_client(4, 4));
    assert_eq!(
        role_for_image(
            std::path::Path::new("/opt/app/settings"),
            std::path::Path::new("/opt/app")
        ),
        Role::App
    );
    assert_eq!(
        role_for_image(
            std::path::Path::new("/tmp/other"),
            std::path::Path::new("/opt/app")
        ),
        Role::LockClient
    );
}

#[test]
fn get_status_snapshots_source_and_health() {
    let mut rows = Vec::new();
    for source in [
        LockSource::Hotkey,
        LockSource::Tray,
        LockSource::App,
        LockSource::LocalApi,
    ] {
        let mut authority = fresh();
        let nonce = lock(&mut authority, source);
        authority
            .accept_host(HostToService::Heartbeat {
                seq: 1,
                guard: GuardHealth {
                    hook_alive: true,
                    last_physical_input_ms: Some(12),
                },
                overlay: OverlayHealth {
                    covered_monitors: 2,
                    capture_excluded: true,
                    topmost: true,
                },
                nonce,
            })
            .unwrap();
        let status = authority.handle_public(Role::LockClient, RequestKind::GetStatus);
        rows.push(status);
    }
    insta::assert_json_snapshot!(rows);
}

#[test]
fn correct_pin_consumes_the_service_nonce() {
    #[cfg(not(windows))]
    let _pin_disk = pin_disk_lock();
    let capture = Capture::new();
    tracing::subscriber::with_default(capture.clone(), || {
        let mut authority = Authority::new(ServicePlatform::new(OsLock::Fails), true);
        let stored = authority.handle_public(
            Role::App,
            RequestKind::SetPin {
                pin: Secret::new(PIN),
            },
        );
        #[cfg(windows)]
        {
            // A process that is not LocalSystem must not write the hash.
            // The nonce check below uses the hash in memory.
            assert_eq!(
                stored,
                ResponseKind::Unsupported,
                "pin was not refused: {}",
                capture.text()
            );
            assert!(
                capture.text().contains("reason=localsystem"),
                "not the LocalSystem refusal: {}",
                capture.text()
            );
            let hash = softlock_core::hash_softlock_pin(&Secret::new(PIN)).expect("hash");
            authority.install_pin_hash(hash);
        }
        #[cfg(not(windows))]
        {
            assert_eq!(
                stored,
                ResponseKind::Ok,
                "pin was not stored: {}",
                capture.text()
            );
        }
        let hash = authority.pin_hash().expect("hash").to_string();
        assert!(hash.starts_with("$argon2id$"));
        let nonce = lock_with_note(&mut authority);
        let reply = authority
            .accept_host(secret_message(&nonce, PIN))
            .expect("pin");
        assert!(matches!(reply, Some(softlock_core::ServiceToHost::Disarm)));
        assert!(matches!(authority.state(), LockState::Unlocked));
        let log = capture.text();
        assert!(!log.contains(PIN), "pin reached the log: {log}");
        assert!(!log.contains(&hash), "hash reached the log: {log}");
        assert!(
            !log.contains("Notepad"),
            "window title reached the log: {log}"
        );
        assert!(!log.contains(NOTE), "note reached the log: {log}");
    });
}

#[test]
fn release_build_does_not_store_a_pin() {
    let mut authority = Authority::new(ServicePlatform::new(OsLock::Fails), false);
    let response = authority.handle_public(
        Role::App,
        RequestKind::SetPin {
            pin: Secret::new(PIN),
        },
    );
    assert!(matches!(response, ResponseKind::NeedsConfirmation { .. }));
    assert!(authority.pin_hash().is_none());
}

#[test]
fn a_client_role_cannot_set_the_pin() {
    let mut authority = fresh();
    let response = authority.handle_public(
        Role::LockClient,
        RequestKind::SetPin {
            pin: Secret::new(PIN),
        },
    );
    assert_eq!(response, ResponseKind::Unauthorized);
    assert!(authority.pin_hash().is_none());
}

#[test]
fn host_loss_while_the_os_lock_is_down_never_unlocks() {
    let mut authority = fresh();
    let nonce = lock(&mut authority, LockSource::Hotkey);
    authority
        .accept_host(HostToService::HostReady { monitors: 1, nonce })
        .unwrap();
    assert_eq!(
        authority.poll_host(Instant::now(), false),
        HostAction::Respawn { arm: true }
    );
    assert!(matches!(
        authority.state(),
        LockState::FailingClosed {
            reason: softlock_core::FailReason::HostExited
        }
    ));
    let attempts = authority.os_lock_attempts();
    let nonce = authority.issue_launch_nonce().unwrap();
    authority
        .accept_host(HostToService::HostReady {
            monitors: 1,
            nonce: LaunchNonce::from_bytes(*nonce.as_bytes()),
        })
        .unwrap();
    assert!(!matches!(authority.state(), LockState::Unlocked));
    let now = Instant::now();
    authority.record_beat(now);
    let _ = authority.poll_host(now, true);
    let later = now + Duration::from_secs(1);
    authority.record_beat(later);
    assert_eq!(authority.poll_host(later, true), HostAction::RetryLock);
    assert!(authority.os_lock_attempts() > attempts);
    authority.note_host_lost(HostLoss::GuardDown);
    authority.note_host_lost(HostLoss::Silent);
    assert!(!matches!(authority.state(), LockState::Unlocked));
    assert!(!authority.returned_to_unlocked());
}

#[test]
fn os_lock_success_moves_to_os_locked_until_the_session_unlocks() {
    let mut authority = Authority::new(ServicePlatform::new(OsLock::Succeeds), true);
    let nonce = lock(&mut authority, LockSource::Hotkey);
    authority
        .accept_host(HostToService::HostReady { monitors: 1, nonce })
        .unwrap();
    authority.note_host_lost(HostLoss::Exited);
    assert!(
        matches!(
            authority.state(),
            LockState::OsLocked {
                after_fail: Some(softlock_core::FailReason::HostExited)
            }
        ),
        "a held OS lock left {:?}",
        authority.state()
    );
    assert_eq!(
        authority.poll_host(Instant::now(), false),
        HostAction::Respawn { arm: false },
        "FailingClosed must not arm a replacement while the OS lock is held"
    );
    authority.notify(Event::Session(SessionEvent::OsUnlocked));
    assert!(
        matches!(authority.state(), LockState::Unlocked),
        "session unlock left {:?}",
        authority.state()
    );
}

#[test]
fn spec_invariant_3_names_the_emergency_path() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/SPEC.md");
    let spec = std::fs::read_to_string(path).expect("spec");
    assert!(
        spec.contains("emergency path"),
        "invariant 3 does not name the emergency path"
    );
    assert!(spec.contains("HostExited"));
    assert!(spec.contains("missed heartbeats"));
    assert!(spec.contains("guard-down report"));
    assert!(
        spec.contains("never reports Unlocked"),
        "invariant 3 does not say how the service closes the path"
    );
    assert!(
        spec.contains("runs `emergency()`"),
        "section 7 does not name the emergency() case the watchdog closes"
    );
    assert!(
        !spec.contains("Known gap, until ticket 13"),
        "section 7 still leaves the emergency() case open"
    );
}

#[test]
fn tray_and_app_lock_sources_are_visible_in_get_status() {
    for source in [LockSource::Tray, LockSource::App] {
        let mut authority = fresh();
        lock(&mut authority, source);
        let status = authority.handle_public(Role::App, RequestKind::GetStatus);
        match status {
            ResponseKind::Status {
                state, lock_source, ..
            } => {
                assert_eq!(state, softlock_core::StateName::Locked);
                assert_eq!(lock_source, Some(source));
            }
            other => panic!("GetStatus was not a status: {other:?}"),
        }
    }
}

#[test]
fn hard_lock_from_the_app_while_locked_is_rejected() {
    let mut authority = fresh();
    let _nonce = lock(&mut authority, LockSource::Hotkey);
    let attempts = authority.os_lock_attempts();
    assert_eq!(
        authority.handle_public(Role::App, RequestKind::HardLock),
        ResponseKind::Unauthorized
    );
    assert!(matches!(authority.state(), LockState::Locked { .. }));
    assert_eq!(authority.os_lock_attempts(), attempts);
}

#[test]
fn hard_lock_from_the_app_while_unlocked_stays_unlocked() {
    let mut authority = Authority::new(ServicePlatform::new(OsLock::Succeeds), true);
    assert_eq!(
        authority.handle_public(Role::App, RequestKind::HardLock),
        ResponseKind::Accepted
    );
    assert!(matches!(authority.state(), LockState::Unlocked));
    assert!(authority.os_lock_attempts() >= 1);
}

#[test]
fn hard_lock_on_the_private_channel_while_locked_fails_closed() {
    let mut authority = Authority::new(ServicePlatform::new(OsLock::Succeeds), true);
    let nonce = lock(&mut authority, LockSource::Hotkey);
    let again = LaunchNonce::from_bytes(*nonce.as_bytes());
    authority
        .accept_host(HostToService::HostReady { monitors: 1, nonce })
        .unwrap();
    authority
        .accept_host(HostToService::HardLock { nonce: again })
        .unwrap();
    assert!(matches!(
        authority.state(),
        LockState::OsLocked {
            after_fail: Some(softlock_core::FailReason::HardLock)
        }
    ));
    assert!(!authority.returned_to_unlocked());
}

#[test]
fn a_new_subscriber_does_not_replay_an_old_fallback() {
    let current = Some(softlock_core::Fallback {
        reason: softlock_core::FailReason::HostExited,
        at_ms: 1_500,
        at_utc_ms: 1_795_176_000_000,
    });
    assert!(softlock_service::fallback_is_new(None, current));
    assert!(!softlock_service::fallback_is_new(current, current));
    let mut authority = Authority::new(ServicePlatform::new(OsLock::Succeeds), true);
    let nonce = lock(&mut authority, LockSource::Hotkey);
    authority
        .accept_host(HostToService::HostReady { monitors: 1, nonce })
        .unwrap();
    authority.note_host_lost(HostLoss::Exited);
    authority.notify(Event::Session(SessionEvent::OsUnlocked));
    let start = authority.fallback_stream_start();
    assert!(start.is_some());
    assert!(!softlock_service::fallback_is_new(
        start,
        authority.last_fallback()
    ));
}

#[test]
fn subscribe_is_accepted_for_the_app_only() {
    let mut authority = fresh();
    assert_eq!(
        authority.handle_public(Role::App, RequestKind::Subscribe),
        ResponseKind::Accepted
    );
    assert_eq!(
        authority.handle_public(Role::LockClient, RequestKind::Subscribe),
        ResponseKind::Unauthorized
    );
}

fn public_kind(index: u8) -> RequestKind {
    match index {
        0 => RequestKind::Hello {
            client_version: "1".to_string(),
        },
        1 => RequestKind::GetStatus,
        2 => RequestKind::Lock {
            source: LockSource::LocalApi,
            note: Some(NOTE.to_string()),
        },
        3 => RequestKind::Subscribe,
        4 => RequestKind::HardLock,
        _ => RequestKind::ClearPin,
    }
}

#[cfg(not(windows))]
#[test]
fn dev_clear_pin_removes_the_on_disk_hash() {
    let _pin_disk = pin_disk_lock();
    let dir = pin_directory();
    std::fs::create_dir_all(&dir).expect("pin dir");
    let store = PinStore::in_dir(&dir);
    store.save(&hash_pin(PIN)).expect("save");
    assert!(dir.join("pin.phc").is_file());
    let mut authority = fresh();
    assert_eq!(
        authority.handle_public(Role::App, RequestKind::ClearPin),
        ResponseKind::Ok
    );
    assert!(!dir.join("pin.phc").exists(), "clear left the hash on disk");
}

#[cfg(windows)]
#[test]
fn clear_pin_refuses_without_localsystem() {
    let mut authority = fresh();
    assert_eq!(
        authority.handle_public(Role::App, RequestKind::ClearPin),
        ResponseKind::Unsupported
    );
}

#[test]
fn an_invalid_edit_keeps_the_last_good_config() {
    let dir = std::env::temp_dir().join(format!("softlock-config-errors-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let mut authority = fresh();
    authority.set_config_dir(dir.clone());
    let ResponseKind::Config {
        toml: before,
        revision,
    } = authority.handle_public(Role::App, RequestKind::GetConfig)
    else {
        panic!("config");
    };
    let response = authority.handle_public(
        Role::App,
        RequestKind::PutConfig {
            toml: "[lock]\nhotkey = \"nope\"\n".to_string(),
            base_revision: revision,
        },
    );
    let ResponseKind::Invalid { errors } = response else {
        panic!("expected field errors, got {response:?}");
    };
    insta::assert_json_snapshot!(errors);
    let written = std::fs::read_to_string(dir.join("config.errors.toml")).expect("errors file");
    assert_eq!(written, errors.join("\n"));
    let ResponseKind::Config {
        toml,
        revision: now,
    } = authority.handle_public(Role::App, RequestKind::GetConfig)
    else {
        panic!("config");
    };
    assert_eq!(toml, before);
    assert_eq!(now, revision);
    assert!(matches!(authority.state(), LockState::Unlocked));
    let huge = format!(
        "[lock]\nmessage = \"{}\"\n",
        "a".repeat(softlock_core::MAX_CONFIG_BYTES)
    );
    let response = authority.handle_public(
        Role::App,
        RequestKind::PutConfig {
            toml: huge,
            base_revision: revision,
        },
    );
    assert!(matches!(response, ResponseKind::Invalid { .. }));
    assert_eq!(
        authority.handle_public(Role::App, RequestKind::GetConfig),
        ResponseKind::Config {
            toml: before,
            revision,
        }
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn put_config_returns_locked_while_soft_locked_and_while_windows_is_locked() {
    let mut authority = fresh();
    let ResponseKind::Config {
        toml: before,
        revision,
    } = authority.handle_public(Role::App, RequestKind::GetConfig)
    else {
        panic!("config");
    };
    lock(&mut authority, LockSource::Hotkey);
    let weakening = "[hardening]\ndisable_touch_pen = false\n".to_string();
    let response = authority.handle_public(
        Role::App,
        RequestKind::PutConfig {
            toml: weakening,
            base_revision: revision,
        },
    );
    assert_eq!(response, ResponseKind::Locked);
    assert!(!matches!(authority.state(), LockState::Unlocked));
    let ResponseKind::Config { toml, .. } =
        authority.handle_public(Role::App, RequestKind::GetConfig)
    else {
        panic!("config");
    };
    assert_eq!(toml, before);

    let mut windows = fresh();
    windows.notify(Event::Session(SessionEvent::OsLocked));
    assert!(matches!(windows.state(), LockState::Unlocked));
    let response = windows.handle_public(
        Role::App,
        RequestKind::PutConfig {
            toml: "[lock]\nhotkey = \"Ctrl+Alt+Shift+K\"\n".to_string(),
            base_revision: revision,
        },
    );
    assert_eq!(response, ResponseKind::Locked);
    assert!(windows.session_os_locked());
    let ResponseKind::Config { toml, .. } =
        windows.handle_public(Role::App, RequestKind::GetConfig)
    else {
        panic!("config");
    };
    assert_eq!(toml, before);
}

#[test]
fn a_stale_base_revision_returns_conflict() {
    let mut authority = fresh();
    let ResponseKind::Config { revision, .. } =
        authority.handle_public(Role::App, RequestKind::GetConfig)
    else {
        panic!("config");
    };
    let updated = "[lock]\nhotkey = \"Ctrl+Alt+Shift+K\"\n".to_string();
    let applied = authority.handle_public(
        Role::App,
        RequestKind::PutConfig {
            toml: updated.clone(),
            base_revision: revision,
        },
    );
    assert_eq!(applied, ResponseKind::Applied);
    let stale = authority.handle_public(
        Role::App,
        RequestKind::PutConfig {
            toml: "[lock]\nhotkey = \"Ctrl+Alt+Shift+M\"\n".to_string(),
            base_revision: revision,
        },
    );
    assert_eq!(stale, ResponseKind::Conflict);
    let ResponseKind::Config { toml, .. } =
        authority.handle_public(Role::App, RequestKind::GetConfig)
    else {
        panic!("config");
    };
    assert_eq!(toml, updated);
    assert!(matches!(authority.state(), LockState::Unlocked));
}

#[test]
fn get_config_and_validate_config_work_for_lock_client() {
    let mut authority = fresh();
    let ResponseKind::Config { toml, revision } =
        authority.handle_public(Role::LockClient, RequestKind::GetConfig)
    else {
        panic!("LockClient GetConfig");
    };
    assert_eq!(revision, 1);
    assert!(toml.contains("Ctrl+Alt+Shift+L"));
    assert_eq!(
        authority.handle_public(
            Role::LockClient,
            RequestKind::ValidateConfig { toml: toml.clone() },
        ),
        ResponseKind::Valid
    );
    let ResponseKind::Invalid { errors } = authority.handle_public(
        Role::LockClient,
        RequestKind::ValidateConfig {
            toml: "schema_version = 1\n[lock]\nhotkey = \"nope\"\n".to_string(),
        },
    ) else {
        panic!("ValidateConfig");
    };
    assert!(
        errors.iter().any(|error| error.contains("hotkey")),
        "{errors:?}"
    );
}

#[test]
fn an_absent_value_is_deleted_after_a_restart_mid_lock() {
    use softlock_core::{
        HostPresence, SessionFact, SettingValue, StartAction, StartupStep, original_previous,
        recovery_action, restore_action, startup_steps, task_manager_document,
    };
    let doc =
        task_manager_document("S-1-5-21-1-2-3-1001", SettingValue::Absent, 0).expect("journal");
    let live = SettingValue::Dword { value: 1 };
    // A different boot deletes the locked dword. It does not write zero, and
    // it runs before config load so the live value is not journaled as the
    // original. The same logon, a query error, and a dead host on an active
    // console fail closed instead of ending Unlocked.
    assert_eq!(
        recovery_action(&doc, SessionFact::DifferentBoot, true, HostPresence::Dead),
        StartAction::Revert
    );
    assert_eq!(
        recovery_action(&doc, SessionFact::SameLogon, false, HostPresence::Dead),
        StartAction::FailClosed
    );
    assert_eq!(
        recovery_action(&doc, SessionFact::Unknown, false, HostPresence::Dead),
        StartAction::FailClosed
    );
    assert_eq!(
        recovery_action(&doc, SessionFact::SameLogon, true, HostPresence::Dead),
        StartAction::FailClosed
    );
    assert_eq!(
        startup_steps(),
        [StartupStep::RecoverJournal, StartupStep::LoadConfig]
    );
    let locked = SettingValue::Dword { value: 1 };
    let previous = original_previous(Some(&doc.entries()[0].previous), &live, &locked).clone();
    assert_eq!(previous, SettingValue::Absent);
    assert_eq!(
        restore_action(&previous, &live).expect("restore"),
        Some(SettingValue::Absent)
    );
}

fn hash_pin(pin: &str) -> String {
    softlock_core::hash_softlock_pin(&Secret::new(pin)).expect("hash")
}

fn secret_message(nonce: &LaunchNonce, pin: &str) -> HostToService {
    let frame = host_secret_frame(4, "softlock_pin", &Secret::new(pin), nonce).unwrap();
    softlock_core::decode_host_frame(&frame).unwrap()
}

fn lock_with_note(authority: &mut Authority) -> LaunchNonce {
    let nonce = authority.issue_launch_nonce().expect("nonce");
    authority.handle_public(
        Role::App,
        RequestKind::Lock {
            source: LockSource::App,
            note: Some(NOTE.to_string()),
        },
    );
    nonce
}
