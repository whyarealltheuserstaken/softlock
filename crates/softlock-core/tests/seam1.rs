//! Seam 1. Lock core against the in-memory platform: state machine, backoff,
//! heartbeat, journal, and IPC. No operating-system lock is involved.

use std::time::Duration;

use proptest::prelude::*;
use proptest::string::string_regex;
use softlock_core::{
    ARM_STEP_TIMEOUT, ApplyObservation, ArmStep, Event, FailReason, FailureEvent, FakePlatform,
    GuardHealth, HEARTBEAT_INTERVAL, HEARTBEAT_MISSES_BEFORE_FAILURE, HeartbeatReport, IpcError,
    Journal, JournalEntry, LockController, LockSource, LockState, MethodId, OverlayHealth,
    RequestKind, ResponseKind, Role, SERVICE_SILENCE_LIMIT, Secret, SessionEvent, StateName,
    UNSUPPORTED_PLATFORM_MESSAGE, UnsupportedPlatform, VerdictChannel,
};

fn started(platform: FakePlatform) -> LockController<FakePlatform> {
    LockController::new(platform)
}

fn locked_started() -> LockController<FakePlatform> {
    let mut controller = started(FakePlatform::new());
    controller.handle(Event::LockRequested {
        source: LockSource::Hotkey,
    });
    assert!(
        matches!(controller.state(), LockState::Locked { .. }),
        "lock request should reach Locked, state is {:?}",
        controller.state()
    );
    controller
}

/// The backoff waits are longer than the heartbeat deadline, so a locked
/// controller has to keep reporting healthy while virtual time moves.
fn advance_while_locked(controller: &mut LockController<FakePlatform>, by: Duration) {
    let mut left = by;
    while !left.is_zero() {
        let chunk = if left > HEARTBEAT_INTERVAL {
            HEARTBEAT_INTERVAL
        } else {
            left
        };
        controller.handle(Event::AdvanceTime(chunk));
        controller.handle(Event::Heartbeat(HeartbeatReport {
            seq: 1,
            guard: healthy_guard(),
            overlay: healthy_overlay(),
        }));
        left -= chunk;
    }
}

fn healthy_guard() -> GuardHealth {
    GuardHealth {
        hook_alive: true,
        last_physical_input_ms: None,
    }
}

fn healthy_overlay() -> OverlayHealth {
    OverlayHealth {
        covered_monitors: 1,
        capture_excluded: true,
        topmost: true,
    }
}

fn put_config(revision: u64) -> RequestKind {
    RequestKind::PutConfig {
        toml: "schema_version = 1\n".to_string(),
        base_revision: revision,
    }
}

fn public(kind: RequestKind) -> Event {
    Event::Public {
        role: Role::App,
        id: 1,
        kind,
    }
}

fn reject_config(controller: &mut LockController<FakePlatform>) {
    let before = controller.state().clone();
    let revision = controller.config_revision();
    controller.handle(public(put_config(revision)));
    assert_eq!(controller.state(), &before);
    assert_eq!(controller.last_response(), Some(&ResponseKind::Locked));
}

fn sample_requests() -> Vec<RequestKind> {
    vec![
        RequestKind::Hello {
            client_version: "0.1.0".to_string(),
        },
        RequestKind::GetStatus,
        RequestKind::Lock {
            source: LockSource::Tray,
            note: Some("agent".to_string()),
        },
        RequestKind::Lock {
            source: LockSource::App,
            note: None,
        },
        RequestKind::HardLock,
        RequestKind::GetConfig,
        RequestKind::ValidateConfig {
            toml: "schema_version = 1\n".to_string(),
        },
        RequestKind::PutConfig {
            toml: "schema_version = 1\n".to_string(),
            base_revision: 1,
        },
        RequestKind::ConfirmSensitiveChange {
            token: "token".to_string(),
        },
        RequestKind::SetPin {
            pin: "1234".to_string(),
        },
        RequestKind::ClearPin,
        RequestKind::Subscribe,
        RequestKind::CheckUpdate,
        RequestKind::InstallUpdate {
            version: "0.2.0".to_string(),
        },
    ]
}

