//! Seam 3. One test per Ctrl+Alt+Del and UAC row.
//!
//! Lock sets the row. Unlock restores the previous value, including an
//! administrator value. Nothing here writes the live registry.

use softlock_core::{
    Caller, JournalFault, LOCKDOWN, SettingValue, blocks_owner_sign_in,
    clears_disable_lock_workstation, commit_then_change, encode_document, lockdown_document,
    lockdown_policy, mark_workstation_clear, parse_document, reassert_write, restore_action_for,
    workstation_value_after_attempt,
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
fn no_path_leaves_disable_lock_workstation_cleared() {
    for previous in [
        SettingValue::Absent,
        SettingValue::Dword { value: 0 },
        SettingValue::Dword { value: 1 },
    ] {
        let doc = mark_workstation_clear(&locked_document("DisableLockWorkstation", previous))
            .expect("clear is journaled");
        assert!(doc.workstation_clear);
        let (settled, write) = workstation_value_after_attempt(&doc)
            .expect("settle")
            .expect("the row is still in the journal");
        assert!(
            !settled.workstation_clear,
            "the temporary clear must not survive the attempt"
        );
        assert_eq!(write, SettingValue::Dword { value: 1 });
        assert_eq!(
            reassert_write(
                "DisableLockWorkstation",
                SettingValue::Dword { value: 1 },
                SettingValue::Absent,
                settled.workstation_clear
            ),
            Some(SettingValue::Dword { value: 1 }),
            "re-assert must stop skipping once the flag is cleared"
        );
        let entry = settled
            .entries()
            .iter()
            .find(|entry| entry.target == "DisableLockWorkstation")
            .expect("row");
        assert_eq!(entry.previous, previous);
        let recovered =
            restore_action_for("DisableLockWorkstation", previous, write).expect("recovery");
        if previous == write {
            assert_eq!(recovered, None);
        } else {
            assert_eq!(recovered, Some(previous));
        }
    }
    // A disconnect after the host died did not clear the value itself.
    // The journaled clear is still put back.
    assert!(!clears_disable_lock_workstation(Caller::Service));
}

#[test]
fn fail_closed_clears_disable_lock_workstation_only_before_the_workstation_lock() {
    assert!(clears_disable_lock_workstation(Caller::LockHost));
    assert!(clears_disable_lock_workstation(Caller::LockHostPrimary));
    assert!(clears_disable_lock_workstation(Caller::ServiceCoordinated));
    assert!(!clears_disable_lock_workstation(Caller::Service));
    let doc = locked_document("DisableLockWorkstation", SettingValue::Dword { value: 0 });
    let marked = mark_workstation_clear(&doc).expect("journaled before the delete");
    assert_eq!(
        marked
            .entries()
            .iter()
            .find(|entry| entry.target == "DisableLockWorkstation")
            .expect("row")
            .previous,
        SettingValue::Dword { value: 0 }
    );
    assert!(softlock_core::within_budget(
        std::time::Duration::from_millis(999)
    ));
    assert!(!softlock_core::within_budget(
        std::time::Duration::from_secs(1)
    ));
}

#[test]
fn the_lock_host_does_not_link_the_service() {
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../softlock-lockhost/Cargo.toml"),
    )
    .expect("lock host manifest");
    let mut in_product_deps = false;
    for line in manifest.lines() {
        if line.starts_with('[') {
            in_product_deps = (line.starts_with("[dependencies]")
                || line.contains(".dependencies]"))
                && !line.contains("dev-dependencies");
        }
        if in_product_deps {
            assert!(
                !line.contains("softlock-service"),
                "the lock host links the service: {line}"
            );
        }
    }
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
    assert!(blocks_owner_sign_in("AllowDomainPINLogon"));
    assert!(blocks_owner_sign_in("DisablePasswordUnlock"));
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
