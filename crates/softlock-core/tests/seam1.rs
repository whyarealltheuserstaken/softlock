//! Seam 1. Lock core against the in-memory platform: state machine, backoff,
//! heartbeat, journal, and IPC. No operating-system lock is involved.

use std::time::Duration;

use proptest::prelude::*;
use proptest::string::string_regex;
use softlock_core::{
    ARM_STEP_TIMEOUT, ArmStep, Event, FailReason, FakePlatform, Fallback, GuardHealth,
    HEARTBEAT_INTERVAL, HEARTBEAT_MISSES_BEFORE_FAILURE, HardeningPlan, HeartbeatReport, IpcError,
    Journal, JournalEntry, LaunchNonce, LockController, LockSource, LockState, MAX_FRAME, MethodId,
    OverlayHealth, PlannedChange, PolicySetting, PromptState, RequestKind, ResponseKind, Role,
    SERVICE_SILENCE_LIMIT, Secret, SessionEvent, SessionId, StateName,
    UNSUPPORTED_PLATFORM_MESSAGE, UnsupportedPlatform, Verdict, checked_body_len,
    journal_is_durable_before_apply,
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
        if !matches!(controller.state(), LockState::Locked { .. }) {
            return;
        }
        let seq = controller.next_heartbeat_seq();
        controller.handle(Event::Heartbeat(HeartbeatReport {
            seq,
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

fn verify(controller: &mut LockController<FakePlatform>) {
    controller.handle(Event::RequestHello);
    let nonce = controller.broker_nonce().expect("broker nonce");
    controller.handle(Event::Verified { nonce });
}

fn counter_nonce(value: u8) -> LaunchNonce {
    let mut bytes = [0u8; 32];
    bytes[0] = value;
    LaunchNonce::from_bytes(bytes)
}

/// One byte of the real broker nonce differs. The other 31 match, so a compare
/// that stops early or skips an end still has to reject this verdict.
fn flipped_broker_nonce_stays_locked(index: usize) {
    let mut controller = locked_started();
    controller.handle(Event::RequestHello);
    let nonce = controller.broker_nonce().expect("broker nonce");
    let mut bytes = *nonce.as_bytes();
    bytes[index] ^= 0x01;
    assert_ne!(bytes[index], nonce.as_bytes()[index]);
    assert_eq!(&bytes[..index], &nonce.as_bytes()[..index]);
    assert_eq!(&bytes[index + 1..], &nonce.as_bytes()[index + 1..]);

    controller.handle(Event::Verified {
        nonce: LaunchNonce::from_bytes(bytes),
    });
    assert!(
        matches!(controller.state(), LockState::Locked { .. }),
        "flipping byte {index} of the broker nonce must stay Locked, state is {:?}",
        controller.state()
    );
    assert!(
        !controller.returned_to_unlocked(),
        "flipping byte {index} unlocked"
    );

    controller.handle(Event::Verified { nonce });
    assert!(
        matches!(controller.state(), LockState::Unlocked),
        "the unflipped broker nonce must still verify"
    );
}

fn public(kind: RequestKind) -> Event {
    Event::Public {
        role: Role::App,
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
            token: Secret::new("token"),
        },
        RequestKind::SetPin {
            pin: Secret::new("1234"),
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
fn locked_leaves_only_on_a_verified_broker_verdict() {
    for role in [Role::App, Role::LockClient] {
        for kind in sample_requests() {
            let mut controller = locked_started();
            let hard_lock = role == Role::App && matches!(kind, RequestKind::HardLock);
            let label = format!("{role:?} {kind:?}");
            controller.handle(Event::Public { role, kind });
            assert!(
                if hard_lock {
                    matches!(
                        controller.state(),
                        LockState::FailingClosed {
                            reason: FailReason::HardLock
                        }
                    )
                } else {
                    matches!(controller.state(), LockState::Locked { .. })
                },
                "{label} left the lock path for {:?}",
                controller.state()
            );
            assert!(
                !controller.returned_to_unlocked(),
                "{label} reached Unlocked"
            );
        }
    }

    let mut controller = locked_started();
    let event = public(RequestKind::SetPin {
        pin: Secret::new("1234"),
    });
    let rendered = format!("{event:?}");
    assert!(!rendered.contains("1234"));
    controller.handle(event);
    assert_redacted(&debug_output(&controller), "1234");

    let mut controller = locked_started();
    controller.handle(Event::Verified {
        nonce: counter_nonce(1),
    });
    assert!(
        matches!(controller.state(), LockState::Locked { .. }),
        "a Lock Host style counter nonce must not unlock"
    );
    assert!(
        !controller
            .visited()
            .iter()
            .any(|state| matches!(state, LockState::Unlocking { .. }))
    );

    let mut controller = locked_started();
    controller.handle(Event::RequestHello);
    assert!(controller.broker_launched());
    let nonce = controller.broker_nonce().expect("broker nonce");
    assert!(
        !is_counter_nonce(&nonce),
        "the broker nonce must come from a CSPRNG, not a counter"
    );
    controller.handle(Event::Verified {
        nonce: counter_nonce(1),
    });
    assert!(
        matches!(controller.state(), LockState::Locked { .. }),
        "a Lock Host verdict must not unlock"
    );
    controller.handle(Event::Verified { nonce });
    assert!(
        controller
            .visited()
            .iter()
            .any(|state| matches!(state, LockState::Unlocking { .. }))
    );
    assert!(matches!(controller.state(), LockState::Unlocked));
    assert!(controller.rejection_log().len() == 1);
}

fn is_counter_nonce(nonce: &LaunchNonce) -> bool {
    let bytes = nonce.as_bytes();
    bytes[1..].iter().all(|byte| *byte == 0)
}

#[test]
fn verified_verdict_rejects_a_broker_nonce_flipped_only_in_the_last_byte() {
    flipped_broker_nonce_stays_locked(31);
}

#[test]
fn verified_verdict_rejects_a_broker_nonce_flipped_only_in_the_first_byte() {
    flipped_broker_nonce_stays_locked(0);
}

#[test]
fn secret_equality_uses_the_constant_time_comparison() {
    let pin = b"pin-value";
    assert_eq!(Secret::new(pin), Secret::new(pin));
    assert_ne!(Secret::new(pin), Secret::new(b"other-val"));
    assert_ne!(Secret::new(pin), Secret::new(b"pin-value-extra"));
    assert_ne!(Secret::new(b"pin-value-extra"), Secret::new(pin));
    assert_ne!(Secret::new(pin), Secret::new(b"pin-valuE"));

    let source = include_str!("../src/state.rs");
    let eq = function_body_after(source, "impl PartialEq for Secret", "fn eq(");
    // `len == len && constant_time_eq(...)` still skips the walk when the lengths differ.
    assert_eq!(
        trimmed_fn_body(eq),
        "constant_time_eq(&self.bytes, &other.bytes)",
        "Secret::eq must be only the constant-time call, body was {eq}"
    );
    let ct = function_body(source, "fn constant_time_eq(");
    assert!(
        !leaves_early(ct),
        "constant_time_eq must not return on the first mismatch, body was {ct}"
    );
    assert!(
        ct.contains("diff == 0"),
        "constant_time_eq must decide only after the full walk"
    );
}

fn trimmed_fn_body(body: &str) -> &str {
    body.trim()
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap_or(body)
        .trim()
}

fn leaves_early(body: &str) -> bool {
    body.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .any(|word| word == "return" || word == "break")
}

fn function_body_after<'a>(source: &'a str, marker: &str, signature: &str) -> &'a str {
    let region = source
        .find(marker)
        .map(|start| &source[start..])
        .unwrap_or_else(|| panic!("missing {marker}"));
    function_body(region, signature)
}

fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("missing {signature}"));
    let rest = &source[start..];
    let open = rest.find('{').expect("function body");
    let mut depth = 0;
    for (index, byte) in rest[open..].bytes().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[open..=open + index];
                }
            }
            _ => {}
        }
    }
    panic!("unclosed {signature}");
}