#[test]
fn locked_leaves_only_on_a_verified_verdict_from_the_lock_host_or_its_broker() {
    for kind in sample_requests() {
        for role in [Role::App, Role::LockClient] {
            let mut controller = locked_started();
            controller.handle(Event::Public {
                role,
                id: 7,
                kind: kind.clone(),
            });
            assert!(
                matches!(controller.state(), LockState::Locked { .. }),
                "{role:?} {kind:?} left Locked for {:?}",
                controller.state()
            );
            assert!(
                !controller.returned_to_unlocked(),
                "{role:?} {kind:?} reached Unlocked"
            );
        }
    }

    let mut controller = locked_started();
    controller.handle(public(RequestKind::SetPin {
        pin: "1234".to_string(),
    }));
    let rendered = format!("{:?}", controller.rejection_log());
    assert!(!rendered.contains("1234"));
    assert!(!controller.config_toml().contains("1234"));

    let mut controller = locked_started();
    controller.handle(Event::Verified {
        channel: VerdictChannel::Broker,
        method: MethodId::windows_hello(),
    });
    assert!(matches!(controller.state(), LockState::Locked { .. }));
    assert!(
        controller
            .rejection_log()
            .iter()
            .any(|rejection| rejection.event == "Verified")
    );

    let mut controller = locked_started();
    controller.handle(Event::RequestHello);
    assert!(controller.broker_launched());
    controller.handle(Event::Verified {
        channel: VerdictChannel::Broker,
        method: MethodId::windows_hello(),
    });
    assert!(
        controller
            .visited()
            .iter()
            .any(|state| matches!(state, LockState::Unlocking { .. }))
    );
    assert!(matches!(controller.state(), LockState::Unlocked));

    let mut controller = locked_started();
    controller.handle(Event::Verified {
        channel: VerdictChannel::LockHost,
        method: MethodId::os_password(),
    });
    assert!(
        controller
            .visited()
            .iter()
            .any(|state| matches!(state, LockState::Unlocking { .. }))
    );
    assert!(matches!(controller.state(), LockState::Unlocked));
    assert!(controller.rejection_log().is_empty());
}

#[test]
fn hardening_changes_are_journaled_before_they_are_applied() {
    let mut platform = FakePlatform::new();
    platform.set_setting("DisableTaskMgr", "admin-set");
    let mut controller = started(platform);
    controller.handle(Event::LockRequested {
        source: LockSource::App,
    });

    let observations = controller.platform().apply_observations();
    assert_eq!(observations.len(), 1, "hardening apply should run once");
    let ApplyObservation {
        durable_entries,
        settings_before,
    } = &observations[0];
    assert!(
        durable_entries.iter().any(|entry| {
            entry.target == "DisableTaskMgr"
                && entry.previous.as_deref() == Some("admin-set")
                && entry.new_value == "1"
        }),
        "persisted journal did not contain the change before apply: {durable_entries:?}"
    );
    assert_eq!(
        settings_before.get("DisableTaskMgr").map(String::as_str),
        Some("admin-set"),
        "the setting changed before the journal was durable"
    );
    assert_eq!(
        controller
            .platform()
            .settings()
            .get("DisableTaskMgr")
            .map(String::as_str),
        Some("1")
    );
    assert!(!controller.journal().is_empty());
}

#[test]
fn arming_or_locked_failure_ends_in_os_locked_never_unlocked() {
    for step in [
        ArmStep::Journal,
        ArmStep::Guard,
        ArmStep::Overlay,
        ArmStep::Hardening,
        ArmStep::Power,
    ] {
        let mut platform = FakePlatform::new();
        platform.fail_step(step);
        let mut controller = started(platform);
        controller.handle(Event::LockRequested {
            source: LockSource::Hotkey,
        });
        assert!(
            matches!(controller.state(), LockState::FailingClosed { .. }),
            "{step:?} should stop in FailingClosed, got {:?}",
            controller.state()
        );
        controller.handle(Event::OsLockConfirmed);
        assert!(
            matches!(
                controller.state(),
                LockState::OsLocked {
                    after_fail: Some(_)
                }
            ),
            "{step:?} should end in OsLocked"
        );
        assert!(
            !controller.returned_to_unlocked(),
            "{step:?} returned to Unlocked"
        );
    }

    let locked_failures = [
        (
            Event::AdvanceTime(HEARTBEAT_INTERVAL * HEARTBEAT_MISSES_BEFORE_FAILURE),
            FailReason::HeartbeatMissed,
        ),
        (Event::HostExited, FailReason::HostExited),
        (
            Event::Heartbeat(HeartbeatReport {
                seq: 1,
                guard: GuardHealth {
                    hook_alive: false,
                    last_physical_input_ms: None,
                },
                overlay: healthy_overlay(),
            }),
            FailReason::GuardUnhealthy,
        ),
        (
            Event::Heartbeat(HeartbeatReport {
                seq: 2,
                guard: healthy_guard(),
                overlay: OverlayHealth {
                    covered_monitors: 1,
                    capture_excluded: false,
                    topmost: true,
                },
            }),
            FailReason::OverlayUnhealthy,
        ),
    ];

    for (event, reason) in locked_failures {
        let mut controller = locked_started();
        controller.handle(event);
        assert!(
            controller.failure_events().contains(&reason),
            "missing {reason:?} in {:?}",
            controller.failure_events()
        );
        assert!(matches!(
            controller.state(),
            LockState::FailingClosed { .. }
        ));
        controller.handle(Event::OsLockConfirmed);
        assert!(matches!(
            controller.state(),
            LockState::OsLocked {
                after_fail: Some(failed)
            } if failed == &reason
        ));
        assert!(!controller.returned_to_unlocked());
    }
}

