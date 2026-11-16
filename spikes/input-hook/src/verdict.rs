//! Decisions for the input-hook proof. No operating-system calls.

/// Keyboard events injected during the load test. Tens of thousands.
pub const LOAD_EVENTS: u32 = 40_000;

/// Key-down events delivered to the probe window in each visibility phase.
pub const VISIBILITY_KEYS: u32 = 20;

/// The liveness check has to report a removed hook inside this budget.
pub const LIVENESS_BUDGET_MS: u64 = 500;

/// `SECURITY_MANDATORY_MEDIUM_RID`.
pub const IL_MEDIUM: u32 = 0x2000;
/// `SECURITY_MANDATORY_HIGH_RID`.
pub const IL_HIGH: u32 = 0x3000;
/// `SECURITY_MANDATORY_SYSTEM_RID`.
pub const IL_SYSTEM: u32 = 0x4000;

/// `dwExtraInfo` for the visibility keys.
pub const EXTRA_VIS: usize = 0x51F7_0001;
/// `dwExtraInfo` for the load burst.
pub const EXTRA_LOAD: usize = 0x51F7_0002;
/// `dwExtraInfo` for the swallowed no-op liveness marker.
pub const EXTRA_MARKER: usize = 0x51F7_0003;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountKind {
    Visible,
    Load,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookAction {
    /// Count the event and pass it to the next hook.
    PassCount { kind: CountKind, injected: bool },
    /// Pass the event through without counting it.
    Pass,
    /// Swallow the event. `count_marker` records a liveness marker.
    Swallow { count_marker: bool },
    /// Removal probe. The caller sleeps, then swallows. This is not the fast path.
    Slow { sleep_ms: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HookEvent {
    pub extra: usize,
    pub key_down: bool,
    pub injected: bool,
    pub slow_armed: bool,
    pub slow_ms: u32,
}

/// Classify one low-level keyboard event. Pure: no I/O and no allocation.
pub fn classify(event: HookEvent) -> HookAction {
    if event.extra == EXTRA_MARKER {
        if event.key_down && event.slow_armed && event.slow_ms > 0 {
            return HookAction::Slow {
                sleep_ms: event.slow_ms,
            };
        }
        return HookAction::Swallow {
            count_marker: event.key_down,
        };
    }
    if event.extra == EXTRA_VIS && event.key_down {
        return HookAction::PassCount {
            kind: CountKind::Visible,
            injected: event.injected,
        };
    }
    if event.extra == EXTRA_LOAD {
        return HookAction::PassCount {
            kind: CountKind::Load,
            injected: event.injected,
        };
    }
    HookAction::Pass
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Design {
    Confirm,
    Reject,
}

impl Design {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirm => "confirm",
            Self::Reject => "reject",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    NotNeeded,
    Needed,
    Unresolved,
}

impl Fallback {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotNeeded => "false",
            Self::Needed => "true",
            Self::Unresolved => "unresolved",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub system_il: u32,
    pub medium_il: u32,
    pub elevated_il: u32,
    pub system_session: u32,
    pub console_session: u32,
    pub system_user_is_system: bool,
    pub system_on_default_desktop: bool,
    pub elevated_token: String,
    pub medium_token: String,
    pub control_system_seen: u32,
    pub control_medium_seen: u32,
    pub control_window: u32,
    pub elevated_system_seen: u32,
    pub elevated_system_injected: u32,
    pub elevated_medium_seen: u32,
    pub elevated_hook_seen: u32,
    pub elevated_window: u32,
    pub load_sent: u32,
    pub load_seen: u32,
    pub hook_alive_after_load: bool,
    pub slow_entered: bool,
    pub liveness_healthy_before: bool,
    pub liveness_detect_ms: u64,
    pub liveness_removed: bool,
    pub timeout_ms: u64,
}

/// Why a sample failed a check. The variant, not the text, decides the result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    SystemIntegrity {
        got: u32,
    },
    MediumIntegrity {
        got: u32,
    },
    ElevatedIntegrity {
        got: u32,
    },
    Session {
        system: u32,
        console: u32,
    },
    NotSystemUser,
    NotDefaultDesktop,
    MediumControl {
        system: u32,
        medium: u32,
        window: u32,
    },
    SystemMissedElevated {
        seen: u32,
    },
    MissingInjectedFlag {
        seen: u32,
    },
    MediumSawElevated {
        seen: u32,
    },
    HighHookMissed {
        seen: u32,
    },
    WindowMissed {
        received: u32,
    },
    LoadTooSmall {
        sent: u32,
    },
    LoadDropped {
        seen: u32,
        sent: u32,
    },
    DeadAfterLoad,
    MarkerNotSeenBefore,
    SlowNeverRan,
    /// The slow callback ran and the hook was still installed afterwards.
    HookNotRemoved,
    /// The hook was removed, and the marker check reported it after the budget.
    LivenessLate {
        detect_ms: u64,
    },
    FallbackNeeded,
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SystemIntegrity { got } => {
                write!(f, "system hook integrity is {got}, want {IL_SYSTEM}")
            }
            Self::MediumIntegrity { got } => {
                write!(f, "medium hook integrity is {got}, want {IL_MEDIUM}")
            }
            Self::ElevatedIntegrity { got } => {
                write!(f, "elevated probe integrity is {got}, want {IL_HIGH}")
            }
            Self::Session { system, console } => {
                write!(
                    f,
                    "system hook session is {system}, console session is {console}"
                )
            }
            Self::NotSystemUser => write!(f, "system hook is not running as SYSTEM"),
            Self::NotDefaultDesktop => write!(f, "system hook is not on winsta0\\default"),
            Self::MediumControl {
                system,
                medium,
                window,
            } => write!(
                f,
                "medium-foreground control saw system {system} medium {medium} window {window}, want {VISIBILITY_KEYS}"
            ),
            Self::SystemMissedElevated { seen } => write!(
                f,
                "system hook saw {seen} of {VISIBILITY_KEYS} keys delivered to the elevated window"
            ),
            Self::MissingInjectedFlag { seen } => {
                write!(
                    f,
                    "system hook saw {seen} injected flags of {VISIBILITY_KEYS}"
                )
            }
            Self::MediumSawElevated { seen } => {
                write!(
                    f,
                    "medium hook saw {seen} keys destined for the elevated window"
                )
            }
            Self::HighHookMissed { seen } => write!(
                f,
                "high-integrity hook saw {seen} of {VISIBILITY_KEYS} elevated-window keys"
            ),
            Self::WindowMissed { received } => write!(
                f,
                "elevated window received {received} of {VISIBILITY_KEYS} keys the system hook passed on"
            ),
            Self::LoadTooSmall { sent } => {
                write!(
                    f,
                    "load sent {sent} events, which is below tens of thousands"
                )
            }
            Self::LoadDropped { seen, sent } => {
                write!(f, "load hook saw {seen} of {sent} injected events")
            }
            Self::DeadAfterLoad => write!(f, "hook was not alive after the load burst"),
            Self::MarkerNotSeenBefore => {
                write!(f, "liveness marker was not seen before the slow callback")
            }
            Self::SlowNeverRan => write!(f, "slow callback never ran"),
            Self::HookNotRemoved => write!(f, "slow callback did not get the hook removed"),
            Self::LivenessLate { detect_ms } => write!(
                f,
                "liveness reported the removed hook in {detect_ms} ms, budget is {LIVENESS_BUDGET_MS}"
            ),
            Self::FallbackNeeded => write!(
                f,
                "system hook missed elevated-window input and the elevated linked-token hook saw it"
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub pass: bool,
    /// Every other check held, and the only miss is that the hook was never removed.
    /// A removal that the marker check reported after the budget is a failure.
    pub measured: bool,
    pub design: Design,
    pub fallback: Fallback,
    pub liveness_within_budget: bool,
    pub passed_through: bool,
    pub injected_flag: bool,
    pub reasons: Vec<Reason>,
}

impl Decision {
    pub fn report(&self, sample: &Sample) -> String {
        let mut lines = vec![
            format!("system-il={}", sample.system_il),
            format!("medium-il={}", sample.medium_il),
            format!("elevated-il={}", sample.elevated_il),
            format!("system-session={}", sample.system_session),
            format!("console-session={}", sample.console_session),
            format!("elevated-token={}", sample.elevated_token),
            format!("medium-token={}", sample.medium_token),
            format!("system-sees-elevated={}", sample.elevated_system_seen),
            format!("system-injected={}", sample.elevated_system_injected),
            format!("medium-sees-elevated={}", sample.elevated_medium_seen),
            format!("elevated-hook-sees={}", sample.elevated_hook_seen),
            format!("elevated-window-received={}", sample.elevated_window),
            format!("injected-flag={}", self.injected_flag),
            format!("passed-through={}", self.passed_through),
            format!("medium-control-seen={}", sample.control_medium_seen),
            format!("system-control-seen={}", sample.control_system_seen),
            format!("medium-window-received={}", sample.control_window),
            format!("load-sent={}", sample.load_sent),
            format!("load-seen={}", sample.load_seen),
            format!("hook-alive-after-load={}", sample.hook_alive_after_load),
            format!("low-level-hooks-timeout-ms={}", sample.timeout_ms),
            format!("liveness-detect-ms={}", sample.liveness_detect_ms),
            format!("hook-removed={}", sample.liveness_removed),
            format!("liveness-within-500ms={}", self.liveness_within_budget),
            format!("fallback-needed={}", self.fallback.as_str()),
            format!("design={}", self.design.as_str()),
        ];
        for reason in &self.reasons {
            lines.push(format!("reason={reason}"));
        }
        lines.push(format!(
            "result={}",
            if self.pass {
                "pass"
            } else if self.measured {
                "measured"
            } else {
                "fail"
            }
        ));
        lines.join("\n")
    }

    /// `0` when the sample passed or the only miss is that removal never happened.
    pub fn exit_code(&self) -> i32 {
        if self.pass || self.measured { 0 } else { 1 }
    }
}

pub fn judge(sample: &Sample) -> Decision {
    let mut reasons = Vec::new();
    let system_saw_elevated = sample.elevated_system_seen == VISIBILITY_KEYS;
    let injected_flag = system_saw_elevated && sample.elevated_system_injected == VISIBILITY_KEYS;
    let passed_through = system_saw_elevated && sample.elevated_window == VISIBILITY_KEYS;
    let medium_missed_elevated = sample.elevated_medium_seen == 0;
    let high_saw_elevated = sample.elevated_hook_seen == VISIBILITY_KEYS;

    if sample.system_il != IL_SYSTEM {
        reasons.push(Reason::SystemIntegrity {
            got: sample.system_il,
        });
    }
    if sample.medium_il != IL_MEDIUM {
        reasons.push(Reason::MediumIntegrity {
            got: sample.medium_il,
        });
    }
    if sample.elevated_il != IL_HIGH {
        reasons.push(Reason::ElevatedIntegrity {
            got: sample.elevated_il,
        });
    }
    if sample.system_session == 0 || sample.system_session != sample.console_session {
        reasons.push(Reason::Session {
            system: sample.system_session,
            console: sample.console_session,
        });
    }
    if !sample.system_user_is_system {
        reasons.push(Reason::NotSystemUser);
    }
    if !sample.system_on_default_desktop {
        reasons.push(Reason::NotDefaultDesktop);
    }
    if sample.control_medium_seen != VISIBILITY_KEYS
        || sample.control_system_seen != VISIBILITY_KEYS
        || sample.control_window != VISIBILITY_KEYS
    {
        reasons.push(Reason::MediumControl {
            system: sample.control_system_seen,
            medium: sample.control_medium_seen,
            window: sample.control_window,
        });
    }
    if !system_saw_elevated {
        reasons.push(Reason::SystemMissedElevated {
            seen: sample.elevated_system_seen,
        });
    }
    if system_saw_elevated && !injected_flag {
        reasons.push(Reason::MissingInjectedFlag {
            seen: sample.elevated_system_injected,
        });
    }
    if !medium_missed_elevated {
        reasons.push(Reason::MediumSawElevated {
            seen: sample.elevated_medium_seen,
        });
    }
    if !high_saw_elevated {
        reasons.push(Reason::HighHookMissed {
            seen: sample.elevated_hook_seen,
        });
    }
    if system_saw_elevated && sample.elevated_window != VISIBILITY_KEYS {
        reasons.push(Reason::WindowMissed {
            received: sample.elevated_window,
        });
    }
    if sample.load_sent < 20_000 {
        reasons.push(Reason::LoadTooSmall {
            sent: sample.load_sent,
        });
    }
    if sample.load_seen != sample.load_sent {
        reasons.push(Reason::LoadDropped {
            seen: sample.load_seen,
            sent: sample.load_sent,
        });
    }
    if !sample.hook_alive_after_load {
        reasons.push(Reason::DeadAfterLoad);
    }
    if !sample.liveness_healthy_before {
        reasons.push(Reason::MarkerNotSeenBefore);
    }
    if !sample.slow_entered {
        reasons.push(Reason::SlowNeverRan);
    }
    let liveness_within_budget = sample.liveness_healthy_before
        && sample.slow_entered
        && sample.liveness_removed
        && sample.liveness_detect_ms <= LIVENESS_BUDGET_MS;
    if !sample.liveness_removed {
        reasons.push(Reason::HookNotRemoved);
    } else if sample.liveness_detect_ms > LIVENESS_BUDGET_MS {
        reasons.push(Reason::LivenessLate {
            detect_ms: sample.liveness_detect_ms,
        });
    }

    let design = if system_saw_elevated && medium_missed_elevated && passed_through {
        Design::Confirm
    } else {
        Design::Reject
    };
    // The fall-back question is only whether SYSTEM saw the elevated window.
    // A medium hook that also saw it, or a window that missed a swallowed key,
    // is a different failure.
    let fallback = if system_saw_elevated {
        Fallback::NotNeeded
    } else if high_saw_elevated {
        Fallback::Needed
    } else {
        Fallback::Unresolved
    };
    if design == Design::Reject && fallback == Fallback::Needed {
        reasons.push(Reason::FallbackNeeded);
    }

    let pass = reasons.is_empty();
    // Only "the hook was never removed". A late report of a real removal fails.
    let measured = !pass
        && reasons
            .iter()
            .all(|reason| matches!(reason, Reason::HookNotRemoved));
    Decision {
        pass,
        measured,
        design,
        fallback,
        liveness_within_budget,
        passed_through,
        injected_flag,
        reasons,
    }
}

#[cfg(test)]
fn passing_sample() -> Sample {
    Sample {
        system_il: IL_SYSTEM,
        medium_il: IL_MEDIUM,
        elevated_il: IL_HIGH,
        system_session: 2,
        console_session: 2,
        system_user_is_system: true,
        system_on_default_desktop: true,
        elevated_token: "linked".to_string(),
        medium_token: "user".to_string(),
        control_system_seen: VISIBILITY_KEYS,
        control_medium_seen: VISIBILITY_KEYS,
        control_window: VISIBILITY_KEYS,
        elevated_system_seen: VISIBILITY_KEYS,
        elevated_system_injected: VISIBILITY_KEYS,
        elevated_medium_seen: 0,
        elevated_hook_seen: VISIBILITY_KEYS,
        elevated_window: VISIBILITY_KEYS,
        load_sent: LOAD_EVENTS,
        load_seen: LOAD_EVENTS,
        hook_alive_after_load: true,
        slow_entered: true,
        liveness_healthy_before: true,
        liveness_detect_ms: 250,
        liveness_removed: true,
        timeout_ms: 300,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(extra: usize, key_down: bool, injected: bool, slow_armed: bool) -> HookEvent {
        HookEvent {
            extra,
            key_down,
            injected,
            slow_armed,
            slow_ms: if slow_armed { 1_800 } else { 0 },
        }
    }

    #[test]
    fn visible_key_down_passes_with_the_injected_flag() {
        assert_eq!(
            classify(event(EXTRA_VIS, true, true, false)),
            HookAction::PassCount {
                kind: CountKind::Visible,
                injected: true,
            }
        );
        assert_eq!(
            classify(event(EXTRA_VIS, true, false, false)),
            HookAction::PassCount {
                kind: CountKind::Visible,
                injected: false,
            }
        );
    }

    #[test]
    fn visible_key_up_passes_without_counting() {
        assert_eq!(
            classify(event(EXTRA_VIS, false, true, false)),
            HookAction::Pass
        );
    }

    #[test]
    fn load_events_pass_through() {
        assert_eq!(
            classify(event(EXTRA_LOAD, true, true, false)),
            HookAction::PassCount {
                kind: CountKind::Load,
                injected: true,
            }
        );
        assert_eq!(
            classify(event(EXTRA_LOAD, false, true, false)),
            HookAction::PassCount {
                kind: CountKind::Load,
                injected: true,
            }
        );
    }

    #[test]
    fn marker_is_swallowed_and_only_the_key_down_counts() {
        assert_eq!(
            classify(event(EXTRA_MARKER, true, true, false)),
            HookAction::Swallow { count_marker: true }
        );
        assert_eq!(
            classify(event(EXTRA_MARKER, false, true, false)),
            HookAction::Swallow {
                count_marker: false
            }
        );
    }

    #[test]
    fn armed_marker_key_down_is_the_slow_probe() {
        assert_eq!(
            classify(event(EXTRA_MARKER, true, true, true)),
            HookAction::Slow { sleep_ms: 1_800 }
        );
        assert_eq!(
            classify(event(EXTRA_VIS, true, true, true)),
            HookAction::PassCount {
                kind: CountKind::Visible,
                injected: true,
            }
        );
    }

    #[test]
    fn a_zero_sleep_does_not_arm_the_slow_path() {
        let mut event = event(EXTRA_MARKER, true, true, true);
        event.slow_ms = 0;
        assert_eq!(classify(event), HookAction::Swallow { count_marker: true });
    }

    #[test]
    fn unrelated_keys_pass_through() {
        assert_eq!(classify(event(0, true, false, false)), HookAction::Pass);
    }

    #[test]
    fn load_constant_is_tens_of_thousands() {
        const _: () = assert!(LOAD_EVENTS >= 20_000);
        const _: () = assert!(LOAD_EVENTS.is_multiple_of(2));
    }

    #[test]
    fn passing_sample_confirms_the_system_design_without_the_fallback() {
        let sample = passing_sample();
        let decision = judge(&sample);
        assert!(decision.pass, "{:?}", decision.reasons);
        assert_eq!(decision.design, Design::Confirm);
        assert_eq!(decision.fallback, Fallback::NotNeeded);
        assert!(decision.injected_flag);
        assert!(decision.passed_through);
        assert!(decision.liveness_within_budget);
        let report = decision.report(&sample);
        assert!(report.contains("result=pass"));
        assert!(report.contains("design=confirm"));
        assert!(report.contains("fallback-needed=false"));
        assert!(report.contains("injected-flag=true"));
        assert!(report.contains("passed-through=true"));
        assert!(report.contains("liveness-within-500ms=true"));
        assert!(report.contains("medium-sees-elevated=0"));
        assert!(report.contains(&format!("load-sent={LOAD_EVENTS}")));
    }

    #[test]
    fn medium_hook_seeing_an_elevated_window_rejects_the_design() {
        let mut sample = passing_sample();
        sample.elevated_medium_seen = VISIBILITY_KEYS;
        let decision = judge(&sample);
        assert!(!decision.pass);
        assert_eq!(decision.design, Design::Reject);
        assert_eq!(decision.fallback, Fallback::NotNeeded);
    }

    #[test]
    fn system_miss_with_a_high_hook_hit_needs_the_fallback() {
        let mut sample = passing_sample();
        sample.elevated_system_seen = 0;
        sample.elevated_system_injected = 0;
        let decision = judge(&sample);
        assert!(!decision.pass);
        assert_eq!(decision.design, Design::Reject);
        assert_eq!(decision.fallback, Fallback::Needed);
        let report = decision.report(&sample);
        assert!(report.contains("fallback-needed=true"));
        assert!(report.contains("design=reject"));
        assert!(report.contains("result=fail"));
    }

    #[test]
    fn both_hooks_missing_leaves_the_fallback_unresolved() {
        let mut sample = passing_sample();
        sample.elevated_system_seen = 0;
        sample.elevated_system_injected = 0;
        sample.elevated_hook_seen = 0;
        sample.elevated_window = 0;
        let decision = judge(&sample);
        assert_eq!(decision.design, Design::Reject);
        assert_eq!(decision.fallback, Fallback::Unresolved);
        assert!(
            decision
                .report(&sample)
                .contains("fallback-needed=unresolved")
        );
    }

    #[test]
    fn a_dropped_injected_flag_fails_even_when_the_key_was_seen() {
        let mut sample = passing_sample();
        sample.elevated_system_injected = 0;
        let decision = judge(&sample);
        assert!(!decision.pass);
        assert!(!decision.injected_flag);
        assert_eq!(decision.design, Design::Confirm);
    }

    #[test]
    fn swallowed_keys_fail_the_pass_through_check() {
        let mut sample = passing_sample();
        sample.elevated_window = 0;
        let decision = judge(&sample);
        assert!(!decision.pass);
        assert!(!decision.passed_through);
        assert_eq!(decision.design, Design::Reject);
    }

    #[test]
    fn a_dead_medium_hook_cannot_explain_a_zero_elevated_count() {
        let mut sample = passing_sample();
        sample.control_medium_seen = 0;
        let decision = judge(&sample);
        assert!(!decision.pass);
        assert!(
            decision
                .reasons
                .iter()
                .any(|reason| matches!(reason, Reason::MediumControl { .. }))
        );
    }

    #[test]
    fn load_must_keep_the_hook_alive_and_count_every_event() {
        let mut dropped = passing_sample();
        dropped.load_seen = LOAD_EVENTS - 1;
        assert!(!judge(&dropped).pass);

        let mut dead = passing_sample();
        dead.hook_alive_after_load = false;
        assert!(!judge(&dead).pass);

        let mut small = passing_sample();
        small.load_sent = 100;
        small.load_seen = 100;
        assert!(!judge(&small).pass);
    }

    #[test]
    fn liveness_budget_is_500_ms_and_requires_removal() {
        let mut on_budget = passing_sample();
        on_budget.liveness_detect_ms = 500;
        assert!(judge(&on_budget).pass);

        let mut late_only = passing_sample();
        late_only.liveness_detect_ms = 501;
        let decision = judge(&late_only);
        assert!(!decision.pass);
        assert!(!decision.measured);
        assert!(!decision.liveness_within_budget);
        assert_eq!(decision.exit_code(), 1);
        assert!(
            decision
                .reasons
                .iter()
                .any(|reason| matches!(reason, Reason::LivenessLate { detect_ms: 501 }))
        );
        assert!(decision.report(&late_only).contains("result=fail"));
    }

    #[test]
    fn removal_never_happened_is_measured_and_exits_zero() {
        let mut sample = passing_sample();
        sample.liveness_removed = false;
        let decision = judge(&sample);
        assert!(!decision.pass);
        assert!(decision.measured);
        assert_eq!(decision.exit_code(), 0);
        assert!(!decision.liveness_within_budget);
        assert!(
            decision
                .reasons
                .iter()
                .all(|reason| matches!(reason, Reason::HookNotRemoved))
        );
        assert!(decision.report(&sample).contains("result=measured"));
        assert!(!decision.report(&sample).contains("result=fail"));

        let mut also_dropped = sample;
        also_dropped.load_seen = 0;
        let decision = judge(&also_dropped);
        assert!(!decision.measured);
        assert_eq!(decision.exit_code(), 1);
        assert!(decision.report(&also_dropped).contains("result=fail"));
    }

    #[test]
    fn system_hook_in_session_0_fails() {
        let mut sample = passing_sample();
        sample.system_session = 0;
        assert!(!judge(&sample).pass);
    }

    #[test]
    fn hook_callback_source_does_no_io() {
        let source = include_str!("win/callback.rs");
        for forbidden in [
            "println!",
            "eprintln!",
            "writeln!",
            "print!",
            "format!",
            "OpenOptions",
            "File::",
            "std::fs",
            "std::io",
            "log_line",
            "Vec",
            "String",
            "Box::",
        ] {
            assert!(
                !source.contains(forbidden),
                "hook callback source contains {forbidden}"
            );
        }
        assert_eq!(
            source.matches("Sleep(").count(),
            1,
            "the blocking wait is only the slow-callback removal probe"
        );
    }
}