fn debug_output(controller: &LockController<FakePlatform>) -> String {
    format!(
        "{:?} {:?} {:?} {:?} {:?} {:?} {}",
        controller.rejection_log(),
        controller.visited(),
        controller.journal(),
        controller.notifications(),
        controller.last_response(),
        controller.failure_events(),
        controller.config_toml()
    )
}

fn assert_redacted(text: &str, secret: &str) {
    assert!(!text.contains(secret), "{text}");
}

#[test]
fn hardening_changes_are_journaled_before_they_are_applied() {
    let mut platform = FakePlatform::new();
    platform.set_setting(PolicySetting::DisableTaskMgr, "admin-set");
    let mut controller = started(platform);
    controller.handle(Event::LockRequested {
        source: LockSource::App,
    });

    let events = controller.platform().apply_events();
    assert!(
        journal_is_durable_before_apply(events),
        "fake did not see a durable journal before the settings changed: {events:?}"
    );
    let durable = events.iter().find_map(|event| match event {
        softlock_core::ApplyEvent::JournalDurable { entries, settings } => {
            Some((entries, settings))
        }
        _ => None,
    });
    let (durable_entries, settings_before) = durable.expect("durable journal event");
    assert!(
        durable_entries.iter().any(|entry| {
            entry.target == "DisableTaskMgr"
                && entry.previous.as_deref() == Some("admin-set")
                && entry.new_value == "1"
        }),
        "persisted journal did not contain the change before apply: {durable_entries:?}"
    );
    assert_eq!(
        settings_before
            .iter()
            .find(|(setting, _)| *setting == PolicySetting::DisableTaskMgr)
            .and_then(|(_, value)| value.as_deref()),
        Some("admin-set"),
        "the setting changed before the journal was durable"
    );
    assert_eq!(
        controller.platform().setting(PolicySetting::DisableTaskMgr),
        Some("1")
    );
    assert!(!controller.journal().is_empty());
}