#[test]
fn config_writes_are_rejected_outside_unlocked_with_the_session_not_os_locked() {
    let mut unlocked = started(FakePlatform::new());
    let revision = unlocked.config_revision();
    unlocked.handle(public(put_config(revision)));
    assert_eq!(unlocked.last_response(), Some(&ResponseKind::Applied));
    assert_eq!(unlocked.config_revision(), revision + 1);

    unlocked.handle(public(RequestKind::PutConfig {
        toml: String::new(),
        base_revision: unlocked.config_revision(),
    }));
    assert_eq!(
        unlocked.last_response(),
        Some(&ResponseKind::Invalid {
            errors: vec!["empty config".to_string()],
        })
    );

    unlocked.handle(public(put_config(0)));
    assert_eq!(unlocked.last_response(), Some(&ResponseKind::Conflict));

    unlocked.handle(Event::Public {
        role: Role::LockClient,
        id: 3,
        kind: put_config(unlocked.config_revision()),
    });
    assert_eq!(unlocked.last_response(), Some(&ResponseKind::Unauthorized));

    let mut session_locked = started(FakePlatform::new());
    session_locked.handle(Event::Session(SessionEvent::OsLocked));
    assert!(matches!(session_locked.state(), LockState::Unlocked));
    assert!(session_locked.session_os_locked());
    reject_config(&mut session_locked);
    session_locked.handle(Event::Session(SessionEvent::OsUnlocked));
    assert!(!session_locked.session_os_locked());
    let revision = session_locked.config_revision();
    session_locked.handle(public(put_config(revision)));
    assert_eq!(session_locked.last_response(), Some(&ResponseKind::Applied));

    let mut arming = started(FakePlatform::new());
    arming.drive_manually();
    arming.handle(Event::LockRequested {
        source: LockSource::Hotkey,
    });
    assert!(matches!(arming.state(), LockState::Arming { .. }));
    reject_config(&mut arming);

    let mut locked = locked_started();
    reject_config(&mut locked);

    locked.drive_manually();
    locked.handle(Event::Verified {
        channel: VerdictChannel::LockHost,
        method: MethodId::os_password(),
    });
    assert!(matches!(locked.state(), LockState::Unlocking { .. }));
    reject_config(&mut locked);

    let mut platform = FakePlatform::new();
    platform.fail_step(ArmStep::Guard);
    let mut failing = started(platform);
    failing.handle(Event::LockRequested {
        source: LockSource::Hotkey,
    });
    assert!(matches!(failing.state(), LockState::FailingClosed { .. }));
    reject_config(&mut failing);
    failing.handle(Event::OsLockConfirmed);
    assert!(matches!(failing.state(), LockState::OsLocked { .. }));
    reject_config(&mut failing);

    let mut journal = Journal::empty();
    journal.record(JournalEntry {
        target: "DisableTaskMgr".to_string(),
        previous: Some("0".to_string()),
        new_value: "1".to_string(),
    });
    journal.mark_locked();
    let mut recovering = started(FakePlatform::new());
    recovering.drive_manually();
    recovering.handle(Event::ServiceStarted {
        journal,
        session_active: true,
    });
    assert!(matches!(recovering.state(), LockState::Recovering));
    reject_config(&mut recovering);
}

