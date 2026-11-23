//! Hosted-runner proof that a kill between the journal and the registry,
//! and a kill after the change, restores DisableTaskMgr exactly.
//! A normal `cargo test` returns immediately. The settings-journal workflow
//! sets `SOFTLOCK_JOURNAL_LIVE` and builds with `live-fail-closed`.

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use softlock_core::SERVICE_NAME;

const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\System";
const EXPLORER: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer";
const MACHINE: &str = r"HKLM\Software\Microsoft\Windows\CurrentVersion\Policies\System";

struct Saved {
    key: &'static str,
    name: &'static str,
    value: String,
}

fn rows() -> Vec<Saved> {
    [
        snap(KEY, "DisableTaskMgr"),
        snap(EXPLORER, "NoLogoff"),
        snap(KEY, "DisableChangePassword"),
        snap(MACHINE, "HideFastUserSwitching"),
        snap(KEY, "DisableLockWorkstation"),
        snap(EXPLORER, "NoClose"),
        snap(MACHINE, "ConsentPromptBehaviorAdmin"),
        snap(MACHINE, "ConsentPromptBehaviorUser"),
    ]
    .into_iter()
    .collect()
}

fn snap(key: &'static str, name: &'static str) -> Saved {
    Saved {
        key,
        name,
        value: query_value(key, name),
    }
}