#[test]
fn arming_or_locked_failure_ends_in_os_locked_never_unlocked() {
    for step in [
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
    verify(&mut locked);
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
    platform.set_setting(PolicySetting::DisableTaskMgr, "admin-set");
    platform.set_setting(PolicySetting::DisableLockWorkstation, "0");
    let mut controller = started(platform);
    controller.handle(Event::LockRequested {
        source: LockSource::LocalApi,
    });
    assert_eq!(
        controller.platform().setting(PolicySetting::DisableTaskMgr),
        Some("1")
    );
    assert!(!controller.journal().is_empty());

    verify(&mut controller);
    assert!(matches!(controller.state(), LockState::Unlocked));
    assert!(controller.journal().is_empty());
    assert_eq!(
        controller.platform().setting(PolicySetting::DisableTaskMgr),
        Some("admin-set")
    );
    assert_eq!(
        controller
            .platform()
            .setting(PolicySetting::DisableLockWorkstation),
        Some("0")
    );

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
    assert_eq!(
        controller.platform().setting(PolicySetting::DisableTaskMgr),
        Some("admin-set")
    );
    assert_eq!(
        controller
            .platform()
            .setting(PolicySetting::DisableLockWorkstation),
        Some("0")
    );
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
        nonce: counter_nonce(1),
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

    let mut controller = started(FakePlatform::new());
    controller.handle(Event::ServiceStopRequested);
    assert!(matches!(controller.state(), LockState::Unlocked));
    assert!(
        controller
            .rejection_log()
            .iter()
            .any(|rejection| rejection.event == "ServiceStopRequested")
    );
}

#[test]
fn backoff_attempts_one_through_four_have_no_delay_then_double_from_thirty_seconds_to_fifteen_minutes_and_reset_on_success()
 {
    let mut controller = locked_started();
    for attempt in 1..=3 {
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
                prompt: PromptState::Backoff { .. },
                ..
            }
        ));
    }

    controller.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("nope"),
    });
    assert_eq!(controller.failure_count(), 4);
    assert_eq!(
        controller.backoff_delay(),
        Duration::from_secs(30),
        "the 5th attempt waits 30s, so the delay starts after the 4th failure"
    );
    assert_eq!(controller.journal().failures(), 4);
    assert!(matches!(
        controller.state(),
        LockState::Locked {
            prompt: PromptState::Backoff { failures: 4, .. },
            ..
        }
    ));

    controller.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("nope"),
    });
    assert_eq!(
        controller.failure_count(),
        4,
        "the 5th attempt is held for the delay"
    );

    advance_while_locked(
        &mut controller,
        Duration::from_secs(30) - Duration::from_millis(1),
    );
    controller.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("nope"),
    });
    assert_eq!(controller.failure_count(), 4);
    controller.handle(Event::AdvanceTime(Duration::from_millis(1)));

    let schedule = [
        (5, 60u64),
        (6, 120),
        (7, 240),
        (8, 480),
        (9, 900),
        (10, 900),
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

    verify(&mut controller);
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
    assert_eq!(beating.failure_events(), &[FailReason::HeartbeatMissed]);

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
    assert_eq!(reset.failure_events(), &[FailReason::HeartbeatMissed]);

    let mut controller = locked_started();
    controller.handle(Event::Heartbeat(HeartbeatReport {
        seq: 1,
        guard: GuardHealth {
            hook_alive: false,
            last_physical_input_ms: None,
        },
        overlay: healthy_overlay(),
    }));
    assert_eq!(controller.failure_events(), &[FailReason::GuardUnhealthy]);

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
    assert_eq!(controller.failure_events(), &[FailReason::OverlayUnhealthy]);

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
    assert_eq!(arming.failure_events(), &[FailReason::ArmTimeout]);

    let mut stepped = started(FakePlatform::new());
    stepped.drive_manually();
    stepped.handle(Event::LockRequested {
        source: LockSource::Hotkey,
    });
    stepped.handle(Event::AdvanceTime(
        ARM_STEP_TIMEOUT - Duration::from_millis(1),
    ));
    assert!(stepped.failure_events().is_empty());
    stepped.finish_arm_step();
    assert!(matches!(
        stepped.state(),
        LockState::Arming {
            step: ArmStep::Guard,
            ..
        }
    ));
    stepped.handle(Event::AdvanceTime(
        ARM_STEP_TIMEOUT - Duration::from_millis(1),
    ));
    assert!(
        stepped.failure_events().is_empty(),
        "the 2s limit starts again on each arming step"
    );
    stepped.handle(Event::AdvanceTime(Duration::from_millis(1)));
    assert_eq!(stepped.failure_events(), &[FailReason::ArmTimeout]);

    let mut replay = locked_started();
    replay.handle(Event::Heartbeat(HeartbeatReport {
        seq: replay.next_heartbeat_seq(),
        guard: healthy_guard(),
        overlay: healthy_overlay(),
    }));
    replay.handle(Event::AdvanceTime(Duration::from_millis(200)));
    replay.handle(Event::Heartbeat(HeartbeatReport {
        seq: 1,
        guard: healthy_guard(),
        overlay: healthy_overlay(),
    }));
    replay.handle(Event::AdvanceTime(Duration::from_millis(600)));
    assert_eq!(
        replay.failure_events(),
        &[FailReason::HeartbeatMissed],
        "a replayed heartbeat must not refresh the watchdog"
    );

    let mut forged = locked_started();
    forged.handle(Event::Heartbeat(HeartbeatReport {
        seq: 1,
        guard: healthy_guard(),
        overlay: healthy_overlay(),
    }));
    forged.handle(Event::AdvanceTime(Duration::from_millis(200)));
    forged.handle(Event::Heartbeat(HeartbeatReport {
        seq: 99,
        guard: healthy_guard(),
        overlay: healthy_overlay(),
    }));
    forged.handle(Event::AdvanceTime(Duration::from_millis(600)));
    assert!(
        forged.failure_events().is_empty(),
        "a strictly later sequence is healthy"
    );

    let mut stale = locked_started();
    stale.handle(Event::Heartbeat(HeartbeatReport {
        seq: 5,
        guard: healthy_guard(),
        overlay: healthy_overlay(),
    }));
    stale.handle(Event::AdvanceTime(Duration::from_millis(200)));
    stale.handle(Event::Heartbeat(HeartbeatReport {
        seq: 4,
        guard: healthy_guard(),
        overlay: healthy_overlay(),
    }));
    stale.handle(Event::AdvanceTime(Duration::from_millis(600)));
    assert_eq!(
        stale.failure_events(),
        &[FailReason::HeartbeatMissed],
        "a stale heartbeat must not refresh the watchdog"
    );
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
                token: Secret::new("token"),
            },
        ),
        (
            "request-set-softlock-pin",
            RequestKind::SetPin {
                pin: Secret::new("1234"),
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
        let json = softlock_core::message_json(11, &kind).expect("request json");
        let value: serde_json::Value = serde_json::from_str(&json).expect("request json");
        assert_eq!(value["v"], 1, "{name} is missing the version field");
        assert_eq!(value["id"], 11);
        insta::assert_json_snapshot!(name, value);

        let frame = softlock_core::encode_frame(&json).expect("frame");
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
                fallback: Fallback {
                    reason: FailReason::HeartbeatMissed,
                    at_ms: 750,
                },
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
        let json = softlock_core::message_json(11, &kind).expect("response json");
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
        let frame = softlock_core::encode_frame(&json).expect("frame");
        let error = softlock_core::decode_request_frame(&frame).expect_err("unknown major");
        assert_eq!(error, IpcError::Unsupported { id: 9 });
    }

    let unlock = softlock_core::encode_frame(r#"{"v":1,"id":4,"kind":"Unlock"}"#).expect("frame");
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

#[test]
fn apply_before_journal_flush_fails_the_order_check() {
    let mut platform = FakePlatform::new();
    platform.set_setting(PolicySetting::DisableTaskMgr, "admin-set");
    let mut journal = Journal::empty();
    journal.record(JournalEntry {
        target: "DisableTaskMgr".to_string(),
        previous: Some("admin-set".to_string()),
        new_value: "1".to_string(),
    });
    let plan = HardeningPlan {
        changes: vec![PlannedChange {
            target: "DisableTaskMgr".to_string(),
            new_value: "1".to_string(),
        }],
    };
    platform.apply_settings_before_journal(&journal, &plan);
    assert!(
        !journal_is_durable_before_apply(platform.apply_events()),
        "applying before the flush must fail the order check: {:?}",
        platform.apply_events()
    );
}

#[test]
fn revert_failure_keeps_the_journal_and_does_not_unlock() {
    let mut unlocking = locked_started();
    unlocking.platform_mut().fail_revert(true);
    verify(&mut unlocking);
    assert!(matches!(
        unlocking.state(),
        LockState::FailingClosed {
            reason: FailReason::RevertFailed
        }
    ));
    assert!(!unlocking.journal().is_empty());
    unlocking.handle(Event::OsLockConfirmed);
    assert!(matches!(
        unlocking.state(),
        LockState::OsLocked {
            after_fail: Some(FailReason::RevertFailed)
        }
    ));
    assert!(
        !unlocking.journal().is_empty(),
        "a failed revert on OS-lock confirmation keeps the journal"
    );
    assert!(!unlocking.returned_to_unlocked());

    let mut controller = locked_started();
    controller.platform_mut().fail_revert(true);
    controller.handle(Event::Session(SessionEvent::OsLocked));
    assert!(matches!(
        controller.state(),
        LockState::OsLocked { after_fail: None }
    ));
    assert!(!controller.journal().is_empty());
    controller.handle(Event::Session(SessionEvent::OsUnlocked));
    assert!(
        matches!(controller.state(), LockState::OsLocked { .. }),
        "a failed revert on sign-in must not reach Unlocked, state is {:?}",
        controller.state()
    );
    assert!(!controller.journal().is_empty());
    assert!(!controller.returned_to_unlocked());
}

#[test]
fn flush_persists_failure_count_and_locked_flag_and_recovery_restores_backoff() {
    let mut journal = Journal::empty();
    journal.mark_locked();
    journal.set_failures(4);
    assert_eq!(journal.persisted().failures(), 0);
    assert!(!journal.persisted().says_locked());
    journal.flush();
    let saved = journal.persisted();
    assert_eq!(saved.failures(), 4);
    assert!(saved.says_locked());

    let mut controller = started(FakePlatform::new());
    controller.handle(Event::ServiceStarted {
        journal: saved,
        session_active: true,
    });
    assert_eq!(controller.failure_count(), 4);
    assert_eq!(controller.journal().failures(), 4);
    let mut restarted = started(FakePlatform::new());
    restarted.handle(Event::ServiceStarted {
        journal: controller.journal().persisted(),
        session_active: true,
    });
    assert_eq!(
        restarted.failure_count(),
        4,
        "restarting from the saved journal must keep the backoff count"
    );
    assert!(matches!(
        controller.state(),
        LockState::FailingClosed { .. }
    ));
    assert!(
        controller
            .visited()
            .iter()
            .any(|state| matches!(state, LockState::Recovering))
    );
}

#[test]
fn pin_and_confirmation_token_are_redacted_and_unsupported() {
    let pin = "1234";
    let token = "confirm-token";
    let set_pin = Event::Public {
        role: Role::App,
        kind: RequestKind::SetPin {
            pin: Secret::new(pin),
        },
    };
    assert_redacted(&format!("{set_pin:?}"), pin);
    let mut controller = started(FakePlatform::new());
    controller.handle(set_pin);
    assert_eq!(controller.last_response(), Some(&ResponseKind::Unsupported));
    assert!(
        controller
            .rejection_log()
            .iter()
            .any(|rejection| rejection.event == "SetPin")
    );
    assert_redacted(&debug_output(&controller), pin);
    let confirm = Event::Public {
        role: Role::App,
        kind: RequestKind::ConfirmSensitiveChange {
            token: Secret::new(token),
        },
    };
    assert_redacted(&format!("{confirm:?}"), token);
    controller.handle(confirm);
    assert_eq!(controller.last_response(), Some(&ResponseKind::Unsupported));
    assert_redacted(&debug_output(&controller), token);
    assert_redacted(&debug_output(&controller), pin);
    controller.handle(public(RequestKind::ClearPin));
    assert_eq!(controller.last_response(), Some(&ResponseKind::Unsupported));
    controller.handle(public(RequestKind::CheckUpdate));
    assert_eq!(controller.last_response(), Some(&ResponseKind::Unsupported));
    controller.handle(public(RequestKind::InstallUpdate {
        version: "0.2.0".to_string(),
    }));
    assert_eq!(controller.last_response(), Some(&ResponseKind::Unsupported));
}

#[test]
fn submit_secret_without_a_verified_verdict_does_not_unlock() {
    let mut controller = locked_started();
    controller.platform_mut().accept_secrets(true);
    assert!(controller.platform().secrets_flag());
    controller.platform_mut().fail_revert(true);
    controller.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("ok"),
    });
    assert!(matches!(controller.state(), LockState::Locked { .. }));
    assert!(
        !controller
            .visited()
            .iter()
            .any(|state| matches!(state, LockState::Unlocking { .. }))
    );

    let mut verified = locked_started();
    verified
        .platform_mut()
        .set_secret_verdict(Verdict::Verified);
    verified.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("ok"),
    });
    assert!(
        verified
            .visited()
            .iter()
            .any(|state| matches!(state, LockState::Unlocking { .. }))
    );
    assert!(matches!(verified.state(), LockState::Unlocked));
}

