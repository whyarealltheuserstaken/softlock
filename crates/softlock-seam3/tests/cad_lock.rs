//! Seam 3. One test per Ctrl+Alt+Del and UAC row.
//!
//! Lock sets the row. Unlock restores the previous value, including an
//! administrator value. Nothing here writes the live registry.

use softlock_core::{
    Caller, JournalFault, LOCKDOWN, SettingValue, blocks_owner_sign_in,
    clears_disable_lock_workstation, commit_then_change, encode_document, lockdown_document,
    lockdown_policy, mark_workstation_clear, parse_document, reassert_write, restore_action_for,
};

const ALICE: &str = "S-1-5-21-1-2-3-1001";

fn locked_document(target: &str, previous: SettingValue) -> softlock_core::SettingsJournal {
    let rows: Vec<(&str, SettingValue)> = LOCKDOWN
        .iter()
        .map(|policy| {
            let value = if policy.target == target {
                previous
            } else {
                SettingValue::Absent
            };
            (policy.target, value)
        })
        .collect();
    lockdown_document(ALICE, &rows, 0).expect("journal")
}

fn assert_row(target: &str, locked: u32, samples: &[SettingValue]) {
    let policy = lockdown_policy(target).unwrap_or_else(|| panic!("missing {target}"));
    assert_eq!(policy.locked, locked);
    assert!(!blocks_owner_sign_in(target));
    for sample in samples {
        let doc = locked_document(target, *sample);
        assert_eq!(doc.sid, ALICE);
        let parsed = parse_document(&encode_document(&doc).expect("encode")).expect("parse");
        let entry = parsed
            .entries()
            .iter()
            .find(|entry| entry.target == target)
            .unwrap_or_else(|| panic!("{target} missing"));
        assert_eq!(entry.previous, *sample);
        assert_eq!(entry.new_value, SettingValue::Dword { value: locked });
        let restored =
            restore_action_for(target, entry.previous, entry.new_value).expect("restore");
        if *sample == entry.new_value {
            assert_eq!(restored, None, "{target} was already the locked value");
        } else {
            assert_eq!(restored, Some(*sample), "{target} did not restore exactly");
        }
    }
}

#[test]
fn sign_out_is_set_on_lock_and_restored_including_an_administrator_value() {
    assert_row(
        "NoLogoff",
        1,
        &[
            SettingValue::Absent,
            SettingValue::Dword { value: 0 },
            SettingValue::Dword { value: 1 },
        ],
    );
}

#[test]
fn change_password_is_set_on_lock_and_restored_including_an_administrator_value() {
    assert_row(
        "DisableChangePassword",
        1,
        &[
            SettingValue::Absent,
            SettingValue::Dword { value: 0 },
            SettingValue::Dword { value: 1 },
        ],
    );
}

#[test]
fn switch_user_is_set_on_lock_and_restored_including_an_administrator_value() {
    let policy = lockdown_policy("HideFastUserSwitching").expect("row");
    assert_eq!(policy.hive, softlock_core::PolicyHive::Machine);
    assert_row(
        "HideFastUserSwitching",
        1,
        &[
            SettingValue::Absent,
            SettingValue::Dword { value: 0 },
            SettingValue::Dword { value: 1 },
        ],
    );
}

#[test]
fn lock_is_set_on_lock_and_restored_including_an_administrator_value() {
    assert_row(
        "DisableLockWorkstation",
        1,
        &[
            SettingValue::Absent,
            SettingValue::Dword { value: 0 },
            SettingValue::Dword { value: 1 },
        ],
    );
    let doc = locked_document("DisableLockWorkstation", SettingValue::Dword { value: 1 });
    let marked = mark_workstation_clear(&doc).expect("clear is journaled");
    assert!(marked.workstation_clear);
    let entry = marked
        .entries()
        .iter()
        .find(|entry| entry.target == "DisableLockWorkstation")
        .expect("row");
    assert_eq!(entry.previous, SettingValue::Dword { value: 1 });
    let mut deleted = false;
    let saved = commit_then_change(
        || Err(JournalFault::NotDurable),
        || {
            deleted = true;
            Ok::<(), &str>(())
        },
    );
    assert!(saved.is_err());
    assert!(!deleted);
    let mut saved = false;
    let deleted = commit_then_change(
        || {
            saved = true;
            let _ = encode_document(&marked).expect("marked journal");
            Ok(())
        },
        || Ok::<(), &'static str>(()),
    );
    assert!(saved);
    assert!(deleted.is_ok());
}

#[test]
fn shut_down_is_set_on_lock_and_restored_including_an_administrator_value() {
    assert_row(
        "NoClose",
        1,
        &[
            SettingValue::Absent,
            SettingValue::Dword { value: 0 },
            SettingValue::Dword { value: 1 },
        ],
    );
}