#[test]
fn unlocked_leaves_the_journal_empty_and_restores_pre_lock_settings() {
    let mut platform = FakePlatform::new();
    platform.set_setting("DisableTaskMgr", "admin-set");
    platform.set_setting("DisableLockWorkstation", "0");
    let before = platform.settings().clone();
    let mut controller = started(platform);
    controller.handle(Event::LockRequested {
        source: LockSource::LocalApi,
    });
    assert_ne!(controller.platform().settings(), &before);
    assert!(!controller.journal().is_empty());

    controller.handle(Event::Verified {
        channel: VerdictChannel::LockHost,
        method: MethodId::softlock_pin(),
    });
    assert!(matches!(controller.state(), LockState::Unlocked));
    assert!(controller.journal().is_empty());
    assert_eq!(controller.platform().settings(), &before);

    controller.handle(Event::LockRequested {
        source: LockSource::Hotkey,
    });
    controller.handle(Event::HostExited);
    controller.handle(Event::OsLockConfirmed);
    assert!(matches!(
        controller.state(),
        LockState::OsLocked {
            after_fail: Some(_)
        }
    ));
    controller.handle(Event::Session(SessionEvent::OsUnlocked));
    assert!(matches!(controller.state(), LockState::Unlocked));
    assert!(controller.journal().is_empty());
    assert_eq!(controller.platform().settings(), &before);
    assert!(
        controller
            .notifications()
            .iter()
            .any(|notice| notice.reason == FailReason::HostExited)
    );
}

#[test]
fn unlisted_transition_is_rejected_and_logged() {
    let mut controller = started(FakePlatform::new());
    controller.handle(Event::Verified {
        channel: VerdictChannel::LockHost,
        method: MethodId::os_password(),
    });
    assert!(matches!(controller.state(), LockState::Unlocked));
    assert!(
        controller
            .rejection_log()
            .iter()
            .any(|rejection| rejection.event == "Verified")
    );

    let mut controller = locked_started();
    controller.handle(Event::Session(SessionEvent::OsUnlocked));
    assert!(matches!(controller.state(), LockState::Locked { .. }));
    assert!(
        controller
            .rejection_log()
            .iter()
            .any(|rejection| rejection.event == "OsUnlocked")
    );
}

#[test]
fn backoff_attempts_one_through_four_have_no_delay_then_double_from_thirty_seconds_to_fifteen_minutes_and_reset_on_success()
 {
    let mut controller = locked_started();
    for attempt in 1..=4 {
        controller.handle(Event::SubmitSecret {
            method: MethodId::os_password(),
            secret: Secret::new("nope"),
        });
        assert_eq!(controller.failure_count(), attempt);
        assert_eq!(controller.backoff_delay(), Duration::ZERO);
        assert_eq!(controller.journal().failures(), attempt);
        assert!(!matches!(
            controller.state(),
            LockState::Locked {
                prompt: softlock_core::PromptState::Backoff { .. },
                ..
            }
        ));
    }

    controller.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("nope"),
    });
    assert_eq!(controller.failure_count(), 5);
    assert_eq!(controller.backoff_delay(), Duration::from_secs(30));
    assert_eq!(controller.journal().failures(), 5);
    assert!(matches!(
        controller.state(),
        LockState::Locked {
            prompt: softlock_core::PromptState::Backoff { failures: 5, .. },
            ..
        }
    ));

    controller.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("nope"),
    });
    assert_eq!(
        controller.failure_count(),
        5,
        "backoff should hold the attempt"
    );

    advance_while_locked(
        &mut controller,
        Duration::from_secs(30) - Duration::from_millis(1),
    );
    controller.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("nope"),
    });
    assert_eq!(controller.failure_count(), 5);
    controller.handle(Event::AdvanceTime(Duration::from_millis(1)));

    let schedule = [
        (6, 60u64),
        (7, 120),
        (8, 240),
        (9, 480),
        (10, 900),
        (11, 900),
    ];
    for (attempt, seconds) in schedule {
        controller.handle(Event::SubmitSecret {
            method: MethodId::os_password(),
            secret: Secret::new("nope"),
        });
        assert_eq!(controller.failure_count(), attempt);
        assert_eq!(controller.backoff_delay(), Duration::from_secs(seconds));
        assert_eq!(controller.journal().failures(), attempt);
        advance_while_locked(&mut controller, Duration::from_secs(seconds));
    }

    let leaked = format!("{:?}", controller.rejection_log());
    assert!(!leaked.contains("nope"));

    controller.platform_mut().accept_secrets(true);
    controller.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("ok"),
    });
    assert!(matches!(controller.state(), LockState::Unlocked));
    assert_eq!(controller.failure_count(), 0);
    assert!(controller.journal().is_empty());

    controller.handle(Event::LockRequested {
        source: LockSource::Hotkey,
    });
    controller.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("nope"),
    });
    assert_eq!(controller.failure_count(), 1);
    assert_eq!(controller.backoff_delay(), Duration::ZERO);
    assert_eq!(controller.journal().failures(), 1);
}