#[test]
fn service_stop_during_arming_fails_closed() {
    let mut controller = started(FakePlatform::new());
    controller.drive_manually();
    controller.handle(Event::LockRequested {
        source: LockSource::Hotkey,
    });
    assert!(matches!(controller.state(), LockState::Arming { .. }));
    controller.handle(Event::ServiceStopRequested);
    assert!(matches!(
        controller.state(),
        LockState::FailingClosed {
            reason: FailReason::ServiceStop
        }
    ));
    assert!(!controller.returned_to_unlocked());
}

#[test]
fn prompt_events_do_not_clear_the_backoff_deadline() {
    let mut controller = locked_started();
    for _ in 0..4 {
        controller.handle(Event::SubmitSecret {
            method: MethodId::os_password(),
            secret: Secret::new("nope"),
        });
    }
    assert!(!controller.backoff_delay().is_zero());
    let count = controller.failure_count();
    controller.handle(Event::PromptOpened {
        method: MethodId::windows_hello(),
    });
    controller.handle(Event::PassthroughStarted {
        method: MethodId::windows_hello(),
    });
    controller.handle(Event::VerificationStarted {
        method: MethodId::os_password(),
    });
    assert!(!controller.backoff_delay().is_zero());
    controller.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("nope"),
    });
    assert_eq!(controller.failure_count(), count);
    assert!(
        controller
            .rejection_log()
            .iter()
            .any(|rejection| rejection.event == "SubmitSecret")
    );
    assert!(matches!(controller.state(), LockState::Locked { .. }));
}

