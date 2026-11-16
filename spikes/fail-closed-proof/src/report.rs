//! Text report printed by the proof job.

use crate::verdict::{ObservedPath, Outcome, notification_name, within_one_second};

pub fn render(outcome: &Outcome) -> String {
    let mut lines = Vec::new();
    let result = if crate::verdict::passed(outcome) {
        "pass"
    } else {
        "fail"
    };
    lines.push(format!("result={result}"));
    lines.push(format!("runner-image={}", outcome.runner_image));
    lines.push(format!("session={}", outcome.session_id));
    match outcome.service_session_id {
        Some(session) => lines.push(format!("service-session={session}")),
        None => lines.push("service-session=missing".to_string()),
    }
    if outcome.policies.is_empty() {
        lines.push("policies-applied=none".to_string());
    } else {
        lines.push(format!("policies-applied={}", outcome.policies.join(",")));
    }
    let win32 = outcome
        .lock_blocked_win32
        .map(|code| code.to_string())
        .unwrap_or_else(|| "none".to_string());
    let session_lock = match outcome.lock_session_locked {
        Some(true) => "locked",
        Some(false) => "unlocked",
        None => "unknown",
    };
    lines.push(format!(
        "lock-while-policy={} win32={win32} notified={} session-lock-state={session_lock}",
        if outcome.lock_blocked {
            "blocked"
        } else {
            "unexpected"
        },
        outcome.lock_saw_notification
    ));
    lines.push(path_line(
        "path-service-disconnect",
        outcome.service_disconnect.as_ref(),
        crate::verdict::WTS_CONSOLE_DISCONNECT,
    ));
    lines.push(path_line(
        "path-session-disconnect",
        outcome.session_disconnect.as_ref(),
        crate::verdict::WTS_CONSOLE_DISCONNECT,
    ));
    lines.push(format!(
        "cleared-disable-lock-workstation={}",
        outcome.lock_cleared
    ));
    lines.push(path_line(
        "path-user-lock",
        outcome.user_lock.as_ref(),
        crate::verdict::WTS_SESSION_LOCK,
    ));
    let background = heartbeat_ok(outcome.service_disconnect.as_ref())
        && heartbeat_ok(outcome.session_disconnect.as_ref())
        && heartbeat_ok(outcome.user_lock.as_ref());
    lines.push(format!("background-kept-running={background}"));
    lines.push(format!("policies-restored={}", outcome.policies_restored));
    if let Some(error) = &outcome.error {
        lines.push(format!("error={error}"));
    }
    lines.join("\n")
}

fn heartbeat_ok(path: Option<&ObservedPath>) -> bool {
    path.is_some_and(|path| {
        path.heartbeat_gap_ms
            .is_some_and(|gap| gap < crate::verdict::HEARTBEAT_GAP_LIMIT_MS)
    })
}

fn path_line(name: &str, path: Option<&ObservedPath>, expected: u32) -> String {
    let Some(path) = path else {
        return format!("{name}=missing");
    };
    let notification = path.notification.map(notification_name).unwrap_or("none");
    let elapsed = path
        .elapsed_us
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string());
    let within = path.elapsed_us.is_some_and(within_one_second);
    let local_ok = !path.checked_locally || path.local_notification == Some(expected);
    let confirmed = path.api_ok && within && path.notification == Some(expected) && local_ok;
    let gap = path
        .heartbeat_gap_ms
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string());
    let mut line = format!(
        "{name}={} notification={notification} elapsed-us={elapsed} within-1s={within} api-ok={} heartbeat-gap-ms={gap}",
        if confirmed { "ok" } else { "fail" },
        path.api_ok
    );
    if path.checked_locally {
        let local = path
            .local_notification
            .map(notification_name)
            .unwrap_or("none");
        line.push_str(&format!(" local-notification={local}"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::{ObservedPath, WTS_CONSOLE_DISCONNECT, WTS_SESSION_LOCK};

    fn path(notification: u32) -> ObservedPath {
        ObservedPath {
            api_ok: true,
            notification: Some(notification),
            checked_locally: false,
            local_notification: None,
            elapsed_us: Some(8_000),
            heartbeat_gap_ms: Some(40),
        }
    }

    #[test]
    fn a_passing_report_names_the_three_confirmations() {
        let outcome = crate::verdict::Outcome {
            runner_image: "windows-2025".to_string(),
            session_id: 2,
            service_session_id: Some(0),
            policies: crate::journal::LOCKDOWN
                .iter()
                .map(|policy| policy.name.to_string())
                .collect(),
            lock_blocked: true,
            lock_blocked_win32: Some(5),
            lock_saw_notification: false,
            lock_session_locked: Some(false),
            service_disconnect: Some(path(WTS_CONSOLE_DISCONNECT)),
            session_disconnect: Some({
                let mut observed = path(WTS_CONSOLE_DISCONNECT);
                observed.checked_locally = true;
                observed.local_notification = Some(WTS_CONSOLE_DISCONNECT);
                observed
            }),
            lock_cleared: true,
            user_lock: Some(path(WTS_SESSION_LOCK)),
            policies_restored: true,
            error: None,
        };
        let text = render(&outcome);
        assert!(text.starts_with("result=pass\n"), "{text}");
        assert!(text.contains(
            "lock-while-policy=blocked win32=5 notified=false session-lock-state=unlocked"
        ));
        assert!(text.contains("local-notification=WTS_CONSOLE_DISCONNECT"));
        assert!(text.contains(
            "path-service-disconnect=ok notification=WTS_CONSOLE_DISCONNECT elapsed-us=8000 within-1s=true"
        ));
        assert!(text.contains(
            "path-session-disconnect=ok notification=WTS_CONSOLE_DISCONNECT elapsed-us=8000 within-1s=true"
        ));
        assert!(text.contains("cleared-disable-lock-workstation=true"));
        assert!(text.contains(
            "path-user-lock=ok notification=WTS_SESSION_LOCK elapsed-us=8000 within-1s=true"
        ));
        assert!(text.contains("background-kept-running=true"));
        assert!(text.contains("policies-restored=true"));
        assert!(text.contains("DisableLockWorkstation"));
    }
}