#[test]
fn uac_admin_prompt_requires_credentials_and_restores_an_administrator_value() {
    let policy = lockdown_policy("ConsentPromptBehaviorAdmin").expect("row");
    assert_eq!(policy.locked, 1);
    assert_eq!(policy.hive, softlock_core::PolicyHive::Machine);
    assert_row(
        "ConsentPromptBehaviorAdmin",
        1,
        &[
            SettingValue::Absent,
            SettingValue::Dword { value: 0 },
            SettingValue::Dword { value: 2 },
            SettingValue::Dword { value: 5 },
        ],
    );
    assert!(
        lockdown_document(
            ALICE,
            &full_with(
                "ConsentPromptBehaviorAdmin",
                SettingValue::Dword { value: 6 }
            ),
            0
        )
        .is_err(),
        "a DWORD outside 0 through 5 is not written"
    );
}

#[test]
fn uac_standard_user_auto_denies_and_restores_an_administrator_value() {
    assert_row(
        "ConsentPromptBehaviorUser",
        0,
        &[
            SettingValue::Absent,
            SettingValue::Dword { value: 1 },
            SettingValue::Dword { value: 3 },
        ],
    );
    let doc = locked_document(
        "ConsentPromptBehaviorUser",
        SettingValue::Dword { value: 3 },
    );
    let entry = doc
        .entries()
        .iter()
        .find(|entry| entry.target == "ConsentPromptBehaviorUser")
        .expect("row");
    assert_eq!(
        restore_action_for(entry.target.as_str(), entry.previous, entry.new_value)
            .expect("restore"),
        Some(SettingValue::Dword { value: 3 })
    );
}

#[test]
fn a_group_policy_overwrite_is_reasserted() {
    let doc = locked_document("NoLogoff", SettingValue::Absent);
    let sign_out = doc
        .entries()
        .iter()
        .find(|entry| entry.target == "NoLogoff")
        .expect("sign out");
    assert_eq!(
        reassert_write(
            "NoLogoff",
            sign_out.new_value,
            SettingValue::Dword { value: 0 },
            false
        ),
        Some(SettingValue::Dword { value: 1 })
    );
    let user = doc
        .entries()
        .iter()
        .find(|entry| entry.target == "ConsentPromptBehaviorUser")
        .expect("standard user");
    assert_eq!(
        reassert_write(
            "ConsentPromptBehaviorUser",
            user.new_value,
            SettingValue::Dword { value: 3 },
            false
        ),
        Some(SettingValue::Dword { value: 0 })
    );
    let marked = mark_workstation_clear(&doc).expect("flag");
    assert_eq!(
        reassert_write(
            "DisableLockWorkstation",
            SettingValue::Dword { value: 1 },
            SettingValue::Absent,
            marked.workstation_clear
        ),
        None
    );
}

#[test]
fn fail_closed_clears_disable_lock_workstation_only_before_the_workstation_lock() {
    assert!(clears_disable_lock_workstation(Caller::LockHost));
    assert!(clears_disable_lock_workstation(Caller::LockHostPrimary));
    assert!(clears_disable_lock_workstation(Caller::ServiceCoordinated));
    assert!(!clears_disable_lock_workstation(Caller::Service));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let service = std::fs::read_to_string(root.join("crates/softlock-service/src/windows.rs"))
        .expect("service");
    assert!(service.contains("clears_disable_lock_workstation"));
    assert!(service.contains("clear_disable_lock_workstation_for_os_lock"));
    assert!(service.contains("reapply_disable_lock_workstation"));
    assert!(
        !service.contains("revert_durable_journal"),
        "an OS lock confirmation must not restore the owner's values"
    );
    let host = std::fs::read_to_string(root.join("crates/softlock-lockhost/src/win/failclosed.rs"))
        .expect("host");
    assert!(host.contains("clear_disable_lock_workstation_for_os_lock"));
    let reconnect = format!("{}{}", "WTSConnect", "Session");
    assert!(!host.contains(&reconnect));
    let live =
        std::fs::read_to_string(root.join("crates/softlock-lockhost/tests/fail_closed_live.rs"))
            .expect("live");
    assert!(live.contains("Duration::from_secs(1)"));
}

#[test]
fn lockdown_does_not_block_windows_hello_or_the_password() {
    for policy in LOCKDOWN {
        assert!(
            !blocks_owner_sign_in(policy.target),
            "{} would block the owner's sign-in",
            policy.target
        );
    }
    assert!(blocks_owner_sign_in("scforceoption"));
    assert!(blocks_owner_sign_in("DisabledCredentialProviders"));
    let guide = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/user-guide/ctrl-alt-del.md"),
    )
    .expect("user guide");
    for needle in [
        "NoLogoff",
        "DisableChangePassword",
        "HideFastUserSwitching",
        "DisableLockWorkstation",
        "NoClose",
        "ConsentPromptBehaviorAdmin",
        "ConsentPromptBehaviorUser",
        "Windows Hello",
        "password",
        "power",
        "network",
        "accessibility",
        "crash",
    ] {
        assert!(
            guide.contains(needle),
            "{needle} missing from the user guide"
        );
    }
}

fn full_with(target: &str, previous: SettingValue) -> Vec<(&str, SettingValue)> {
    LOCKDOWN
        .iter()
        .map(|policy| {
            let value = if policy.target == target {
                previous
            } else {
                SettingValue::Absent
            };
            (policy.target, value)
        })
        .collect()
}