#[test]
fn failing_closed_counts_wrong_secrets_and_applies_backoff() {
    let mut controller = locked_started();
    controller.handle(Event::HostExited);
    assert!(matches!(
        controller.state(),
        LockState::FailingClosed {
            reason: FailReason::HostExited
        }
    ));

    for attempt in 1..=3 {
        controller.handle(Event::SubmitSecret {
            method: MethodId::softlock_pin(),
            secret: Secret::new("nope"),
        });
        assert_eq!(controller.failure_count(), attempt);
        assert_eq!(controller.backoff_delay(), Duration::ZERO);
        assert_eq!(controller.journal().failures(), attempt);
        assert!(matches!(
            controller.state(),
            LockState::FailingClosed {
                reason: FailReason::HostExited
            }
        ));
    }

    controller.handle(Event::SubmitSecret {
        method: MethodId::softlock_pin(),
        secret: Secret::new("nope"),
    });
    assert_eq!(controller.failure_count(), 4);
    assert_eq!(controller.backoff_delay(), Duration::from_secs(30));
    assert_eq!(controller.journal().failures(), 4);
    assert!(matches!(
        controller.state(),
        LockState::FailingClosed {
            reason: FailReason::HostExited
        }
    ));

    controller.handle(Event::SubmitSecret {
        method: MethodId::softlock_pin(),
        secret: Secret::new("nope"),
    });
    assert_eq!(
        controller.failure_count(),
        4,
        "the 5th attempt is held for the delay"
    );
    assert!(
        controller
            .rejection_log()
            .iter()
            .any(|rejection| rejection.event == "SubmitSecret")
    );

    controller.handle(Event::AdvanceTime(Duration::from_secs(30)));
    assert_eq!(controller.backoff_delay(), Duration::ZERO);
    assert!(matches!(
        controller.state(),
        LockState::FailingClosed {
            reason: FailReason::HostExited
        }
    ));
    controller.handle(Event::SubmitSecret {
        method: MethodId::softlock_pin(),
        secret: Secret::new("nope"),
    });
    assert_eq!(controller.failure_count(), 5);
    assert_eq!(controller.backoff_delay(), Duration::from_secs(60));
}

#[test]
fn note_failure_flushes_the_failure_count() {
    let mut controller = locked_started();
    controller.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("nope"),
    });
    assert_eq!(controller.journal().persisted().failures(), 1);
    let before = controller.visited().len();
    controller.handle(Event::SubmitSecret {
        method: MethodId::os_password(),
        secret: Secret::new("again"),
    });
    assert!(controller.visited().len() > before);
    assert_eq!(controller.journal().persisted().failures(), 2);
}

#[test]
fn backend_instance_ids_stay_in_the_controller_journal() {
    let mut controller = locked_started();
    assert!(
        controller
            .journal()
            .entries()
            .iter()
            .any(|entry| entry.target == FakePlatform::DEVICE_INSTANCE),
        "backend journal writes were dropped: {:?}",
        controller.journal().entries()
    );
    assert_eq!(
        controller.platform().extra(FakePlatform::DEVICE_INSTANCE),
        Some("blocked")
    );
    verify(&mut controller);
    assert!(controller.journal().is_empty());
    assert_eq!(
        controller.platform().extra(FakePlatform::DEVICE_INSTANCE),
        None
    );
}

#[test]
fn only_the_app_may_hard_lock_while_locked() {
    for role in [Role::LockHost, Role::LockClient, Role::Broker] {
        let mut controller = locked_started();
        controller.handle(Event::Public {
            role,
            kind: RequestKind::HardLock,
        });
        assert_eq!(
            controller.last_response(),
            Some(&ResponseKind::Unauthorized)
        );
        assert!(
            controller
                .rejection_log()
                .iter()
                .any(|rejection| rejection.event == "HardLock"),
            "{role:?} was not logged"
        );
        assert!(matches!(controller.state(), LockState::Locked { .. }));
    }
}

#[test]
fn each_overlay_health_field_fails_closed() {
    let broken = [
        OverlayHealth {
            covered_monitors: 0,
            capture_excluded: true,
            topmost: true,
        },
        OverlayHealth {
            covered_monitors: 1,
            capture_excluded: false,
            topmost: true,
        },
        OverlayHealth {
            covered_monitors: 1,
            capture_excluded: true,
            topmost: false,
        },
    ];
    for overlay in broken {
        let mut controller = locked_started();
        controller.handle(Event::Heartbeat(HeartbeatReport {
            seq: 1,
            guard: healthy_guard(),
            overlay,
        }));
        assert_eq!(controller.failure_events(), &[FailReason::OverlayUnhealthy]);
    }
}

#[test]
fn revert_failure_on_os_lock_confirmation_keeps_the_original_reason() {
    let mut controller = locked_started();
    controller.platform_mut().fail_revert(true);
    controller.handle(Event::HostExited);
    controller.handle(Event::OsLockConfirmed);
    assert!(matches!(
        controller.state(),
        LockState::OsLocked {
            after_fail: Some(FailReason::HostExited)
        }
    ));
    assert!(!controller.journal().is_empty());
    assert!(controller.platform().guard_is_armed());
}

#[test]
fn failed_revert_after_sign_in_disarms_and_asks_for_repair() {
    let mut controller = locked_started();
    controller.platform_mut().fail_revert(true);
    controller.handle(Event::HostExited);
    controller.handle(Event::OsLockConfirmed);
    controller.handle(Event::Session(SessionEvent::OsUnlocked));
    assert!(matches!(controller.state(), LockState::OsLocked { .. }));
    assert!(!controller.journal().is_empty());
    assert!(!controller.returned_to_unlocked());
    assert!(!controller.platform().guard_is_armed());
    assert!(!controller.platform().overlay_is_shown());
    assert!(
        controller
            .notifications()
            .iter()
            .any(|notice| notice.reason == FailReason::Repair)
    );
}

#[test]
fn os_lock_uses_the_console_session_and_its_result() {
    let mut platform = FakePlatform::new();
    platform.set_console_session(SessionId::from_raw(7));
    let mut controller = started(platform);
    controller.handle(Event::LockRequested {
        source: LockSource::Hotkey,
    });
    controller.handle(Event::HostExited);
    assert_eq!(
        controller.platform().last_os_lock_session(),
        Some(SessionId::from_raw(7))
    );

    let mut platform = FakePlatform::new();
    platform.fail_os_lock(true);
    let mut controller = started(platform);
    controller.handle(Event::LockRequested {
        source: LockSource::Hotkey,
    });
    controller.handle(Event::HostExited);
    assert!(
        controller
            .rejection_log()
            .iter()
            .any(|rejection| rejection.event == "OsLock")
    );
    assert!(matches!(
        controller.state(),
        LockState::FailingClosed {
            reason: FailReason::HostExited
        }
    ));
}

