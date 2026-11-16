//! Windows power requests, machine report, and the per-tick battery and thermal sample.

use std::cell::Cell;
use std::mem::size_of;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree, NTSTATUS, WIN32_ERROR};
use windows::Win32::System::Power::{
    CallNtPowerInformation, ES_CONTINUOUS, EXECUTION_STATE, GetPwrCapabilities,
    GetSystemPowerStatus, PO_TZ_ACTIVE, PO_TZ_PASSIVE, PROCESSOR_POWER_INFORMATION,
    PowerClearRequest, PowerCreateRequest, PowerGetActiveScheme, PowerReadACValueIndex,
    PowerReadDCValueIndex, PowerRequestDisplayRequired, PowerRequestSystemRequired,
    PowerSetRequest, ProcessorInformation, SYSTEM_POWER_CAPABILITIES, SYSTEM_POWER_INFORMATION,
    SYSTEM_POWER_STATUS, SetThreadExecutionState, SystemPowerInformation,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessInformation, POWER_REQUEST_CONTEXT_SIMPLE_STRING,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, REASON_CONTEXT, REASON_CONTEXT_0,
};
use windows::core::{GUID, PWSTR};

use softlock_core::PRODUCT_DISPLAY_NAME;

use crate::plan::{MachineView, classify_ac_line, spike_power_plan};
use crate::signals::{BatteryFlag, Cooling, PowerSignals};

const BUTTON_SUBGROUP: GUID = GUID::from_u128(0x4f971e89_eebd_4455_a8de_9e59040e7347);
const LID_CLOSE_ACTION: GUID = GUID::from_u128(0x5ca83367_6e45_459f_a27b_476b1d01c936);

pub struct PowerHold {
    request: HANDLE,
    flags: EXECUTION_STATE,
    _reason: Box<[u16]>,
    warned: Cell<bool>,
    held: bool,
}

impl PowerHold {
    pub fn acquire() -> Result<Self, String> {
        let plan = spike_power_plan();
        let flags = EXECUTION_STATE(plan.execution_state);
        // SAFETY: the flag value is the documented continuous | system-required pair.
        let previous = unsafe { SetThreadExecutionState(flags) };
        if previous.0 == 0 {
            return Err("Could not hold SetThreadExecutionState.".to_string());
        }
        let text = format!("{PRODUCT_DISPLAY_NAME} keep-awake spike");
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let reason = wide.into_boxed_slice();
        let mut context = REASON_CONTEXT {
            Version: 0,
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0::default(),
        };
        // SAFETY: `reason` is a null-terminated UTF-16 buffer. PowerCreateRequest copies it
        // during the call. The box is kept so the allocation stays valid through the call,
        // and moving the box afterwards does not move the heap bytes.
        let request = unsafe {
            context.Reason.SimpleReasonString = PWSTR(reason.as_ptr() as *mut u16);
            match PowerCreateRequest(&context) {
                Ok(request) => request,
                Err(error) => {
                    let _ = SetThreadExecutionState(ES_CONTINUOUS);
                    return Err(format!("Could not create the power request: {error}"));
                }
            }
        };
        if let Err(error) = unsafe { PowerSetRequest(request, PowerRequestSystemRequired) } {
            release_request(request);
            return Err(format!(
                "Could not hold the system-required power request: {error}"
            ));
        }
        if let Err(error) = unsafe { PowerSetRequest(request, PowerRequestDisplayRequired) } {
            let _ = unsafe { PowerClearRequest(request, PowerRequestSystemRequired) };
            release_request(request);
            return Err(format!(
                "Could not hold the display-required power request: {error}"
            ));
        }
        Ok(Self {
            request,
            flags,
            _reason: reason,
            warned: Cell::new(false),
            held: true,
        })
    }

    pub fn is_held(&self) -> bool {
        self.held
    }

    pub fn refresh(&self) {
        // SAFETY: same flags that were accepted in `acquire`, called on the thread that set them.
        let previous = unsafe { SetThreadExecutionState(self.flags) };
        if previous.0 == 0 && !self.warned.replace(true) {
            eprintln!("Could not refresh SetThreadExecutionState.");
        }
    }
}