#[test]
fn three_missed_heartbeats_or_an_unhealthy_guard_or_overlay_emits_a_failure() {
    assert_eq!(HEARTBEAT_INTERVAL, Duration::from_millis(250));
    assert_eq!(HEARTBEAT_MISSES_BEFORE_FAILURE, 3);
    assert_eq!(ARM_STEP_TIMEOUT, Duration::from_secs(2));
    assert_eq!(SERVICE_SILENCE_LIMIT, Duration::from_secs(1));
    assert!(
        !softlock_core::lock_host_fails_closed_when_service_is_silent(Duration::from_millis(999))
    );
    assert!(softlock_core::lock_host_fails_closed_when_service_is_silent(SERVICE_SILENCE_LIMIT));

    let mut beating = locked_started();
    beating.handle(Event::AdvanceTime(HEARTBEAT_INTERVAL * 2));
    assert!(beating.failure_events().is_empty());
    beating.handle(Event::AdvanceTime(Duration::from_millis(249)));
    assert!(beating.failure_events().is_empty());
    beating.handle(Event::AdvanceTime(Duration::from_millis(1)));
    assert_eq!(beating.failure_events(), &[FailureEvent::HeartbeatMissed]);

    let mut reset = locked_started();
    reset.handle(Event::AdvanceTime(HEARTBEAT_INTERVAL * 2));
    reset.handle(Event::Heartbeat(HeartbeatReport {
        seq: 1,
        guard: healthy_guard(),
        overlay: healthy_overlay(),
    }));
    reset.handle(Event::AdvanceTime(HEARTBEAT_INTERVAL * 2));
    assert!(reset.failure_events().is_empty());
    reset.handle(Event::AdvanceTime(HEARTBEAT_INTERVAL));
    assert_eq!(reset.failure_events(), &[FailureEvent::HeartbeatMissed]);

    let mut controller = locked_started();
    controller.handle(Event::Heartbeat(HeartbeatReport {
        seq: 1,
        guard: GuardHealth {
            hook_alive: false,
            last_physical_input_ms: None,
        },
        overlay: healthy_overlay(),
    }));
    assert_eq!(controller.failure_events(), &[FailureEvent::GuardUnhealthy]);

    let mut controller = locked_started();
    controller.handle(Event::Heartbeat(HeartbeatReport {
        seq: 1,
        guard: healthy_guard(),
        overlay: OverlayHealth {
            covered_monitors: 0,
            capture_excluded: true,
            topmost: false,
        },
    }));
    assert_eq!(
        controller.failure_events(),
        &[FailureEvent::OverlayUnhealthy]
    );

    let mut idle = started(FakePlatform::new());
    idle.handle(Event::AdvanceTime(Duration::from_secs(5)));
    assert!(idle.failure_events().is_empty());

    let mut arming = started(FakePlatform::new());
    arming.drive_manually();
    arming.handle(Event::LockRequested {
        source: LockSource::Hotkey,
    });
    arming.handle(Event::AdvanceTime(
        ARM_STEP_TIMEOUT - Duration::from_millis(1),
    ));
    assert!(arming.failure_events().is_empty());
    arming.handle(Event::AdvanceTime(Duration::from_millis(1)));
    assert_eq!(arming.failure_events(), &[FailureEvent::ArmTimeout]);
}