#[test]
fn sensitive_config_needs_confirmation_and_is_not_applied() {
    let cases = [
        ("[unlock]\nmethods = []\n", vec!["unlock".to_string()]),
        ("unlock.methods = []\n", vec!["unlock".to_string()]),
        ("unlock = { methods = [] }\n", vec!["unlock".to_string()]),
        ("[unlock.pin]\n", vec!["unlock".to_string()]),
        ("[ unlock ]\n", vec!["unlock".to_string()]),
        ("takeover.win_l = true\n", vec!["takeover".to_string()]),
    ];
    for (toml, fields) in cases {
        let mut controller = started(FakePlatform::new());
        let revision = controller.config_revision();
        let before = controller.config_toml().to_string();
        controller.handle(public(RequestKind::PutConfig {
            toml: toml.to_string(),
            base_revision: revision,
        }));
        assert_eq!(
            controller.last_response(),
            Some(&ResponseKind::NeedsConfirmation { fields }),
            "{toml:?} was applied or named the wrong tables"
        );
        assert_eq!(controller.config_revision(), revision, "{toml:?}");
        assert_eq!(controller.config_toml(), before, "{toml:?}");
    }

    let mut controller = started(FakePlatform::new());
    let revision = controller.config_revision();
    let before = controller.config_toml().to_string();
    controller.handle(public(RequestKind::PutConfig {
        toml: "unlock = [\n".to_string(),
        base_revision: revision,
    }));
    assert_eq!(
        controller.last_response(),
        Some(&ResponseKind::NeedsConfirmation {
            fields: vec![
                "unlock".to_string(),
                "hardening".to_string(),
                "takeover".to_string(),
            ],
        }),
        "a document that does not parse must not be applied"
    );
    assert_eq!(controller.config_revision(), revision);
    assert_eq!(controller.config_toml(), before);
}

#[test]
fn failure_events_and_notifications_stay_within_the_ring() {
    let mut controller = started(FakePlatform::new());
    for _ in 0..80 {
        controller.handle(Event::LockRequested {
            source: LockSource::Hotkey,
        });
        controller.handle(Event::HostExited);
        controller.handle(Event::OsLockConfirmed);
        controller.handle(Event::Session(SessionEvent::OsUnlocked));
    }
    assert_eq!(controller.failure_events().len(), 64);
    assert_eq!(controller.notifications().len(), 64);
}

#[test]
fn hard_lock_from_the_app_while_locked_fails_closed() {
    let mut controller = locked_started();
    controller.handle(Event::Public {
        role: Role::App,
        kind: RequestKind::HardLock,
    });
    assert!(matches!(
        controller.state(),
        LockState::FailingClosed {
            reason: FailReason::HardLock
        }
    ));
    assert!(!controller.returned_to_unlocked());
}

#[test]
fn length_prefix_above_64_kib_is_rejected_before_allocation() {
    let over = ((MAX_FRAME as u32) + 1).to_le_bytes();
    assert_eq!(checked_body_len(&over), Err(IpcError::Malformed));
    assert_eq!(
        checked_body_len(&u32::MAX.to_le_bytes()),
        Err(IpcError::Malformed)
    );
    assert_eq!(checked_body_len(&32u32.to_le_bytes()).expect("fits"), 32);
}

#[test]
fn ipc_rejects_an_oversized_length_prefix() {
    let mut declared = ((MAX_FRAME as u32) + 1).to_le_bytes().to_vec();
    declared.extend_from_slice(&[b'{'; 8]);
    assert_eq!(
        softlock_core::decode_request_frame(&declared),
        Err(IpcError::Malformed)
    );
    assert_eq!(
        softlock_core::decode_request_frame(&u32::MAX.to_le_bytes()),
        Err(IpcError::Malformed)
    );
    let oversized = "x".repeat(MAX_FRAME + 1);
    assert!(softlock_core::encode_frame(&oversized).is_err());
}

#[test]
fn ipc_unknown_fields_are_rejected() {
    let frame = softlock_core::encode_frame(r#"{"v":1,"id":3,"kind":"GetStatus","extra":true}"#)
        .expect("frame");
    assert_eq!(
        softlock_core::decode_request_frame(&frame),
        Err(IpcError::Malformed)
    );
}

#[test]
fn pin_wire_kind_is_fixed() {
    let json = softlock_core::message_json(
        1,
        &RequestKind::SetPin {
            pin: Secret::new("1234"),
        },
    )
    .expect("json");
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert_eq!(value["kind"], "SetPin");
    let cleared = softlock_core::message_json(1, &RequestKind::ClearPin).expect("json");
    let cleared: serde_json::Value = serde_json::from_str(&cleared).expect("json");
    assert_eq!(cleared["kind"], "ClearPin");
}

#[test]
fn trace_and_rejections_stay_within_the_ring() {
    let mut controller = started(FakePlatform::new());
    for _ in 0..80 {
        controller.handle(Event::Verified {
            nonce: counter_nonce(1),
        });
    }
    assert_eq!(controller.rejection_log().len(), 64);
    assert!(controller.visited().len() <= 64);
}

proptest! {
    #[test]
    fn ipc_decoder_does_not_panic_on_arbitrary_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..256)
    ) {
        let _ = softlock_core::decode_request_frame(&bytes);
    }

    #[test]
    fn ipc_decoder_rejects_every_oversized_prefix(
        length in ((MAX_FRAME as u32) + 1)..=u32::MAX
    ) {
        prop_assert_eq!(
            softlock_core::decode_request_frame(&length.to_le_bytes()),
            Err(IpcError::Malformed)
        );
    }
}

#[test]
fn service_stop_requested_while_locked_fails_closed() {
    let mut controller = locked_started();
    controller.handle(Event::ServiceStopRequested);
    assert!(matches!(
        controller.state(),
        LockState::FailingClosed {
            reason: FailReason::ServiceStop
        }
    ));
    assert!(!controller.returned_to_unlocked());
}

#[test]
fn recovering_with_no_active_lock_reverts_the_journal_and_unlocks() {
    let mut platform = FakePlatform::new();
    platform.set_setting(PolicySetting::DisableTaskMgr, "1");
    let mut journal = Journal::empty();
    journal.record(JournalEntry {
        target: "DisableTaskMgr".to_string(),
        previous: Some("0".to_string()),
        new_value: "1".to_string(),
    });
    journal.mark_locked();
    journal.flush();
    let mut controller = started(platform);
    controller.handle(Event::ServiceStarted {
        journal,
        session_active: false,
    });
    assert!(
        controller
            .visited()
            .iter()
            .any(|state| matches!(state, LockState::Recovering))
    );
    assert!(matches!(controller.state(), LockState::Unlocked));
    assert!(controller.journal().is_empty());
    assert_eq!(
        controller.platform().setting(PolicySetting::DisableTaskMgr),
        Some("0")
    );
}