#[test]
fn live_settings_journal() {
    let Ok(case) = std::env::var("SOFTLOCK_JOURNAL_LIVE") else {
        return;
    };
    match softlock_windows::runner_may_lock() {
        Ok(()) => {}
        Err(reason) => panic!("refusing the settings journal on this machine: {reason}"),
    }
    if installed_service_running() {
        panic!("refusing while the installed service is running");
    }
    fs::create_dir_all(guard_dir()).expect("temp");
    fs::create_dir_all(artifact_dir()).expect("artifact");
    ensure_service().expect("LocalSystem service");
    let mut guard = Guard {
        rows: rows(),
        reported: false,
        config_path: None,
        config_dacl: None,
        config_bytes: None,
    };
    let outcome = match case.as_str() {
        "restore" => restore(&mut guard),
        "recover-active" => recover_active(&mut guard),
        "clock" => clock_shift_stays_locked(&mut guard),
        #[cfg(feature = "live-fail-closed")]
        "sign-in" => sign_in_then_the_next_lock_unlocks(&mut guard),
        #[cfg(not(feature = "live-fail-closed"))]
        "sign-in" => Err("the sign-in case is compiled with live-fail-closed".to_string()),
        other => Err(format!("unknown case {other}")),
    };
    match outcome {
        Ok(()) => {
            capture();
            guard.reported = true;
            write_result(r#"{"ok":true}"#);
        }
        Err(reason) => {
            capture();
            guard.reported = true;
            write_result(&format!(r#"{{"ok":false,"reason":"{reason}"}}"#));
            panic!("{reason}");
        }
    }
}

fn restore(guard: &mut Guard) -> Result<(), String> {
    // The all-rows lock has to run while this console session is still
    // active. A later hard stop of an unpaused lock can fail closed and
    // reject the next lock.
    every_row_restores(guard)?;
    for order in KILL_ORDERS {
        for previous in ["absent", "0", "1"] {
            kill_before_apply(guard, previous, order)?;
        }
        kill_after_apply(guard, order)?;
    }
    reassert_and_repair(guard)?;
    config_acl_round_trip(guard)?;
    Ok(())
}

#[derive(Clone, Copy)]
enum KillOrder {
    ServiceFirst,
    HostFirst,
    Both,
}

const KILL_ORDERS: [KillOrder; 3] = [
    KillOrder::ServiceFirst,
    KillOrder::HostFirst,
    KillOrder::Both,
];

fn order_name(order: KillOrder) -> &'static str {
    match order {
        KillOrder::ServiceFirst => "service-first",
        KillOrder::HostFirst => "host-first",
        KillOrder::Both => "both",
    }
}

fn config_acl_round_trip(guard: &mut Guard) -> Result<(), String> {
    #[cfg(not(feature = "live-fail-closed"))]
    {
        let _ = guard;
        Err("the config ACL proof needs live-fail-closed".to_string())
    }
    #[cfg(feature = "live-fail-closed")]
    {
        // The earlier proofs disconnect the console. A lock from this process
        // is refused until that session is the active console again.
        align_console()?;
        let path = softlock_service::owner_config_file().map_err(|reason| reason.to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| "config dir".to_string())?;
        }
        // The Service has to create this file. A copy the console user already
        // had is put back when the guard drops.
        if path.exists() {
            guard.config_bytes = Some(fs::read(&path).unwrap_or_default());
            force_remove(&path);
            if path.exists() {
                return Err("could not remove the existing config file".to_string());
            }
        }
        let pause = guard_dir().join("pause-config");
        let _ = fs::remove_file(&pause);
        spawn_service(guard, Some("before"), &pause, true)?;
        wait_pipe()?;
        let created = wait_for_config(&path);
        if !created {
            let owner = softlock_service::testing_owner(&path)
                .map_err(|error| error.to_string())
                .unwrap_or_else(|error| error);
            let dacl = softlock_service::testing_dacl(&path)
                .map_err(|error| error.to_string())
                .unwrap_or_else(|error| error);
            return Err(format!(
                "config file before the ACL change exists={} write={} journal={} owner={} dacl={}",
                path.is_file(),
                write_state(&path),
                journal_ready(&journal_file()),
                owner,
                dacl
            ));
        }
        guard.config_path = Some(path.clone());
        let body = fs::read_to_string(&path).map_err(|_| "config read".to_string())?;
        if !body.contains("schema_version") {
            return Err("the service did not write the config file".to_string());
        }
        let original = softlock_service::testing_dacl(&path).map_err(|_| "dacl".to_string())?;
        let owner = softlock_service::testing_owner(&path).map_err(|_| "owner".to_string())?;
        let mine = console_user_sid()?;
        if !owner.eq_ignore_ascii_case(&mine) && !explicit_user_dacl(&original, &mine) {
            return Err(format!(
                "config owner {owner} is not the console user {mine} and the DACL is not an explicit grant"
            ));
        }
        guard.config_dacl = Some(original.clone());
        fs::write(&pause, "go").map_err(|_| "pause".to_string())?;
        if !wait_until(Duration::from_secs(20), || owner_rights_denied(&path)) {
            return Err(format!(
                "the console user kept a write right while locked: {}",
                owner_rights(&path)
            ));
        }
        // The locked DACL no longer lets the console user write it. The
        // external change has to come from LocalSystem, which still has full
        // control.
        restore_dacl_as_system(&path, &original)?;
        if !wait_until(Duration::from_secs(12), || owner_rights_denied(&path)) {
            return Err(format!(
                "an external DACL change was not re-asserted: {}",
                owner_rights(&path)
            ));
        }
        // Deny SYSTEM WRITE_DAC and allow the console user write. Access
        // denied must not count as held. The service puts the locked DACL
        // back, or the user would be able to edit the file while locked.
        let hostile = format!("D:P(D;;WD;;;SY)(A;;FA;;;{mine})(A;;FR;;;OW)");
        restore_dacl_as_system(&path, &hostile)?;
        if !wait_until(Duration::from_secs(15), || owner_rights_denied(&path)) {
            let dacl = softlock_service::testing_dacl(&path)
                .map_err(|error| error.to_string())
                .unwrap_or_else(|error| error);
            let owner = softlock_service::testing_owner(&path)
                .map_err(|error| error.to_string())
                .unwrap_or_else(|error| error);
            let log = fs::read_to_string(guard_dir().join("service.log")).unwrap_or_default();
            let start = log.len().saturating_sub(700);
            let start = (start..=log.len())
                .find(|index| log.is_char_boundary(*index))
                .unwrap_or(log.len());
            return Err(format!(
                "a DACL that denies SYSTEM WRITE_DAC left the console user able to write (owner {owner} dacl {dacl} log {})",
                &log[start..]
            ));
        }
        hard_stop(guard)?;
        let _ = wait_until(Duration::from_secs(5), || !host_running());
        spawn_service(guard, None, &pause, true)?;
        let restored = wait_until(Duration::from_secs(20), || {
            softlock_service::testing_dacl(&path)
                .ok()
                .is_some_and(|current| current == original)
                && owner_rights_allowed(&path)
        });
        hard_stop(guard)?;
        if !restored {
            return Err("boot after the host was gone did not restore the config ACL".to_string());
        }
        Ok(())
    }
}

#[cfg(feature = "live-fail-closed")]
fn wait_for_config(path: &Path) -> bool {
    let started = Instant::now();
    let mut next_lock = Instant::now();
    while started.elapsed() < Duration::from_secs(40) {
        if path.is_file() && owner_rights_allowed(path) && journal_ready(&journal_file()) {
            return true;
        }
        // A paused lock already saved the journal. Asking again would block
        // behind that pause.
        if Instant::now() >= next_lock && !journal_ready(&journal_file()) {
            lock_in_background();
            next_lock = Instant::now() + Duration::from_secs(3);
        }
        thread::sleep(Duration::from_millis(100));
    }
    path.is_file() && owner_rights_allowed(path) && journal_ready(&journal_file())
}

#[cfg(feature = "live-fail-closed")]
fn align_console() -> Result<(), String> {
    if session_aligned() {
        return Ok(());
    }
    let api = format!("{}{}{}", "WTS", "Connect", "SessionW");
    let script = guard_dir().join("align.ps1");
    let log = guard_dir().join("align.txt");
    let body = format!(
        "$sig = @'\nusing System.Runtime.InteropServices;\npublic static class SoftlockAlign {{\n  [DllImport(\"wtsapi32.dll\", CharSet=CharSet.Unicode)]\n  public static extern bool {api}(uint session, uint target, string password, bool wait);\n}}\n'@\nAdd-Type -TypeDefinition $sig\n$id = (Get-Process -Id {pid}).SessionId\n$result = [SoftlockAlign]::{api}([uint32]$id, [uint32]::MaxValue, '', $true)\nSet-Content -Path '{log}' -Value $result\n",
        api = api,
        pid = std::process::id(),
        log = log.display(),
    );
    fs::write(&script, body).map_err(|_| "align script".to_string())?;
    let powershell = std::env::var("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let bin = format!(
        "{} -NoProfile -ExecutionPolicy Bypass -File {}",
        powershell.display(),
        script.display()
    );
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", "sl-align", "/F"])
        .status();
    let created = Command::new("schtasks")
        .args([
            "/Create", "/TN", "sl-align", "/TR", &bin, "/SC", "ONCE", "/ST", "00:00", "/RU",
            "SYSTEM", "/RL", "HIGHEST", "/F",
        ])
        .status()
        .map_err(|_| "align task".to_string())?;
    if !created.success() {
        return Err("align task was not created".to_string());
    }
    let _ = Command::new("schtasks")
        .args(["/Run", "/TN", "sl-align"])
        .status();
    let aligned = wait_until(Duration::from_secs(20), session_aligned);
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", "sl-align", "/F"])
        .status();
    if aligned {
        Ok(())
    } else {
        let note = fs::read_to_string(&log).unwrap_or_default();
        Err(format!("console session was not the test session ({note})"))
    }
}

#[cfg(feature = "live-fail-closed")]
fn restore_dacl_as_system(path: &Path, sddl: &str) -> Result<(), String> {
    let script = guard_dir().join("restore-dacl.ps1");
    let log = guard_dir().join("restore-dacl.log");
    let sddl = sddl.replace('\'', "''");
    let path_text = path.display().to_string().replace('\'', "''");
    let body = format!(
        r#"$ErrorActionPreference = 'Stop'
$log = '{log}'
try {{
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class SoftlockAcl {{
  [DllImport("advapi32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
  public static extern int SetNamedSecurityInfo(string name, int type, int security, IntPtr owner, IntPtr group, IntPtr dacl, IntPtr sacl);
  [DllImport("advapi32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
  public static extern bool ConvertStringSecurityDescriptorToSecurityDescriptor(string sddl, int revision, out IntPtr sd, out uint length);
  [DllImport("advapi32.dll", SetLastError=true)]
  public static extern bool GetSecurityDescriptorDacl(IntPtr sd, out bool present, out IntPtr dacl, out bool defaulted);
  [DllImport("advapi32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
  public static extern int GetNamedSecurityInfo(string name, int type, int security, out IntPtr owner, out IntPtr group, out IntPtr dacl, out IntPtr sacl, out IntPtr sd);
  [DllImport("advapi32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
  public static extern bool ConvertSecurityDescriptorToStringSecurityDescriptor(IntPtr sd, int revision, int security, out IntPtr text, out uint length);
}}
'@
  $sd = [IntPtr]::Zero
  $length = [uint32]0
  $converted = [SoftlockAcl]::ConvertStringSecurityDescriptorToSecurityDescriptor('{sddl}', 1, [ref]$sd, [ref]$length)
  if (-not $converted) {{ throw "convert $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }}
  $present = $false
  $dacl = [IntPtr]::Zero
  $defaulted = $false
  [void][SoftlockAcl]::GetSecurityDescriptorDacl($sd, [ref]$present, [ref]$dacl, [ref]$defaulted)
  if (-not $present) {{ throw 'no dacl' }}
  $status = [SoftlockAcl]::SetNamedSecurityInfo('{path_text}', 1, 0x80000004, [IntPtr]::Zero, [IntPtr]::Zero, $dacl, [IntPtr]::Zero)
  if ($status -ne 0) {{ throw "set $status" }}
  $owner = [IntPtr]::Zero
  $group = [IntPtr]::Zero
  $nowDacl = [IntPtr]::Zero
  $sacl = [IntPtr]::Zero
  $nowSd = [IntPtr]::Zero
  $read = [SoftlockAcl]::GetNamedSecurityInfo('{path_text}', 1, 4, [ref]$owner, [ref]$group, [ref]$nowDacl, [ref]$sacl, [ref]$nowSd)
  if ($read -ne 0) {{ throw "read $read" }}
  $rendered = [IntPtr]::Zero
  $renderedLen = [uint32]0
  $shown = [SoftlockAcl]::ConvertSecurityDescriptorToStringSecurityDescriptor($nowSd, 1, 4, [ref]$rendered, [ref]$renderedLen)
  if (-not $shown) {{ throw "render $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }}
  $text = [Runtime.InteropServices.Marshal]::PtrToStringUni($rendered)
  "readback=$text" | Set-Content -LiteralPath $log
}} catch {{
  $_ | Out-String | Set-Content -LiteralPath $log
  exit 1
}}
"#,
        log = log.display(),
        sddl = sddl,
        path_text = path_text,
    );
    fs::write(&script, body).map_err(|_| "dacl script".to_string())?;
    let powershell = std::env::var("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let bin = format!(
        "{} -NoProfile -ExecutionPolicy Bypass -File {}",
        powershell.display(),
        script.display()
    );
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", "sl-dacl", "/F"])
        .status();
    let created = Command::new("schtasks")
        .args([
            "/Create", "/TN", "sl-dacl", "/TR", &bin, "/SC", "ONCE", "/ST", "00:00", "/RU",
            "SYSTEM", "/RL", "HIGHEST", "/F",
        ])
        .status()
        .map_err(|_| "dacl task".to_string())?;
    if !created.success() {
        return Err("dacl task was not created".to_string());
    }
    let started = Command::new("schtasks")
        .args(["/Run", "/TN", "sl-dacl"])
        .status()
        .map_err(|_| "dacl task".to_string())?;
    let recorded = started.success()
        && wait_until(Duration::from_secs(15), || {
            fs::read_to_string(&log)
                .ok()
                .is_some_and(|text| text.contains("readback="))
        });
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", "sl-dacl", "/F"])
        .status();
    let note = fs::read_to_string(&log).unwrap_or_default();
    // The locked DACL still says FA for SYSTEM. A restored file has no FR ACE.
    let granted = note.lines().any(|line| {
        let Some(body) = line.trim().strip_prefix("readback=") else {
            return false;
        };
        body.contains("FA") && !body.contains("FR")
    });
    if recorded && granted {
        Ok(())
    } else {
        Err(format!("dacl task did not restore write ({note})"))
    }
}

#[cfg(feature = "live-fail-closed")]
fn session_aligned() -> bool {
    use windows::Win32::System::RemoteDesktop::{
        ProcessIdToSessionId, WTSGetActiveConsoleSessionId,
    };
    use windows::Win32::System::Threading::GetCurrentProcessId;
    unsafe {
        let console = WTSGetActiveConsoleSessionId();
        let mut mine = 0u32;
        if ProcessIdToSessionId(GetCurrentProcessId(), &mut mine).is_err() {
            return false;
        }
        mine != 0 && mine != u32::MAX && mine == console
    }
}

#[cfg(feature = "live-fail-closed")]
fn owner_rights(path: &Path) -> String {
    softlock_service::owner_lock_rights()
        .iter()
        .map(|right| {
            let state = match softlock_service::owner_right_denied(path, right.access) {
                Ok(true) => "denied",
                Ok(false) => "allowed",
                Err(error) => return format!("{}={error}", right.name),
            };
            format!("{}={state}", right.name)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(feature = "live-fail-closed")]
fn owner_rights_denied(path: &Path) -> bool {
    softlock_service::owner_lock_rights().iter().all(|right| {
        matches!(
            softlock_service::owner_right_denied(path, right.access),
            Ok(true)
        )
    })
}

#[cfg(feature = "live-fail-closed")]
fn owner_rights_allowed(path: &Path) -> bool {
    softlock_service::owner_lock_rights().iter().all(|right| {
        matches!(
            softlock_service::owner_right_denied(path, right.access),
            Ok(false)
        )
    })
}

#[cfg(feature = "live-fail-closed")]
fn force_remove(path: &Path) {
    let text = path.display().to_string();
    let _ = fs::remove_file(path);
    if path.exists() {
        let _ = Command::new("takeown").args(["/f", &text, "/a"]).status();
        let _ = Command::new("icacls").args([&text, "/reset"]).status();
        let _ = Command::new("icacls")
            .args([&text, "/grant", "Everyone:F"])
            .status();
        let _ = fs::remove_file(path);
    }
}

#[cfg(feature = "live-fail-closed")]
fn write_state(path: &Path) -> String {
    match softlock_service::owner_write_denied(path) {
        Ok(true) => "denied".to_string(),
        Ok(false) => "allowed".to_string(),
        Err(error) => error.to_string(),
    }
}

#[cfg(feature = "live-fail-closed")]
fn explicit_user_dacl(dacl: &str, sid: &str) -> bool {
    if !dacl.starts_with("D:P") {
        return false;
    }
    let upper = dacl.to_ascii_uppercase();
    if upper.contains(&sid.to_ascii_uppercase()) {
        return true;
    }
    // The built-in administrator is printed as LA.
    sid.rsplit('-').next() == Some("500") && upper.contains(";;;LA)")
}

#[cfg(feature = "live-fail-closed")]
fn console_user_sid() -> Result<String, String> {
    let output = Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .map_err(|_| "whoami".to_string())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let sid = text
        .split(',')
        .nth(1)
        .unwrap_or("")
        .trim()
        .trim_matches('"')
        .to_string();
    if sid.starts_with("S-1-") {
        Ok(sid)
    } else {
        Err(format!("whoami did not name a SID: {text}"))
    }
}

fn kill_before_apply(guard: &mut Guard, previous: &str, order: KillOrder) -> Result<(), String> {
    realign()?;
    set_taskmgr(previous)?;
    let pause = guard_dir().join(format!("pause-{}-{previous}", order_name(order)));
    let _ = fs::remove_file(&pause);
    let before = journal_text();
    spawn_service(guard, Some("before"), &pause, true)?;
    wait_pipe()?;
    lock_in_background();
    let journal = journal_file();
    let saw = wait_until(Duration::from_secs(20), || {
        journal_ready(&journal) && journal_text() != before && query_taskmgr() == previous
    });
    if !saw || !journal_ready(&journal) {
        return Err(format!(
            "journal was not durable before the {previous} change ({})",
            order_name(order)
        ));
    }
    // The service is blocked after the flush. A stop file is not read until
    // that pause ends, which would apply the value. Kill it.
    let baseline = log_baseline();
    stop_in_order(guard, order)?;
    if query_taskmgr() != previous {
        return Err(format!(
            "kill changed DisableTaskMgr from {previous} ({})",
            order_name(order)
        ));
    }
    let _ = wait_until(Duration::from_secs(5), || !host_running());
    // Leave the console where the kill left it. Attaching here would turn a
    // confirmed disconnect back into an active desktop before recovery runs.
    spawn_service(guard, None, &pause, false)?;
    wait_os_held(
        previous,
        &format!("before {previous} {}", order_name(order)),
        &baseline,
    )?;
    hard_stop(guard)?;
    clear_os_hold()?;
    realign()?;
    if query_taskmgr() != previous {
        return Err(format!(
            "restored value was not {previous} ({})",
            order_name(order)
        ));
    }
    Ok(())
}

fn kill_after_apply(guard: &mut Guard, order: KillOrder) -> Result<(), String> {
    realign()?;
    set_taskmgr("absent")?;
    let pause = guard_dir().join(format!("pause-after-{}", order_name(order)));
    let _ = fs::remove_file(&pause);
    // Hold the service after the registry write. Killing both processes
    // leaves the journal durable. The restart is the same boot. It must
    // OS-lock or leave the session OS-held, and must not end Unlocked while
    // the session is active. Settings are restored once that lock is
    // confirmed (SPEC section 7).
    spawn_service(guard, Some("after"), &pause, true)?;
    wait_pipe()?;
    lock_in_background();
    let applied = wait_until(Duration::from_secs(20), || query_taskmgr() == "1");
    if !applied {
        return Err(format!(
            "DisableTaskMgr was not set to 1 ({})",
            order_name(order)
        ));
    }
    let baseline = log_baseline();
    stop_in_order(guard, order)?;
    let _ = wait_until(Duration::from_secs(5), || !host_running());
    spawn_service(guard, None, &pause, false)?;
    wait_os_held("absent", &format!("after {}", order_name(order)), &baseline)?;
    // The owner coming back attaches the console again. The product never
    // reconnects a session. The restored value has to still be absent.
    realign()?;
    let still = wait_until(Duration::from_secs(10), || query_taskmgr() == "absent");
    let status = transact(&pipe_name(), status_body()).unwrap_or_default();
    let state = status_state(&status).unwrap_or_default();
    hard_stop(guard)?;
    clear_os_hold()?;
    if !still || query_taskmgr() != "absent" {
        return Err(format!(
            "settings were not the absent original after the owner returned (state {state}, {})",
            order_name(order)
        ));
    }
    Ok(())
}

fn wait_os_held(expected: &str, label: &str, baseline: &str) -> Result<(), String> {
    let mut ended_unlocked = false;
    let mut last_state = String::new();
    let held = wait_until(Duration::from_secs(25), || {
        let status = transact(&pipe_name(), status_body()).unwrap_or_default();
        let state = status_state(&status).unwrap_or_default();
        last_state = state.clone();
        let os_held = session_os_held();
        // Unlocked with no confirmed OS lock is a fail-open. Unlocked after
        // `os-lock result=confirmed` was written during this wait, with the
        // original value back, is the owner (or the runner's auto-logon)
        // returning to this logon. The service clears OsLocked itself. A
        // reconnect on a real machine asks for credentials unless auto-logon
        // is on. A missing confirmation line cannot take this branch.
        if state == "Unlocked" && !os_held {
            if confirmed_since(baseline, expected) {
                return true;
            }
            ended_unlocked = true;
            return false;
        }
        let reported = state == "OsLocked" || os_held;
        reported && query_taskmgr() == expected && !journal_ready(&journal_file())
    });
    if !held && ended_unlocked {
        return Err(format!(
            "{label} ended Unlocked with no OS lock (value {})",
            query_taskmgr()
        ));
    }
    if !held {
        return Err(format!(
            "{label} did not OS-lock or disconnect and restore (state {last_state}, value {})",
            query_taskmgr()
        ));
    }
    Ok(())
}

fn clock_shift_stays_locked(guard: &mut Guard) -> Result<(), String> {
    let original = powershell("(Get-Date).ToString('o')")?;
    let _restore = ClockRestore {
        original: original.clone(),
    };
    for order in KILL_ORDERS {
        clock_one_order(guard, order)?;
    }
    Ok(())
}

fn clock_one_order(guard: &mut Guard, order: KillOrder) -> Result<(), String> {
    realign()?;
    set_taskmgr("absent")?;
    let pause = guard_dir().join(format!("pause-clock-{}", order_name(order)));
    let _ = fs::remove_file(&pause);
    spawn_service(guard, None, &pause, true)?;
    wait_pipe()?;
    lock_in_background();
    if !wait_until(Duration::from_secs(20), || query_taskmgr() == "1") {
        return Err(format!(
            "the clock case did not set DisableTaskMgr ({})",
            order_name(order)
        ));
    }
    // A large forward jump. BootTime moves with the wall clock. Unbiased
    // uptime does not. The live service has to stay locked across the jump.
    // The restart, not the kill, is the recovery under test.
    powershell("[void](Set-Date -Date (Get-Date).AddDays(3))")?;
    thread::sleep(Duration::from_secs(2));
    let live = transact(&pipe_name(), status_body()).unwrap_or_default();
    let live_state = status_state(&live).unwrap_or_default();
    if live_state == "Unlocked" || query_taskmgr() != "1" {
        return Err(format!(
            "a clock change moved the live lock (state {live_state}, value {}, {})",
            query_taskmgr(),
            order_name(order)
        ));
    }
    let baseline = log_baseline();
    stop_in_order(guard, order)?;
    let _ = wait_until(Duration::from_secs(5), || !host_running());
    spawn_service(guard, None, &pause, false)?;
    wait_os_held("absent", &format!("clock {}", order_name(order)), &baseline)?;
    hard_stop(guard)?;
    clear_os_hold()?;
    Ok(())
}

struct ClockRestore {
    original: String,
}

impl Drop for ClockRestore {
    fn drop(&mut self) {
        let script = format!(
            "[void](Set-Date -Date '{}')",
            self.original.replace('\'', "")
        );
        let _ = powershell(&script);
    }
}

fn powershell(script: &str) -> Result<String, String> {
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|_| "powershell".to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!(
            "powershell failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn reassert_and_repair(guard: &mut Guard) -> Result<(), String> {
    set_taskmgr("absent")?;
    let pause = guard_dir().join("pause-reassert");
    spawn_service(guard, None, &pause, true)?;
    wait_pipe()?;
    transact(&pipe_name(), lock_now()).map_err(|error| format!("lock: {error}"))?;
    if !wait_until(Duration::from_secs(20), || query_taskmgr() == "1") {
        return Err("lock did not set DisableTaskMgr".to_string());
    }
    let _ = Command::new("reg")
        .args(["delete", KEY, "/v", "DisableTaskMgr", "/f"])
        .status();
    if !wait_until(Duration::from_secs(8), || query_taskmgr() == "1") {
        return Err("cleared DisableTaskMgr was not re-asserted within 5 s".to_string());
    }
    let _ = set_value(EXPLORER, "NoLogoff", "absent");
    let _ = set_value(MACHINE, "ConsentPromptBehaviorUser", "3");
    if !wait_until(Duration::from_secs(8), || {
        query_value(EXPLORER, "NoLogoff") == "1"
            && query_value(MACHINE, "ConsentPromptBehaviorUser") == "0"
    }) {
        return Err("a Group Policy overwrite was not re-asserted".to_string());
    }
    let (repair, _) = repair_settings()?;
    if repair == 0 {
        return Err("repair ran while locked".to_string());
    }
    if query_taskmgr() != "1" {
        return Err("repair changed DisableTaskMgr while locked".to_string());
    }
    hard_stop(guard)?;
    // The disconnect can leave no console session. Repair then fail-closes
    // instead of restoring. Attach the session first; the host is already gone.
    #[cfg(feature = "live-fail-closed")]
    align_console()?;
    // Sign-in recovery runs as LocalSystem. The console user no longer has
    // file write, and on some runners cannot set the DACL either.
    let mut first = (1, String::new());
    let mut restored = false;
    for _ in 0..8 {
        first = repair_as_system()?;
        if first.0 == 0 {
            restored = true;
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    let second = repair_as_system()?;
    if !restored || second.0 != 0 {
        return Err(format!(
            "repair after the kill returned {} ({}) then {} ({})",
            first.0, first.1, second.0, second.1
        ));
    }
    if query_taskmgr() != "absent" {
        return Err("repair did not restore the absent value".to_string());
    }
    Ok(())
}

#[cfg(feature = "live-fail-closed")]
fn sign_in_then_the_next_lock_unlocks(guard: &mut Guard) -> Result<(), String> {
    realign()?;
    set_taskmgr("absent")?;
    seed_system_pin(&guard_dir().join("pin"))?;
    let pause = guard_dir().join("pause-sign-in");
    let _ = fs::remove_file(&pause);
    spawn_service(guard, None, &pause, true)?;
    wait_pipe()?;
    let locked = transact(&pipe_name(), lock_now()).map_err(|error| format!("lock: {error}"))?;
    if !locked.contains("Accepted") {
        return Err(format!("first lock was not accepted: {locked}"));
    }
    if !wait_until(Duration::from_secs(20), || {
        query_taskmgr() == "1"
            && status_state(&transact(&pipe_name(), status_body()).unwrap_or_default()).as_deref()
                == Some("Locked")
    }) {
        return Err("first lock did not hold".to_string());
    }
    // The host dies. The service process stays up and confirms the disconnect.
    // The runner's auto-logon can bring this logon back before the next
    // sample. That is the sign-in, so Unlocked after the confirmation is
    // the same success as a disconnect we still have to reconnect.
    let baseline = log_baseline();
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "softlock-lockhost.exe"])
        .status();
    let held = wait_until(Duration::from_secs(25), || {
        session_os_held()
            || (status_state(&transact(&pipe_name(), status_body()).unwrap_or_default()).as_deref()
                == Some("Unlocked")
                && confirmed_since(&baseline, "absent"))
    });
    let signed_back = !session_os_held()
        && status_state(&transact(&pipe_name(), status_body()).unwrap_or_default()).as_deref()
            == Some("Unlocked")
        && confirmed_since(&baseline, "absent");
    if !held && !signed_back {
        let state = transact(&pipe_name(), status_body()).unwrap_or_default();
        return Err(format!(
            "the host death did not disconnect the console: {state}"
        ));
    }
    if !signed_back {
        // Same logon, same session, back on the console. The service clears
        // OsLocked itself once that sign-in is the console. No repair command.
        align_console()?;
        if !wait_until(Duration::from_secs(25), || {
            status_state(&transact(&pipe_name(), status_body()).unwrap_or_default()).as_deref()
                == Some("Unlocked")
                && confirmed_since(&baseline, "absent")
        }) {
            let state = transact(&pipe_name(), status_body()).unwrap_or_default();
            return Err(format!("sign-in did not clear OsLocked: {state}"));
        }
    }
    // The disconnected session's host was dropped. The replacement has to
    // be ready before this lock, or the arm wait fail-closes.
    thread::sleep(Duration::from_secs(2));
    let again =
        transact(&pipe_name(), lock_now()).map_err(|error| format!("second lock: {error}"))?;
    if !again.contains("Accepted") {
        return Err(format!("the next lock did not start: {again}"));
    }
    if !wait_until(Duration::from_secs(20), || {
        status_state(&transact(&pipe_name(), status_body()).unwrap_or_default()).as_deref()
            == Some("Locked")
    }) {
        let state = transact(&pipe_name(), status_body()).unwrap_or_default();
        return Err(format!("the next lock did not reach Locked: {state}"));
    }
    type_pin_keys()?;
    if !wait_until(Duration::from_secs(30), || {
        status_state(&transact(&pipe_name(), status_body()).unwrap_or_default()).as_deref()
            == Some("Unlocked")
    }) {
        let state = transact(&pipe_name(), status_body()).unwrap_or_default();
        return Err(format!("the next lock did not unlock: {state}"));
    }
    if query_taskmgr() != "absent" {
        return Err(format!(
            "unlock did not restore DisableTaskMgr ({})",
            query_taskmgr()
        ));
    }
    hard_stop(guard)?;
    Ok(())
}

#[cfg(feature = "live-fail-closed")]
fn seed_system_pin(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|_| "pin dir".to_string())?;
    let secret = softlock_core::Secret::new(b"135742");
    let hash = softlock_core::hash_softlock_pin(&secret).map_err(|_| "pin hash".to_string())?;
    let staged = guard_dir().join("pin-hash.txt");
    fs::write(&staged, hash.as_bytes()).map_err(|_| "pin stage".to_string())?;
    let script = guard_dir().join("pin-seed.ps1");
    let log = guard_dir().join("pin-seed.txt");
    let pin_path = dir.join("pin.phc");
    let body = format!(
        r#"$ErrorActionPreference = 'Stop'
$log = '{log}'
try {{
  $hash = [IO.File]::ReadAllText('{staged}')
  [IO.File]::WriteAllText('{pin}', $hash)
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class SoftlockPin {{
  [DllImport("advapi32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
  public static extern bool ConvertStringSecurityDescriptorToSecurityDescriptor(string sddl, int revision, out IntPtr sd, out uint length);
  [DllImport("advapi32.dll", SetLastError=true)]
  public static extern bool GetSecurityDescriptorOwner(IntPtr sd, out IntPtr owner, out bool defaulted);
  [DllImport("advapi32.dll", SetLastError=true)]
  public static extern bool GetSecurityDescriptorDacl(IntPtr sd, out bool present, out IntPtr dacl, out bool defaulted);
  [DllImport("advapi32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
  public static extern int SetNamedSecurityInfo(string name, int type, int security, IntPtr owner, IntPtr group, IntPtr dacl, IntPtr sacl);
}}
'@
  $sd = [IntPtr]::Zero
  $length = [uint32]0
  if (-not [SoftlockPin]::ConvertStringSecurityDescriptorToSecurityDescriptor('O:SYD:P(A;;FA;;;SY)', 1, [ref]$sd, [ref]$length)) {{
    throw "convert $([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
  }}
  $owner = [IntPtr]::Zero
  $ownerDefault = $false
  [void][SoftlockPin]::GetSecurityDescriptorOwner($sd, [ref]$owner, [ref]$ownerDefault)
  $present = $false
  $dacl = [IntPtr]::Zero
  $daclDefault = $false
  [void][SoftlockPin]::GetSecurityDescriptorDacl($sd, [ref]$present, [ref]$dacl, [ref]$daclDefault)
  $status = [SoftlockPin]::SetNamedSecurityInfo('{pin}', 1, 0x80000005, $owner, [IntPtr]::Zero, $dacl, [IntPtr]::Zero)
  if ($status -ne 0) {{ throw "set $status" }}
  'ok' | Set-Content -LiteralPath $log
}} catch {{
  $_ | Out-String | Set-Content -LiteralPath $log
  exit 1
}}
"#,
        log = log.display(),
        staged = staged.display(),
        pin = pin_path.display(),
    );
    fs::write(&script, body).map_err(|_| "pin script".to_string())?;
    run_system_task("sl-pin", &script)?;
    let _ = wait_until(Duration::from_secs(20), || log.is_file());
    let note = fs::read_to_string(&log).unwrap_or_default();
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", "sl-pin", "/F"])
        .status();
    if note.trim() == "ok" {
        Ok(())
    } else {
        Err(format!("pin seed failed ({note})"))
    }
}

#[cfg(feature = "live-fail-closed")]
fn run_system_task(name: &str, script: &Path) -> Result<(), String> {
    let powershell = std::env::var("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let bin = format!(
        "{} -NoProfile -ExecutionPolicy Bypass -File {}",
        powershell.display(),
        script.display()
    );
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", name, "/F"])
        .status();
    let created = Command::new("schtasks")
        .args([
            "/Create", "/TN", name, "/TR", &bin, "/SC", "ONCE", "/ST", "00:00", "/RU", "SYSTEM",
            "/RL", "HIGHEST", "/F",
        ])
        .status()
        .map_err(|_| "task".to_string())?;
    if !created.success() {
        return Err(format!("{name} was not created"));
    }
    let _ = Command::new("schtasks")
        .args(["/Run", "/TN", name])
        .status();
    Ok(())
}

fn log_baseline() -> String {
    match fs::read_to_string(guard_dir().join("service.log")) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        // An unread snapshot must not treat the whole file as new. A sentinel
        // that cannot prefix a real log keeps the suffix empty.
        Err(_) => "\u{0}unreadable".to_string(),
    }
}

fn confirmed_since(baseline: &str, expected: &str) -> bool {
    let log = fs::read_to_string(guard_dir().join("service.log")).unwrap_or_default();
    softlock_core::confirmed_during_wait(
        baseline,
        &log,
        &query_taskmgr(),
        expected,
        journal_ready(&journal_file()),
    )
}

#[cfg(any(feature = "simulated-physical", feature = "live-fail-closed"))]
#[cfg_attr(not(feature = "live-fail-closed"), allow(dead_code))]
fn type_pin_keys() -> Result<(), String> {
    // Keeps the release-scan marker on the only build that can compile this
    // path. A product binary does not enable either feature. After the
    // disconnect this thread is not on the input desktop, so SendInput from
    // here returns success and the console hook never sees the keys.
    // `last_physical_input_ms` stayed null on 3d6d754 for that reason. An
    // interactive task runs on the console desktop, which is where the lock
    // host installed its hook.
    let _ = softlock_service::present_physical_marker_bytes();
    let script = guard_dir().join("type-pin.ps1");
    let log = guard_dir().join("type-pin.txt");
    let _ = fs::remove_file(&log);
    let body = format!(
        r#"$ErrorActionPreference = 'Stop'
$log = '{log}'
try {{
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SoftlockPinKeys {{
  [StructLayout(LayoutKind.Explicit, Size = 40)]
  public struct INPUT {{
    [FieldOffset(0)] public uint type;
    [FieldOffset(8)] public ushort wVk;
    [FieldOffset(10)] public ushort wScan;
    [FieldOffset(12)] public uint dwFlags;
    [FieldOffset(16)] public uint time;
    [FieldOffset(24)] public ulong extra;
  }}
  [DllImport("user32.dll", SetLastError=true)]
  public static extern uint SendInput(uint n, INPUT[] inputs, int cb);
  [DllImport("user32.dll", SetLastError=true)]
  public static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
  [DllImport("user32.dll", SetLastError=true)]
  public static extern bool SetThreadDesktop(IntPtr desktop);
}}
'@
  $desk = [SoftlockPinKeys]::OpenInputDesktop(0, $false, 0x1FF)
  $switched = $false
  if ($desk -ne [IntPtr]::Zero) {{
    $switched = [SoftlockPinKeys]::SetThreadDesktop($desk)
  }}
  $extra = [Convert]::ToUInt64('5048595350494E31', 16)
  foreach ($vk in 0x7C,0x31,0x33,0x35,0x37,0x34,0x32,0x0D) {{
    $down = New-Object SoftlockPinKeys+INPUT
    $down.type = 1
    $down.wVk = [uint16]$vk
    $down.extra = $extra
    $up = New-Object SoftlockPinKeys+INPUT
    $up.type = 1
    $up.wVk = [uint16]$vk
    $up.dwFlags = 2
    $up.extra = $extra
    $n = [SoftlockPinKeys]::SendInput(2, @($down, $up), 40)
    if ($n -ne 2) {{
      throw "send $vk n=$n err=$([Runtime.InteropServices.Marshal]::GetLastWin32Error()) desk=$desk switched=$switched"
    }}
    Start-Sleep -Milliseconds 80
  }}
  "ok desk=$desk switched=$switched" | Set-Content -LiteralPath $log
}} catch {{
  $_ | Out-String | Set-Content -LiteralPath $log
  exit 1
}}
"#,
        log = log.display(),
    );
    fs::write(&script, body).map_err(|_| "pin script".to_string())?;
    run_interactive_task("sl-type-pin", &script)?;
    let delivered = wait_until(Duration::from_secs(20), || log.is_file());
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", "sl-type-pin", "/F"])
        .status();
    let note = fs::read_to_string(&log).unwrap_or_default();
    let note = note.split_whitespace().collect::<Vec<_>>().join(" ");
    if delivered && note.starts_with("ok") {
        Ok(())
    } else {
        Err(format!(
            "the pin was not typed on the input desktop ({note})"
        ))
    }
}

#[cfg(any(feature = "simulated-physical", feature = "live-fail-closed"))]
#[cfg_attr(not(feature = "live-fail-closed"), allow(dead_code))]
fn run_interactive_task(name: &str, script: &Path) -> Result<(), String> {
    let user = Command::new("whoami")
        .output()
        .map_err(|_| "whoami".to_string())?;
    let user = String::from_utf8_lossy(&user.stdout).trim().to_string();
    if user.is_empty() {
        return Err("whoami returned no user".to_string());
    }
    let powershell = std::env::var("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let bin = format!(
        "{} -NoProfile -ExecutionPolicy Bypass -File {}",
        powershell.display(),
        script.display()
    );
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", name, "/F"])
        .status();
    let created = Command::new("schtasks")
        .args([
            "/Create", "/TN", name, "/TR", &bin, "/SC", "ONCE", "/ST", "00:00", "/RU", &user,
            "/IT", "/RL", "LIMITED", "/F",
        ])
        .output()
        .map_err(|_| "pin task".to_string())?;
    if !created.status.success() {
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&created.stdout),
            String::from_utf8_lossy(&created.stderr)
        );
        return Err(format!("pin task was not created ({text})"));
    }
    let started = Command::new("schtasks")
        .args(["/Run", "/TN", name])
        .output()
        .map_err(|_| "pin task".to_string())?;
    if started.status.success() {
        Ok(())
    } else {
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&started.stdout),
            String::from_utf8_lossy(&started.stderr)
        );
        Err(format!("pin task did not start ({text})"))
    }
}

fn recover_active(guard: &mut Guard) -> Result<(), String> {
    set_taskmgr("0")?;
    let pause = guard_dir().join("pause-active");
    let _ = fs::remove_file(&pause);
    spawn_service(guard, Some("after"), &pause, true)?;
    wait_pipe()?;
    lock_in_background();
    if !wait_until(Duration::from_secs(20), || {
        query_taskmgr() == "1" && host_running() && journal_ready(&journal_file())
    }) {
        return Err("the lock host was not running with DisableTaskMgr set".to_string());
    }
    // Killing the service makes the host exit. A second process boots the
    // same journal while this service is still paused and the host is alive.
    let boot_pipe = format!(r"\\.\pipe\softlock-journal-boot-{}", std::process::id());
    let runner =
        std::env::var("RUNNER_ENVIRONMENT").unwrap_or_else(|_| "github-hosted".to_string());
    let mut boot = Command::new(service_exe())
        .arg("--foreground")
        .env("SOFTLOCK_PIPE", &boot_pipe)
        .env("SOFTLOCK_LOG", guard_dir().join("boot.log"))
        .env("SOFTLOCK_PIN_DIR", guard_dir().join("pin-boot"))
        .env("SOFTLOCK_DEV_APP", "1")
        .env("SOFTLOCK_STOP_FILE", guard_dir().join("stop-boot"))
        .env("GITHUB_ACTIONS", "true")
        .env("RUNNER_ENVIRONMENT", runner)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "boot spawn".to_string())?;
    let mut last = String::new();
    let mut saw_unlocked = false;
    let closed = wait_until(Duration::from_secs(25), || {
        let status = transact(&boot_pipe, status_body()).unwrap_or_default();
        let state = status_state(&status);
        if state.as_deref() == Some("Unlocked") {
            saw_unlocked = true;
        }
        last = status;
        match state.as_deref() {
            // Still guarded: the locked dword stays.
            Some("FailingClosed" | "Locked" | "Recovering") => query_taskmgr() == "1",
            // OS lock confirmed: section 7 reverts the journal.
            Some("OsLocked") => query_taskmgr() == "0",
            _ => false,
        }
    });
    if saw_unlocked {
        let _ = boot.kill();
        let _ = boot.wait();
        return Err("an active lock returned to Unlocked".to_string());
    }
    let _ = boot.kill();
    let _ = boot.wait();
    let value = query_taskmgr();
    if !closed || (value != "1" && value != "0") {
        return Err(format!(
            "an active journal did not stay fail-closed: {last} value {value}"
        ));
    }
    Ok(())
}

fn session_os_held() -> bool {
    use windows::Win32::System::RemoteDesktop::{
        WTSConnectState, WTSDisconnected, WTSFreeMemory, WTSGetActiveConsoleSessionId,
        WTSQuerySessionInformationW,
    };
    use windows::core::PWSTR;
    let session = unsafe { WTSGetActiveConsoleSessionId() };
    if session == u32::MAX {
        return true;
    }
    let mut buffer = PWSTR::null();
    let mut bytes = 0u32;
    let state = unsafe {
        if WTSQuerySessionInformationW(None, session, WTSConnectState, &mut buffer, &mut bytes)
            .is_err()
            || buffer.is_null()
        {
            return false;
        }
        let state = *buffer.0.cast::<u32>();
        WTSFreeMemory(buffer.0.cast());
        state
    };
    state == WTSDisconnected.0 as u32
}

fn realign() -> Result<(), String> {
    #[cfg(feature = "live-fail-closed")]
    {
        align_console()
    }
    #[cfg(not(feature = "live-fail-closed"))]
    {
        Ok(())
    }
}

fn status_state(json: &str) -> Option<String> {
    let key = "\"state\":\"";
    let start = json.find(key)? + key.len();
    let rest = json.get(start..)?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

struct Guard {
    rows: Vec<Saved>,
    reported: bool,
    config_path: Option<PathBuf>,
    config_dacl: Option<String>,
    config_bytes: Option<Vec<u8>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "softlock-lockhost.exe"])
            .status();
        kill_service_process();
        let _ = Command::new("sc").args(["delete", SERVICE_NAME]).status();
        for row in &self.rows {
            let _ = set_value(row.key, row.name, &row.value);
        }
        let _ = (&self.config_path, &self.config_dacl, &self.config_bytes);
        #[cfg(feature = "live-fail-closed")]
        if let (Some(path), Some(dacl)) = (&self.config_path, &self.config_dacl) {
            let _ = softlock_service::testing_restore_dacl(path, dacl);
        }
        #[cfg(feature = "live-fail-closed")]
        if let Some(path) = &self.config_path {
            if let Some(bytes) = &self.config_bytes {
                let _ = fs::write(path, bytes);
            } else {
                let _ = fs::remove_file(path);
            }
        }
        if !self.reported {
            capture();
            write_result(r#"{"ok":false,"reason":"ended"}"#);
        }
    }
}

fn ensure_service() -> Result<(), String> {
    let _ = Command::new("sc").args(["stop", SERVICE_NAME]).status();
    let _ = Command::new("sc").args(["delete", SERVICE_NAME]).status();
    thread::sleep(Duration::from_millis(400));
    let bin = service_exe().display().to_string();
    let status = Command::new("sc")
        .args([
            "create",
            SERVICE_NAME,
            "binPath=",
            &bin,
            "start=",
            "demand",
            "obj=",
            "LocalSystem",
        ])
        .status()
        .map_err(|_| "sc create".to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("sc create failed".to_string())
    }
}

fn spawn_service(
    _guard: &mut Guard,
    pause: Option<&str>,
    pause_file: &Path,
    attach: bool,
) -> Result<(), String> {
    fs::create_dir_all(guard_dir()).map_err(|_| "temp".to_string())?;
    fs::create_dir_all(guard_dir().join("pin")).map_err(|_| "temp".to_string())?;
    // The environment is read when the service starts. Stop anything that
    // came up before those values were written, then start once.
    write_service_env(pause, pause_file)?;
    kill_service_process();
    if !wait_until(Duration::from_secs(20), service_stopped) {
        return Err(format!("service did not stop: {}", service_state()));
    }
    // A recovery start leaves the console where the kill left it. Attaching
    // first would turn a confirmed disconnect back into an active desktop,
    // and an empty journal would then report Unlocked. SPEC section 7.
    #[cfg(feature = "live-fail-closed")]
    if attach {
        align_console()?;
    }
    #[cfg(not(feature = "live-fail-closed"))]
    {
        let _ = attach;
    }
    match start_service() {
        Ok(()) => Ok(()),
        Err(first) => {
            // 1056 while STOP_PENDING. Wait until it is actually stopped.
            kill_service_process();
            if !wait_until(Duration::from_secs(20), service_stopped) {
                return Err(first);
            }
            start_service()
        }
    }
}

fn start_service() -> Result<(), String> {
    let output = Command::new("sc")
        .args(["start", SERVICE_NAME])
        .output()
        .map_err(|_| "sc start".to_string())?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if wait_until(Duration::from_secs(20), service_running) {
        Ok(())
    } else {
        Err(format!(
            "sc start failed: {} state {}",
            text.trim(),
            service_state()
        ))
    }
}

fn service_state() -> String {
    let Ok(output) = Command::new("sc").args(["query", SERVICE_NAME]).output() else {
        return "unreadable".to_string();
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn service_stopped() -> bool {
    let state = service_state();
    state.contains("STOPPED") || state.contains("1060")
}

fn lock_in_background() {
    let pipe = pipe_name();
    thread::spawn(move || {
        let _ = transact(&pipe, lock_now());
    });
}

fn write_service_env(pause: Option<&str>, pause_file: &Path) -> Result<(), String> {
    let runner =
        std::env::var("RUNNER_ENVIRONMENT").unwrap_or_else(|_| "github-hosted".to_string());
    let mut pairs = vec![
        "GITHUB_ACTIONS=true".to_string(),
        format!("RUNNER_ENVIRONMENT={runner}"),
        "SOFTLOCK_DEV_APP=1".to_string(),
        format!("SOFTLOCK_PIPE={}", pipe_name()),
        format!("SOFTLOCK_LOG={}", guard_dir().join("service.log").display()),
        format!("SOFTLOCK_PIN_DIR={}", guard_dir().join("pin").display()),
        format!("SOFTLOCK_STOP_FILE={}", guard_dir().join("stop").display()),
    ];
    if std::env::var("SOFTLOCK_JOURNAL_LIVE").ok().as_deref() == Some("sign-in") {
        // The lock host is in the console session. The sign-in PIN is
        // injected on the input desktop. The host counts those keys as
        // physical only when it was built with live-fail-closed and this
        // variable is set.
        pairs.push("SOFTLOCK_PRESENT_PHYSICAL=1".to_string());
    }
    if let Some(phase) = pause {
        pairs.push(format!("SOFTLOCK_JOURNAL_PAUSE={phase}"));
        pairs.push(format!(
            "SOFTLOCK_JOURNAL_PAUSE_FILE={}",
            pause_file.display()
        ));
    }
    let data = pairs.join("\\0");
    let key = format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{SERVICE_NAME}");
    let status = Command::new("reg")
        .args([
            "add",
            &key,
            "/v",
            "Environment",
            "/t",
            "REG_MULTI_SZ",
            "/d",
            &data,
            "/f",
        ])
        .status()
        .map_err(|_| "env".to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("env".to_string())
    }
}

fn hard_stop(guard: &mut Guard) -> Result<(), String> {
    stop_in_order(guard, KillOrder::Both)
}

fn stop_in_order(_guard: &mut Guard, order: KillOrder) -> Result<(), String> {
    // Freeze both, then kill in the order under test. A process that exits
    // between the snapshot and the open is already stopped. One that is still
    // listed and cannot be suspended fails the test. The product has to end
    // OsLocked or session-held for every order.
    freeze("softlock-service.exe")?;
    freeze("softlock-lockhost.exe")?;
    match order {
        KillOrder::ServiceFirst => {
            kill_image("softlock-service.exe", "service did not exit")?;
            let _ = Command::new("sc").args(["stop", SERVICE_NAME]).status();
            kill_image("softlock-lockhost.exe", "lock host did not exit")?;
        }
        KillOrder::HostFirst => {
            kill_image("softlock-lockhost.exe", "lock host did not exit")?;
            kill_image("softlock-service.exe", "service did not exit")?;
            let _ = Command::new("sc").args(["stop", SERVICE_NAME]).status();
        }
        KillOrder::Both => {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let _ = Command::new("taskkill")
                    .args(["/F", "/IM", "softlock-service.exe"])
                    .status();
                let _ = Command::new("taskkill")
                    .args(["/F", "/IM", "softlock-lockhost.exe"])
                    .status();
                if !image_running("softlock-service.exe") && !host_running() {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("service and host did not exit together".to_string());
                }
                thread::sleep(Duration::from_millis(50));
            }
            let _ = Command::new("sc").args(["stop", SERVICE_NAME]).status();
        }
    }
    Ok(())
}

fn kill_image(image: &str, error: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let _ = Command::new("taskkill").args(["/F", "/IM", image]).status();
        let gone = if image == "softlock-lockhost.exe" {
            !host_running()
        } else {
            !image_running(image)
        };
        if gone {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(error.to_string());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn freeze(image: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(pid) = image_pid(image) {
            return suspend_pid(image, pid);
        }
        if !image_running(image) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    if let Some(pid) = image_pid(image) {
        suspend_pid(image, pid)
    } else if image_running(image) {
        Err(format!("{image} pid"))
    } else {
        Ok(())
    }
}

fn suspend_pid(image: &str, pid: u32) -> Result<(), String> {
    unsafe {
        let opened = windows::Win32::System::Threading::OpenProcess(
            windows::Win32::System::Threading::PROCESS_ACCESS_RIGHTS(0x0800),
            false,
            pid,
        );
        let Ok(process) = opened else {
            return if image_pid(image) == Some(pid) {
                Err(format!("open {image}"))
            } else {
                Ok(())
            };
        };
        let status = NtSuspendProcess(process.0);
        let _ = windows::Win32::Foundation::CloseHandle(process);
        if status == 0 || image_pid(image) != Some(pid) {
            Ok(())
        } else {
            Err(format!("suspend {image} returned {status}"))
        }
    }
}

fn image_running(image: &str) -> bool {
    let Ok(output) = Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {image}"), "/NH"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout)
        .to_ascii_lowercase()
        .contains(&image.to_ascii_lowercase())
}

fn image_pid(image: &str) -> Option<u32> {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {image}"), "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().find(|line| {
        line.to_ascii_lowercase()
            .contains(&image.to_ascii_lowercase())
    })?;
    let pid = line.split(',').nth(1)?.trim().trim_matches('"');
    pid.parse().ok()
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSuspendProcess(process: *mut core::ffi::c_void) -> i32;
}

fn kill_service_process() {
    if let Some(pid) = service_pid() {
        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .status();
    }
    let _ = Command::new("sc").args(["stop", SERVICE_NAME]).status();
}

fn service_pid() -> Option<u32> {
    let output = Command::new("sc")
        .args(["queryex", SERVICE_NAME])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("PID") else {
            continue;
        };
        let rest = rest.trim().trim_start_matches(':').trim();
        if let Ok(pid) = rest.parse::<u32>()
            && pid != 0
        {
            return Some(pid);
        }
    }
    None
}

fn service_running() -> bool {
    let Ok(output) = Command::new("sc").args(["query", SERVICE_NAME]).output() else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).contains("RUNNING")
}

fn capture() {
    let _ = fs::create_dir_all(artifact_dir());
    let log = guard_dir().join("service.log");
    let mut body = fs::read_to_string(&log).unwrap_or_else(|_| "missing".to_string());
    if let Ok(boot) = fs::read_to_string(guard_dir().join("boot.log")) {
        body.push_str("\n-- boot --\n");
        body.push_str(&boot);
    }
    let _ = fs::write(artifact_dir().join("service.log"), body);
    let dir = journal_file();
    let parent = dir.parent().unwrap_or(dir.as_path());
    let listing = Command::new("cmd")
        .args(["/c", "dir", "/a", &parent.display().to_string()])
        .output();
    let text = listing
        .map(|output| {
            let mut body = String::from_utf8_lossy(&output.stdout).into_owned();
            body.push_str(&String::from_utf8_lossy(&output.stderr));
            body
        })
        .unwrap_or_else(|_| "missing".to_string());
    let _ = fs::write(artifact_dir().join("journal-dir.txt"), text);
}

#[cfg(feature = "live-fail-closed")]
fn repair_as_system() -> Result<(i32, String), String> {
    let script = guard_dir().join("repair.ps1");
    let code_path = guard_dir().join("repair-code.txt");
    let log_path = guard_dir().join("repair-out.txt");
    let out_path = guard_dir().join("repair-stdout.txt");
    let _ = fs::remove_file(&code_path);
    let body = format!(
        "$ErrorActionPreference = 'Stop'\ntry {{\n  $p = Start-Process -FilePath '{exe}' -ArgumentList '--repair-settings' -Wait -PassThru -WindowStyle Hidden -RedirectStandardError '{log}' -RedirectStandardOutput '{out}'\n  Set-Content -LiteralPath '{code}' -Value $p.ExitCode -NoNewline\n}} catch {{\n  Set-Content -LiteralPath '{log}' -Value $_.Exception.Message\n  Set-Content -LiteralPath '{code}' -Value '1' -NoNewline\n}}\n",
        exe = service_exe().display(),
        log = log_path.display(),
        out = out_path.display(),
        code = code_path.display(),
    );
    fs::write(&script, body).map_err(|_| "repair script".to_string())?;
    let powershell = std::env::var("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let bin = format!(
        "{} -NoProfile -ExecutionPolicy Bypass -File {}",
        powershell.display(),
        script.display()
    );
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", "sl-repair", "/F"])
        .status();
    let created = Command::new("schtasks")
        .args([
            "/Create",
            "/TN",
            "sl-repair",
            "/TR",
            &bin,
            "/SC",
            "ONCE",
            "/ST",
            "00:00",
            "/RU",
            "SYSTEM",
            "/RL",
            "HIGHEST",
            "/F",
        ])
        .status()
        .map_err(|_| "repair task".to_string())?;
    if !created.success() {
        return Err("repair task was not created".to_string());
    }
    let started = Command::new("schtasks")
        .args(["/Run", "/TN", "sl-repair"])
        .status()
        .map_err(|_| "repair task".to_string())?;
    let ready = started.success() && wait_until(Duration::from_secs(20), || code_path.is_file());
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", "sl-repair", "/F"])
        .status();
    if !ready {
        return Err("repair task did not finish".to_string());
    }
    let code = fs::read_to_string(&code_path).unwrap_or_default();
    let code = code.trim().parse::<i32>().unwrap_or(1);
    let mut note = fs::read_to_string(&log_path).unwrap_or_default();
    note.push(' ');
    note.push_str(&fs::read_to_string(&out_path).unwrap_or_default());
    note = note
        .chars()
        .map(|c| if c.is_control() || c == '"' { ' ' } else { c })
        .collect();
    note = note.split_whitespace().collect::<Vec<_>>().join(" ");
    if note.len() > 160 {
        note.truncate(160);
    }
    Ok((code, note))
}

#[cfg(not(feature = "live-fail-closed"))]
fn repair_as_system() -> Result<(i32, String), String> {
    repair_settings()
}

fn clear_os_hold() -> Result<(), String> {
    // The confirmed disconnect stays OsLocked for this logon. Repair drops
    // the marker only once this logon is the console again, which is the
    // sign-in the next check starts from.
    realign()?;
    let (code, _note) = repair_settings()?;
    if code == 0 {
        return Ok(());
    }
    let (code, note) = repair_as_system()?;
    if code == 0 {
        Ok(())
    } else {
        Err(format!(
            "repair did not clear the OS lock marker ({code} {note})"
        ))
    }
}

fn repair_settings() -> Result<(i32, String), String> {
    let output = Command::new(service_exe())
        .arg("--repair-settings")
        .env("SOFTLOCK_PIPE", pipe_name())
        .output()
        .map_err(|_| "repair".to_string())?;
    let mut note = String::from_utf8_lossy(&output.stderr)
        .chars()
        .map(|c| if c.is_control() || c == '"' { ' ' } else { c })
        .collect::<String>();
    note = note.split_whitespace().collect::<Vec<_>>().join(" ");
    if note.len() > 160 {
        note.truncate(160);
    }
    Ok((output.status.code().unwrap_or(1), note))
}

fn wait_pipe() -> Result<(), String> {
    let ok = wait_until(Duration::from_secs(20), || {
        transact(&pipe_name(), hello()).is_ok()
    });
    if ok {
        Ok(())
    } else {
        Err("pipe did not open".to_string())
    }
}

fn journal_text() -> String {
    fs::read_to_string(journal_file()).unwrap_or_default()
}

fn journal_ready(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    text.contains("\"says_locked\":true")
        && text.contains("\"sid\":\"S-1-5-21-")
        && [
            "DisableTaskMgr",
            "NoLogoff",
            "DisableChangePassword",
            "HideFastUserSwitching",
            "DisableLockWorkstation",
            "NoClose",
            "ConsentPromptBehaviorAdmin",
            "ConsentPromptBehaviorUser",
        ]
        .iter()
        .all(|name| text.contains(name))
}

fn journal_file() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    PathBuf::from(root)
        .join("System32")
        .join("config")
        .join("systemprofile")
        .join("AppData")
        .join("Local")
        .join(format!("{SERVICE_NAME}Journal"))
        .join("journal.json")
}

fn query_taskmgr() -> String {
    query_value(KEY, "DisableTaskMgr")
}

fn query_value(key: &str, name: &str) -> String {
    let output = Command::new("reg")
        .args(["query", key, "/v", name])
        .output();
    let Ok(output) = output else {
        return "absent".to_string();
    };
    if !output.status.success() {
        return "absent".to_string();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(hex) = text.split_whitespace().find(|part| part.starts_with("0x")) else {
        return "other".to_string();
    };
    let Ok(value) = u32::from_str_radix(hex.trim_start_matches("0x"), 16) else {
        return "other".to_string();
    };
    value.to_string()
}

fn set_taskmgr(value: &str) -> Result<(), String> {
    set_value(KEY, "DisableTaskMgr", value)
}

fn set_value(key: &str, name: &str, value: &str) -> Result<(), String> {
    if value == "absent" {
        let _ = Command::new("reg")
            .args(["delete", key, "/v", name, "/f"])
            .status();
        if query_value(key, name) != "absent" {
            return Err(format!("could not delete {name}"));
        }
        return Ok(());
    }
    let status = Command::new("reg")
        .args(["add", key, "/v", name, "/t", "REG_DWORD", "/d", value, "/f"])
        .status()
        .map_err(|_| "reg add".to_string())?;
    if status.success() && query_value(key, name) == value {
        Ok(())
    } else {
        Err(format!("could not set {name}"))
    }
}

fn every_row_restores(guard: &mut Guard) -> Result<(), String> {
    let chosen = [
        (KEY, "DisableTaskMgr", "absent", "1"),
        (EXPLORER, "NoLogoff", "0", "1"),
        (KEY, "DisableChangePassword", "1", "1"),
        (MACHINE, "HideFastUserSwitching", "absent", "1"),
        (KEY, "DisableLockWorkstation", "0", "1"),
        (EXPLORER, "NoClose", "1", "1"),
        (MACHINE, "ConsentPromptBehaviorAdmin", "5", "1"),
        (MACHINE, "ConsentPromptBehaviorUser", "3", "0"),
    ];
    for (key, name, previous, _) in chosen {
        set_value(key, name, previous)?;
    }
    let pause = guard_dir().join("pause-rows");
    spawn_service(guard, None, &pause, true)?;
    wait_pipe()?;
    transact(&pipe_name(), lock_now()).map_err(|error| format!("lock: {error}"))?;
    if !wait_until(Duration::from_secs(20), || {
        chosen
            .iter()
            .all(|(key, name, _, locked)| query_value(key, name) == *locked)
    }) {
        return Err(format!(
            "lock did not set every Ctrl+Alt+Del and UAC row: {}",
            row_report(&chosen)
        ));
    }
    hard_stop(guard)?;
    let _ = wait_until(Duration::from_secs(5), || !host_running());
    spawn_service(guard, None, &pause, true)?;
    let restored = wait_until(Duration::from_secs(25), || {
        chosen
            .iter()
            .all(|(key, name, previous, _)| query_value(key, name) == *previous)
    });
    hard_stop(guard)?;
    clear_os_hold()?;
    realign()?;
    if !restored {
        return Err(format!(
            "restart did not restore every row: {}",
            row_report(&chosen)
        ));
    }
    Ok(())
}

fn row_report(chosen: &[(&str, &str, &str, &str)]) -> String {
    chosen
        .iter()
        .map(|(key, name, previous, locked)| {
            format!(
                "{name}={} (wanted {locked}, previous {previous})",
                query_value(key, name)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn installed_service_running() -> bool {
    let Ok(output) = Command::new("sc").args(["query", SERVICE_NAME]).output() else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).contains("RUNNING")
}

fn host_running() -> bool {
    let Ok(output) = Command::new("tasklist")
        .args([
            "/FI",
            "IMAGENAME eq softlock-lockhost.exe",
            "/FO",
            "CSV",
            "/NH",
        ])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout)
        .to_ascii_lowercase()
        .contains("softlock-lockhost.exe")
}

fn service_exe() -> PathBuf {
    std::env::var("SOFTLOCK_SERVICE_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/journal/release/softlock-service.exe"))
}

fn pipe_name() -> String {
    format!(r"\\.\pipe\softlock-journal-{}", std::process::id())
}

fn guard_dir() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    PathBuf::from(root)
        .join("Temp")
        .join(format!("softlock-journal-{}", std::process::id()))
}

fn artifact_dir() -> PathBuf {
    let base = std::env::var("RUNNER_TEMP")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("softlock-journal")
}

fn hello() -> &'static str {
    r#"{"v":1,"id":1,"kind":"Hello","client_version":"0"}"#
}

fn lock_now() -> &'static str {
    r#"{"v":1,"id":2,"kind":"Lock","source":"App"}"#
}

fn status_body() -> &'static str {
    r#"{"v":1,"id":3,"kind":"GetStatus"}"#
}

fn wait_until(limit: Duration, mut probe: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if probe() {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    probe()
}

fn write_result(body: &str) {
    let dir = artifact_dir();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join("result.json"), body);
}

fn transact(pipe: &str, json: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::os::windows::io::FromRawHandle;
    let mut frame = (json.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(json.as_bytes());
    let wide: Vec<u16> = pipe.encode_utf16().chain(std::iter::once(0)).collect();
    let handle = unsafe {
        windows::Win32::Storage::FileSystem::CreateFileW(
            windows::core::PCWSTR(wide.as_ptr()),
            0xC000_0000,
            windows::Win32::Storage::FileSystem::FILE_SHARE_READ
                | windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE,
            None,
            windows::Win32::Storage::FileSystem::OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(|error| format!("open: {error}"))?
    };
    let mut file = unsafe { std::fs::File::from_raw_handle(handle.0) };
    file.write_all(&frame)
        .map_err(|error| format!("write: {error}"))?;
    let mut prefix = [0u8; 4];
    file.read_exact(&mut prefix)
        .map_err(|error| format!("read: {error}"))?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length > 64 * 1024 {
        return Err("frame".to_string());
    }
    let mut body = vec![0u8; length];
    file.read_exact(&mut body)
        .map_err(|error| format!("body: {error}"))?;
    String::from_utf8(body).map_err(|_| "utf8".to_string())
}