#[test]
fn ipc_request_kinds_serialize_to_versioned_json_snapshots() {
    for (name, kind) in [
        (
            "request-hello",
            RequestKind::Hello {
                client_version: "0.1.0".to_string(),
            },
        ),
        ("request-get-status", RequestKind::GetStatus),
        (
            "request-lock",
            RequestKind::Lock {
                source: LockSource::Hotkey,
                note: Some("agent".to_string()),
            },
        ),
        ("request-hard-lock", RequestKind::HardLock),
        ("request-get-config", RequestKind::GetConfig),
        (
            "request-validate-config",
            RequestKind::ValidateConfig {
                toml: "schema_version = 1\n".to_string(),
            },
        ),
        (
            "request-put-config",
            RequestKind::PutConfig {
                toml: "schema_version = 1\n".to_string(),
                base_revision: 3,
            },
        ),
        (
            "request-confirm-sensitive-change",
            RequestKind::ConfirmSensitiveChange {
                token: "token".to_string(),
            },
        ),
        (
            "request-set-softlock-pin",
            RequestKind::SetPin {
                pin: "1234".to_string(),
            },
        ),
        ("request-clear-softlock-pin", RequestKind::ClearPin),
        ("request-subscribe", RequestKind::Subscribe),
        ("request-check-update", RequestKind::CheckUpdate),
        (
            "request-install-update",
            RequestKind::InstallUpdate {
                version: "0.2.0".to_string(),
            },
        ),
    ] {
        let json = softlock_core::request_json(11, &kind);
        let value: serde_json::Value = serde_json::from_str(&json).expect("request json");
        assert_eq!(value["v"], 1, "{name} is missing the version field");
        assert_eq!(value["id"], 11);
        insta::assert_json_snapshot!(name, value);

        let frame = softlock_core::encode_frame(&json);
        let length = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
        assert_eq!(length, json.len());
        assert_eq!(&frame[4..], json.as_bytes());
        let decoded = softlock_core::decode_request_frame(&frame).expect("round trip");
        assert_eq!(decoded.id, 11);
        assert_eq!(decoded.kind, kind);
    }
}

#[test]
fn ipc_response_kinds_serialize_to_versioned_json_snapshots() {
    let samples = [
        (
            "response-hello",
            ResponseKind::Hello {
                service_version: "0.1.0".to_string(),
                state: StateName::Unlocked,
                capabilities: vec!["lock".to_string()],
            },
        ),
        (
            "response-status",
            ResponseKind::Status {
                state: StateName::Locked,
                lock_source: Some(LockSource::Hotkey),
                guard: healthy_guard(),
                overlay: healthy_overlay(),
                last_fallback: None,
            },
        ),
        ("response-accepted", ResponseKind::Accepted),
        ("response-already-locked", ResponseKind::AlreadyLocked),
        (
            "response-config",
            ResponseKind::Config {
                toml: "schema_version = 1\n".to_string(),
                revision: 4,
            },
        ),
        ("response-valid", ResponseKind::Valid),
        (
            "response-invalid",
            ResponseKind::Invalid {
                errors: vec!["empty config".to_string()],
            },
        ),
        ("response-applied", ResponseKind::Applied),
        (
            "response-needs-confirmation",
            ResponseKind::NeedsConfirmation {
                fields: vec!["unlock.methods".to_string()],
            },
        ),
        ("response-conflict", ResponseKind::Conflict),
        ("response-locked", ResponseKind::Locked),
        ("response-ok", ResponseKind::Ok),
        ("response-up-to-date", ResponseKind::UpToDate),
        (
            "response-available",
            ResponseKind::Available {
                version: "0.2.0".to_string(),
                notes: "notes".to_string(),
            },
        ),
        ("response-installing", ResponseKind::Installing),
        ("response-unsupported", ResponseKind::Unsupported),
        ("response-unauthorized", ResponseKind::Unauthorized),
        (
            "event-state-changed",
            ResponseKind::StateChanged {
                state: StateName::Locked,
            },
        ),
        (
            "event-config-changed",
            ResponseKind::ConfigChanged { revision: 4 },
        ),
        (
            "event-fell-back",
            ResponseKind::FellBackToOsLock {
                reason: FailReason::HeartbeatMissed,
                at_ms: 750,
            },
        ),
        (
            "event-update-available",
            ResponseKind::UpdateAvailable {
                version: "0.2.0".to_string(),
            },
        ),
    ];

    for (name, kind) in samples {
        let json = softlock_core::response_json(11, &kind);
        let value: serde_json::Value = serde_json::from_str(&json).expect("response json");
        assert_eq!(value["v"], 1, "{name} is missing the version field");
        assert_eq!(value["id"], 11);
        insta::assert_json_snapshot!(name, value);
    }
}