#[test]
fn recovering_with_an_active_session_fails_closed() {
    let mut journal = Journal::empty();
    journal.record(JournalEntry {
        target: "DisableTaskMgr".to_string(),
        previous: Some("0".to_string()),
        new_value: "1".to_string(),
    });
    journal.mark_locked();
    journal.flush();
    let mut controller = started(FakePlatform::new());
    controller.handle(Event::ServiceStarted {
        journal,
        session_active: true,
    });
    assert!(
        controller
            .visited()
            .iter()
            .any(|state| matches!(state, LockState::Recovering))
    );
    assert!(matches!(
        controller.state(),
        LockState::FailingClosed {
            reason: FailReason::SessionActive
        }
    ));
    assert!(
        !controller.journal().is_empty(),
        "fail-closed recovery keeps the journal until the OS lock is confirmed"
    );
    assert!(!controller.returned_to_unlocked());
}

/// Request the property test can clone while it shrinks. The PIN and token
/// stay plain strings until the step runs, so shrinking does not clone a `Secret`.
#[derive(Clone, Debug)]
enum GeneratedRequest {
    Hello {
        client_version: String,
    },
    GetStatus,
    Lock {
        source: LockSource,
        note: Option<String>,
    },
    HardLock,
    GetConfig,
    ValidateConfig {
        toml: String,
    },
    PutConfig {
        toml: String,
        base_revision: u64,
    },
    ConfirmSensitiveChange {
        token: String,
    },
    SetPin {
        pin: String,
    },
    ClearPin,
    Subscribe,
    CheckUpdate,
    InstallUpdate {
        version: String,
    },
}

impl GeneratedRequest {
    fn into_kind(self) -> RequestKind {
        match self {
            Self::Hello { client_version } => RequestKind::Hello { client_version },
            Self::GetStatus => RequestKind::GetStatus,
            Self::Lock { source, note } => RequestKind::Lock { source, note },
            Self::HardLock => RequestKind::HardLock,
            Self::GetConfig => RequestKind::GetConfig,
            Self::ValidateConfig { toml } => RequestKind::ValidateConfig { toml },
            Self::PutConfig {
                toml,
                base_revision,
            } => RequestKind::PutConfig {
                toml,
                base_revision,
            },
            Self::ConfirmSensitiveChange { token } => RequestKind::ConfirmSensitiveChange {
                token: Secret::new(token),
            },
            Self::SetPin { pin } => RequestKind::SetPin {
                pin: Secret::new(pin),
            },
            Self::ClearPin => RequestKind::ClearPin,
            Self::Subscribe => RequestKind::Subscribe,
            Self::CheckUpdate => RequestKind::CheckUpdate,
            Self::InstallUpdate { version } => RequestKind::InstallUpdate { version },
        }
    }
}

fn arb_request() -> impl Strategy<Value = GeneratedRequest> {
    prop_oneof![
        string_regex("[a-z0-9.]{1,8}")
            .unwrap()
            .prop_map(|client_version| { GeneratedRequest::Hello { client_version } }),
        Just(GeneratedRequest::GetStatus),
        (
            prop_oneof![
                Just(LockSource::Hotkey),
                Just(LockSource::Tray),
                Just(LockSource::App),
                Just(LockSource::LocalApi),
            ],
            prop::option::of(string_regex("[a-z]{0,12}").unwrap()),
        )
            .prop_map(|(source, note)| GeneratedRequest::Lock { source, note }),
        Just(GeneratedRequest::HardLock),
        Just(GeneratedRequest::GetConfig),
        string_regex("[a-z0-9]{0,16}")
            .unwrap()
            .prop_map(|toml| { GeneratedRequest::ValidateConfig { toml } }),
        (
            string_regex("[a-z0-9]{0,16}").unwrap(),
            any::<u8>().prop_map(u64::from),
        )
            .prop_map(|(toml, base_revision)| GeneratedRequest::PutConfig {
                toml,
                base_revision,
            }),
        string_regex("[a-z0-9]{0,8}")
            .unwrap()
            .prop_map(|token| { GeneratedRequest::ConfirmSensitiveChange { token } }),
        string_regex("[0-9]{4}")
            .unwrap()
            .prop_map(|pin| GeneratedRequest::SetPin { pin }),
        Just(GeneratedRequest::ClearPin),
        Just(GeneratedRequest::Subscribe),
        Just(GeneratedRequest::CheckUpdate),
        string_regex("[0-9.]{1,8}")
            .unwrap()
            .prop_map(|version| { GeneratedRequest::InstallUpdate { version } }),
    ]
}

#[derive(Clone, Debug)]
enum Start {
    Unlocked,
    Arming,
    LockedIdle,
    LockedOpen,
    LockedPassthrough,
    LockedVerifying,
    LockedBackoff,
    Unlocking,
    FailingClosed,
    OsLocked,
    Recovering,
}

#[derive(Clone, Debug)]
enum Step {
    Public {
        role: Role,
        request: GeneratedRequest,
    },
    Session(SessionEvent),
    VerifiedBroker,
    VerifiedStale,
    VerifiedCounter,
    PromptThenSubmit,
    SubmitSecret,
    HostExited,
    Advance,
    HeartbeatFresh,
    HeartbeatReplay,
    HeartbeatForged,
    RequestHello,
    ServiceStop,
    PromptOpened,
}

fn reach(start: Start) -> LockController<FakePlatform> {
    match start {
        Start::Unlocked => started(FakePlatform::new()),
        Start::Arming => {
            let mut controller = started(FakePlatform::new());
            controller.drive_manually();
            controller.handle(Event::LockRequested {
                source: LockSource::Hotkey,
            });
            controller
        }
        Start::LockedIdle => locked_started(),
        Start::LockedOpen => {
            let mut controller = locked_started();
            controller.handle(Event::PromptOpened {
                method: MethodId::windows_hello(),
            });
            controller
        }
        Start::LockedPassthrough => {
            let mut controller = locked_started();
            controller.handle(Event::PassthroughStarted {
                method: MethodId::windows_hello(),
            });
            controller
        }
        Start::LockedVerifying => {
            let mut controller = locked_started();
            controller.handle(Event::VerificationStarted {
                method: MethodId::os_password(),
            });
            controller
        }
        Start::LockedBackoff => {
            let mut controller = locked_started();
            for _ in 0..4 {
                controller.handle(Event::SubmitSecret {
                    method: MethodId::os_password(),
                    secret: Secret::new("nope"),
                });
            }
            controller
        }
        Start::Unlocking => {
            let mut controller = locked_started();
            controller.drive_manually();
            verify(&mut controller);
            controller
        }
        Start::FailingClosed => {
            let mut controller = locked_started();
            controller.handle(Event::HostExited);
            controller
        }
        Start::OsLocked => {
            let mut controller = locked_started();
            controller.handle(Event::HostExited);
            controller.handle(Event::OsLockConfirmed);
            controller
        }
        Start::Recovering => {
            let mut journal = Journal::empty();
            journal.mark_locked();
            journal.flush();
            let mut controller = started(FakePlatform::new());
            controller.drive_manually();
            controller.handle(Event::ServiceStarted {
                journal,
                session_active: true,
            });
            controller
        }
    }
}