impl Drop for PowerHold {
    fn drop(&mut self) {
        // SAFETY: the handle came from PowerCreateRequest and both request types were set.
        unsafe {
            let _ = PowerClearRequest(self.request, PowerRequestDisplayRequired);
            let _ = PowerClearRequest(self.request, PowerRequestSystemRequired);
            let _ = CloseHandle(self.request);
            let _ = SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}

fn release_request(request: HANDLE) {
    // SAFETY: the handle came from PowerCreateRequest and the execution state was set on this thread.
    unsafe {
        let _ = CloseHandle(request);
        let _ = SetThreadExecutionState(ES_CONTINUOUS);
    }
}

pub fn read_machine() -> MachineView {
    let mut machine = MachineView::unknown();
    let mut caps = SYSTEM_POWER_CAPABILITIES::default();
    // SAFETY: the struct is zeroed and large enough for the documented layout.
    if unsafe { GetPwrCapabilities(&mut caps) } {
        machine.aoac = Some(caps.AoAc);
        machine.lid_present = Some(caps.LidPresent);
    }
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_ok() {
        machine.power_source = classify_ac_line(status.ACLineStatus);
        machine.battery_percent = percent_or_none(status.BatteryLifePercent);
    }
    let (ac, dc) = read_lid_actions();
    machine.lid_action_ac = ac;
    machine.lid_action_dc = dc;
    machine
}

pub fn read_signals() -> PowerSignals {
    let mut signals = PowerSignals::default();
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_ok() {
        signals.battery_percent = percent_or_none(status.BatteryLifePercent);
        signals.ac_online = match status.ACLineStatus {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
        signals.battery_saver = match status.SystemStatusFlag {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
        signals.battery_flag = Some(BatteryFlag::from_bits(status.BatteryFlag));
    }
    let mut info = SYSTEM_POWER_INFORMATION::default();
    if nt_ok(unsafe {
        CallNtPowerInformation(
            SystemPowerInformation,
            None,
            0,
            Some((&mut info as *mut SYSTEM_POWER_INFORMATION).cast()),
            size_of::<SYSTEM_POWER_INFORMATION>() as u32,
        )
    }) {
        signals.cooling = Some(cooling_from(info.CoolingMode.0));
    }
    if let Some((max, limit, current)) = processor_mhz() {
        signals.mhz_max = Some(max);
        signals.mhz_limit = Some(limit);
        signals.mhz_current = Some(current);
    }
    signals.execution_throttled = process_execution_throttled();
    signals
}

fn percent_or_none(value: u8) -> Option<u8> {
    if value <= 100 { Some(value) } else { None }
}

fn cooling_from(mode: u16) -> Cooling {
    if mode == PO_TZ_PASSIVE.0 {
        Cooling::Passive
    } else if mode == PO_TZ_ACTIVE.0 {
        Cooling::Active
    } else {
        Cooling::Invalid
    }
}

fn processor_mhz() -> Option<(u32, u32, u32)> {
    let count = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(1, 256);
    let mut cpus = vec![PROCESSOR_POWER_INFORMATION::default(); count];
    let bytes = (cpus.len() * size_of::<PROCESSOR_POWER_INFORMATION>()) as u32;
    let status = unsafe {
        CallNtPowerInformation(
            ProcessorInformation,
            None,
            0,
            Some(cpus.as_mut_ptr().cast()),
            bytes,
        )
    };
    if !nt_ok(status) {
        return None;
    }
    let mut max_mhz = 0u32;
    let mut limit = u32::MAX;
    let mut current = u32::MAX;
    let mut saw = false;
    for cpu in &cpus {
        if cpu.MaxMhz == 0 {
            continue;
        }
        saw = true;
        max_mhz = max_mhz.max(cpu.MaxMhz);
        if cpu.MhzLimit > 0 {
            limit = limit.min(cpu.MhzLimit);
        }
        current = current.min(cpu.CurrentMhz);
    }
    if !saw || limit == u32::MAX {
        return None;
    }
    Some((max_mhz, limit, current))
}

fn process_execution_throttled() -> Option<bool> {
    let mut state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ..PROCESS_POWER_THROTTLING_STATE::default()
    };
    let ok = unsafe {
        GetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&mut state as *mut PROCESS_POWER_THROTTLING_STATE).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    };
    ok.ok()?;
    Some(state.StateMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED != 0)
}

fn read_lid_actions() -> (Option<u32>, Option<u32>) {
    let mut scheme = std::ptr::null_mut();
    // SAFETY: a null root key asks for the active scheme. The pointer is freed below.
    let rc = unsafe { PowerGetActiveScheme(None, &mut scheme) };
    if rc != WIN32_ERROR(0) || scheme.is_null() {
        return (None, None);
    }
    let ac = read_lid_index(scheme, true);
    let dc = read_lid_index(scheme, false);
    unsafe {
        let _ = LocalFree(Some(HLOCAL(scheme.cast())));
    }
    (ac, dc)
}

fn read_lid_index(scheme: *mut GUID, ac: bool) -> Option<u32> {
    let mut value = 0u32;
    let rc = unsafe {
        if ac {
            PowerReadACValueIndex(
                None,
                Some(scheme.cast_const()),
                Some(&BUTTON_SUBGROUP),
                Some(&LID_CLOSE_ACTION),
                &mut value,
            )
            .0
        } else {
            PowerReadDCValueIndex(
                None,
                Some(scheme.cast_const()),
                Some(&BUTTON_SUBGROUP),
                Some(&LID_CLOSE_ACTION),
                &mut value,
            )
        }
    };
    if rc == 0 { Some(value) } else { None }
}

fn nt_ok(status: NTSTATUS) -> bool {
    status.0 >= 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{
        ES_CONTINUOUS as PLAN_CONTINUOUS, ES_SYSTEM_REQUIRED as PLAN_SYSTEM, KEY_DOWN_MESSAGE,
        SYSKEY_DOWN_MESSAGE, overlay_ex_style, overlay_style, spike_power_plan,
    };
    use windows::Win32::System::Power::ES_SYSTEM_REQUIRED;
    use windows::Win32::UI::WindowsAndMessaging::{
        WM_KEYDOWN, WM_SYSKEYDOWN, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    #[test]
    fn plan_matches_the_windows_headers() {
        let plan = spike_power_plan();
        assert_eq!((ES_CONTINUOUS | ES_SYSTEM_REQUIRED).0, plan.execution_state);
        assert_eq!(PLAN_CONTINUOUS | PLAN_SYSTEM, plan.execution_state);
        let style =
            WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE;
        assert_eq!(style.0, overlay_ex_style());
        assert_eq!(WS_POPUP.0, overlay_style());
        assert_eq!(WM_KEYDOWN, KEY_DOWN_MESSAGE);
        assert_eq!(WM_SYSKEYDOWN, SYSKEY_DOWN_MESSAGE);
    }

    #[test]
    fn power_requests_can_be_held_and_released() {
        let hold = PowerHold::acquire().expect("power requests");
        hold.refresh();
        drop(hold);
    }

    #[test]
    fn machine_and_signal_reads_return() {
        let machine = read_machine();
        let signals = read_signals();
        let _ = (machine.aoac, signals.battery_percent, signals.cooling);
    }
}
