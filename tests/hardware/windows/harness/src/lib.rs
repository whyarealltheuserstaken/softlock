//! Decisions for the Windows hardware suite. No operating-system calls.

mod capture;
mod display;
mod failclosed;
mod hello;
mod hookdec;
mod machine;
mod plan;
mod policy;
mod power;
mod redact;
mod report;

pub use capture::{ABSENT_PIXELS, CONTROL_PIXELS, PRESENT_PIXELS, PixelCounts, judge};
pub use display::{DisplayStep, DisplayWatch, display_step_seen};
pub use failclosed::{finish_state, lock_blocked, within_one_second};
pub use hello::{Action, DeviceEvent, InputOrigin, Modifiers, PassthroughWindow, decide};
pub use hookdec::{EXTRA_LOAD, EXTRA_MARKER, EXTRA_VIS, HookAction, HookEvent, classify};
pub use machine::{HelloKind, Machine, SessionKind, decode_facts};
pub use plan::{Action as RowAction, BUDGET_MS, Flags, Planned, plan, plan_value, profile_name};
pub use policy::{Restore, hkcu_value_allowed, hklm_value_allowed, restore_dword, restore_saved};
pub use power::{
    PowerAvailability, SettingIndex, parse_powercfg_a, parse_scheme_guid, parse_setting_index,
    section_contains,
};
pub use redact::redact;
pub use report::{Outcome, Report, SCHEMA, conform, render};

pub const SUITE_NAME: &str = "windows";