fn arb_step() -> impl Strategy<Value = Step> {
    prop_oneof![
        (
            prop_oneof![
                Just(Role::App),
                Just(Role::LockClient),
                Just(Role::LockHost),
                Just(Role::Broker),
            ],
            arb_request(),
        )
            .prop_map(|(role, request)| Step::Public { role, request }),
        prop_oneof![
            Just(SessionEvent::OsLocked),
            Just(SessionEvent::OsUnlocked),
            Just(SessionEvent::Logon),
            Just(SessionEvent::Logoff),
        ]
        .prop_map(Step::Session),
        Just(Step::VerifiedBroker),
        Just(Step::VerifiedStale),
        Just(Step::VerifiedCounter),
        Just(Step::PromptThenSubmit),
        Just(Step::SubmitSecret),
        Just(Step::HostExited),
        Just(Step::Advance),
        Just(Step::HeartbeatFresh),
        Just(Step::HeartbeatReplay),
        Just(Step::HeartbeatForged),
        Just(Step::RequestHello),
        Just(Step::ServiceStop),
        Just(Step::PromptOpened),
    ]
}

fn arb_start() -> impl Strategy<Value = Start> {
    prop_oneof![
        Just(Start::Unlocked),
        Just(Start::Arming),
        Just(Start::LockedIdle),
        Just(Start::LockedOpen),
        Just(Start::LockedPassthrough),
        Just(Start::LockedVerifying),
        Just(Start::LockedBackoff),
        Just(Start::Unlocking),
        Just(Start::FailingClosed),
        Just(Start::OsLocked),
        Just(Start::Recovering),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn adversarial_events_from_any_reachable_state_need_a_genuine_verified_verdict_to_unlock(
        start in arb_start(),
        steps in prop::collection::vec(arb_step(), 0..16),
    ) {
        let mut controller = reach(start);
        let mut last_seq = None;
        for step in steps {
            let before = controller.state().clone();
            let mut genuine = false;
            match step {
                Step::Public { role, request } => {
                    controller.handle(Event::Public {
                        role,
                        kind: request.into_kind(),
                    });
                }
                Step::Session(notification) => {
                    controller.handle(Event::Session(notification));
                }
                Step::VerifiedBroker => {
                    controller.handle(Event::RequestHello);
                    if let Some(nonce) = controller.broker_nonce() {
                        genuine = matches!(before, LockState::Locked { .. });
                        controller.handle(Event::Verified { nonce });
                    }
                }
                Step::VerifiedStale => {
                    controller.handle(Event::RequestHello);
                    controller.handle(Event::Verified {
                        nonce: counter_nonce(9),
                    });
                }
                Step::VerifiedCounter => {
                    controller.handle(Event::Verified {
                        nonce: counter_nonce(1),
                    });
                }
                Step::SubmitSecret => {
                    let waiting = !controller.backoff_delay().is_zero();
                    let count = controller.failure_count();
                    controller.handle(Event::SubmitSecret {
                        method: MethodId::os_password(),
                        secret: Secret::new("secret"),
                    });
                    if waiting {
                        prop_assert_eq!(controller.failure_count(), count);
                    }
                }
                Step::PromptThenSubmit => {
                    let waiting = !controller.backoff_delay().is_zero();
                    let count = controller.failure_count();
                    controller.handle(Event::PromptOpened {
                        method: MethodId::windows_hello(),
                    });
                    controller.handle(Event::PassthroughStarted {
                        method: MethodId::windows_hello(),
                    });
                    controller.handle(Event::VerificationStarted {
                        method: MethodId::os_password(),
                    });
                    controller.handle(Event::SubmitSecret {
                        method: MethodId::os_password(),
                        secret: Secret::new("secret"),
                    });
                    if waiting {
                        prop_assert_eq!(
                            controller.failure_count(),
                            count,
                            "a prompt event must not clear the backoff deadline"
                        );
                        prop_assert!(!controller.backoff_delay().is_zero());
                    }
                }
                Step::HostExited => controller.handle(Event::HostExited),
                Step::Advance => {
                    controller.handle(Event::AdvanceTime(Duration::from_millis(50)));
                }
                Step::HeartbeatFresh => {
                    let seq = controller.next_heartbeat_seq();
                    last_seq = Some(seq);
                    controller.handle(Event::Heartbeat(HeartbeatReport {
                        seq,
                        guard: healthy_guard(),
                        overlay: healthy_overlay(),
                    }));
                }
                Step::HeartbeatReplay => {
                    controller.handle(Event::Heartbeat(HeartbeatReport {
                        seq: last_seq.unwrap_or(1),
                        guard: healthy_guard(),
                        overlay: healthy_overlay(),
                    }));
                }
                Step::HeartbeatForged => {
                    controller.handle(Event::Heartbeat(HeartbeatReport {
                        seq: last_seq.unwrap_or(0).wrapping_add(100),
                        guard: healthy_guard(),
                        overlay: healthy_overlay(),
                    }));
                }
                Step::RequestHello => controller.handle(Event::RequestHello),
                Step::ServiceStop => controller.handle(Event::ServiceStopRequested),
                Step::PromptOpened => controller.handle(Event::PromptOpened {
                    method: MethodId::windows_hello(),
                }),
            }
            let os_authenticated = matches!(before, LockState::OsLocked { .. })
                && matches!(controller.state(), LockState::Unlocked);
            let entered_unlocking = !matches!(before, LockState::Unlocking { .. })
                && matches!(controller.state(), LockState::Unlocking { .. });
            let entered_unlocked = !matches!(before, LockState::Unlocked)
                && matches!(controller.state(), LockState::Unlocked);
            prop_assert!(
                !entered_unlocking || genuine,
                "entered Unlocking from {before:?} without a genuine Verified verdict"
            );
            prop_assert!(
                !entered_unlocked || genuine || os_authenticated,
                "entered Unlocked from {before:?} without a genuine Verified verdict"
            );
        }
    }
}