#[test]
fn ipc_unknown_major_version_is_rejected_as_unsupported() {
    for version in [0, 2, 99] {
        let json = format!(r#"{{"v":{version},"id":9,"kind":"Hello","client_version":"0.1.0"}}"#);
        let frame = softlock_core::encode_frame(&json);
        let error = softlock_core::decode_request_frame(&frame).expect_err("unknown major");
        assert_eq!(error, IpcError::Unsupported { id: 9 });
    }

    let unlock = softlock_core::encode_frame(r#"{"v":1,"id":4,"kind":"Unlock"}"#);
    assert_eq!(
        softlock_core::decode_request_frame(&unlock),
        Err(IpcError::Malformed)
    );
}

#[test]
fn unsupported_platform_message_matches_the_string_resource() {
    let value = common_json("crates/softlock-app/src/strings/platform.json");
    assert_eq!(value["unsupported"], UNSUPPORTED_PLATFORM_MESSAGE);
    assert_eq!(
        UnsupportedPlatform.capability_report().message,
        UNSUPPORTED_PLATFORM_MESSAGE
    );
    assert!(!UnsupportedPlatform.capability_report().is_supported());
}

fn common_json(relative: &str) -> serde_json::Value {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).expect("json")
}

fn arb_request() -> impl Strategy<Value = RequestKind> {
    prop_oneof![
        string_regex("[a-z0-9.]{1,8}")
            .unwrap()
            .prop_map(|client_version| { RequestKind::Hello { client_version } }),
        Just(RequestKind::GetStatus),
        (
            prop_oneof![
                Just(LockSource::Hotkey),
                Just(LockSource::Tray),
                Just(LockSource::App),
                Just(LockSource::LocalApi),
            ],
            prop::option::of(string_regex("[a-z]{0,12}").unwrap()),
        )
            .prop_map(|(source, note)| RequestKind::Lock { source, note }),
        Just(RequestKind::HardLock),
        Just(RequestKind::GetConfig),
        string_regex("[a-z0-9]{0,16}")
            .unwrap()
            .prop_map(|toml| { RequestKind::ValidateConfig { toml } }),
        (
            string_regex("[a-z0-9]{0,16}").unwrap(),
            any::<u8>().prop_map(u64::from),
        )
            .prop_map(|(toml, base_revision)| RequestKind::PutConfig {
                toml,
                base_revision,
            }),
        string_regex("[a-z0-9]{0,8}")
            .unwrap()
            .prop_map(|token| { RequestKind::ConfirmSensitiveChange { token } }),
        string_regex("[0-9]{4}")
            .unwrap()
            .prop_map(|pin| RequestKind::SetPin { pin }),
        Just(RequestKind::ClearPin),
        Just(RequestKind::Subscribe),
        Just(RequestKind::CheckUpdate),
        string_regex("[0-9.]{1,8}")
            .unwrap()
            .prop_map(|version| { RequestKind::InstallUpdate { version } }),
    ]
}

proptest! {
    #[test]
    fn public_requests_and_os_events_never_move_locked_to_unlocking_or_unlocked(
        events in prop::collection::vec(
            prop_oneof![
                (prop_oneof![Just(Role::App), Just(Role::LockClient)], arb_request())
                    .prop_map(|(role, kind)| Event::Public { role, id: 1, kind }),
                prop_oneof![
                    Just(SessionEvent::OsLocked),
                    Just(SessionEvent::OsUnlocked),
                    Just(SessionEvent::Logon),
                    Just(SessionEvent::Logoff),
                ].prop_map(Event::Session),
            ],
            0..24,
        )
    ) {
        let mut controller = locked_started();
        for event in events {
            let before = controller.state().clone();
            controller.handle(event);
            if matches!(before, LockState::Locked { .. }) {
                prop_assert!(
                    !matches!(
                        controller.state(),
                        LockState::Unlocking { .. } | LockState::Unlocked
                    ),
                    "public request or OS event moved Locked to {:?}",
                    controller.state()
                );
            }
            prop_assert!(
                !matches!(controller.state(), LockState::Unlocking { .. }),
                "sequence entered Unlocking"
            );
        }
    }
}
