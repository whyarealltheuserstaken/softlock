//! Real heat signals. Cooling mode, an MHz cap, and process throttling are not read here.
//!
//! `% Passive Limit` below 100 and `Throttle Reasons` bit 0 come from the
//! Thermal Zone Information counters. Zone temperature comes from WMI
//! `MSAcpi_ThermalZoneTemperature` (often admin-only). Battery temperature
//! comes from `IOCTL_BATTERY_QUERY_INFORMATION` / `BatteryTemperature`.
//! A failed or implausible source stays unavailable and does not trip the guard.

use std::mem::size_of;

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoSetProxyBlanket, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
    PDH_FMT_LONG, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery,
    PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
};
use windows::Win32::System::Power::{
    BATTERY_QUERY_INFORMATION, BatteryTemperature, GUID_DEVICE_BATTERY,
    IOCTL_BATTERY_QUERY_INFORMATION, IOCTL_BATTERY_QUERY_TAG,
};
use windows::Win32::System::Variant::{
    VARIANT, VT_I2, VT_I4, VT_INT, VT_UI2, VT_UI4, VT_UINT, VariantClear,
};
use windows::Win32::System::Wmi::{
    IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY,
    WBEM_S_TIMEDOUT, WbemLocator,
};
use windows::core::{BSTR, PCWSTR, w};

use crate::guard::{
    plausible_battery_dc, plausible_passive_limit, plausible_throttle_reasons, plausible_zone_dc,
};
use crate::signals::PowerSignals;

/// `IEnumWbemClassObject::Next` waits at most this long. A timeout leaves the zone unavailable.
const WMI_QUERY_TIMEOUT_MS: i32 = 2_000;

const RPC_C_AUTHN_WINNT: u32 = 10;
const RPC_C_AUTHZ_NONE: u32 = 0;

pub struct HeatReader {
    query: Option<PDH_HQUERY>,
    passive: Option<PDH_HCOUNTER>,
    reasons: Option<PDH_HCOUNTER>,
    wmi: Option<IWbemServices>,
}

impl HeatReader {
    pub fn open() -> Self {
        let (query, passive, reasons) = open_counters();
        let wmi = open_wmi();
        Self {
            query,
            passive,
            reasons,
            wmi,
        }
    }

    pub fn sample_into(&self, signals: &mut PowerSignals) {
        signals.heat_checked = true;
        if let Some(query) = self.query {
            // SAFETY: the query and counters were created together and are closed in Drop.
            let _ = unsafe { PdhCollectQueryData(query) };
            if let Some(counter) = self.passive {
                signals.passive_limit = counter_doubles(counter)
                    .into_iter()
                    .filter_map(plausible_passive_limit)
                    .min();
            }
            if let Some(counter) = self.reasons {
                let mut combined = None;
                for value in counter_longs(counter) {
                    if let Some(reasons) = plausible_throttle_reasons(value) {
                        combined = Some(combined.unwrap_or(0) | reasons);
                    }
                }
                signals.throttle_reasons = combined;
            }
        }
        if let Some(services) = &self.wmi {
            signals.zone_dc = hottest_zone(services);
        }
        signals.battery_dc = hottest_battery();
    }
}

impl Drop for HeatReader {
    fn drop(&mut self) {
        if let Some(query) = self.query {
            // SAFETY: this is the query opened in `open`, closed once.
            let _ = unsafe { PdhCloseQuery(query) };
        }
    }
}

fn open_counters() -> (
    Option<PDH_HQUERY>,
    Option<PDH_HCOUNTER>,
    Option<PDH_HCOUNTER>,
) {
    let mut query = PDH_HQUERY::default();
    // SAFETY: a null data source opens a real-time query. The handle is stored and closed on drop.
    if unsafe { PdhOpenQueryW(None, 0, &mut query) } != 0 {
        return (None, None, None);
    }
    let passive = add_counter(query, w!(r"\Thermal Zone Information(*)\% Passive Limit"));
    let reasons = add_counter(query, w!(r"\Thermal Zone Information(*)\Throttle Reasons"));
    // Two collects let a counter that needs a previous sample produce a value.
    unsafe {
        let _ = PdhCollectQueryData(query);
        let _ = PdhCollectQueryData(query);
    }
    (Some(query), passive, reasons)
}

fn add_counter(query: PDH_HQUERY, path: PCWSTR) -> Option<PDH_HCOUNTER> {
    let mut counter = PDH_HCOUNTER::default();
    // SAFETY: `query` is an open PDH query and `path` is a constant wide string.
    if unsafe { PdhAddEnglishCounterW(query, path, 0, &mut counter) } == 0 {
        Some(counter)
    } else {
        None
    }
}

fn counter_doubles(counter: PDH_HCOUNTER) -> Vec<f64> {
    let items = counter_items(counter, PDH_FMT_DOUBLE);
    items
        .into_iter()
        .filter_map(|item| {
            let status = item.FmtValue.CStatus;
            if status != PDH_CSTATUS_VALID_DATA && status != PDH_CSTATUS_NEW_DATA {
                return None;
            }
            // SAFETY: PDH_FMT_DOUBLE asks for the double member of this union.
            let value = unsafe { item.FmtValue.Anonymous.doubleValue };
            value.is_finite().then_some(value)
        })
        .collect()
}

fn counter_longs(counter: PDH_HCOUNTER) -> Vec<i64> {
    let items = counter_items(counter, PDH_FMT_LONG);
    items
        .into_iter()
        .filter_map(|item| {
            let status = item.FmtValue.CStatus;
            if status != PDH_CSTATUS_VALID_DATA && status != PDH_CSTATUS_NEW_DATA {
                return None;
            }
            // SAFETY: PDH_FMT_LONG asks for the long member of this union.
            Some(i64::from(unsafe { item.FmtValue.Anonymous.longValue }))
        })
        .collect()
}

fn counter_items(
    counter: PDH_HCOUNTER,
    format: windows::Win32::System::Performance::PDH_FMT,
) -> Vec<PDH_FMT_COUNTERVALUE_ITEM_W> {
    let mut size = 0u32;
    let mut count = 0u32;
    // SAFETY: a null item buffer asks PDH for the size it needs.
    let first =
        unsafe { PdhGetFormattedCounterArrayW(counter, format, &mut size, &mut count, None) };
    if first != PDH_MORE_DATA || size == 0 || count == 0 {
        return Vec::new();
    }
    let mut buffer = vec![0u8; size as usize];
    let second = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            format,
            &mut size,
            &mut count,
            Some(buffer.as_mut_ptr().cast()),
        )
    };
    if second != 0 || count == 0 {
        return Vec::new();
    }
    let bytes = count as usize * size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
    if buffer.len() < bytes {
        return Vec::new();
    }
    // SAFETY: PDH wrote `count` items into `buffer`, and the values are copied out before the buffer is dropped.
    let items = unsafe {
        std::slice::from_raw_parts(
            buffer.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
            count as usize,
        )
    };
    items.to_vec()
}

fn open_wmi() -> Option<IWbemServices> {
    // SAFETY: COM is initialized on this thread by the UI probe before the reader is opened.
    let locator = unsafe {
        CoCreateInstance::<_, IWbemLocator>(&WbemLocator, None, CLSCTX_INPROC_SERVER).ok()?
    };
    let namespace = BSTR::from("root\\wmi");
    let blank = BSTR::new();
    let services = unsafe {
        locator
            .ConnectServer(&namespace, &blank, &blank, &blank, 0, &blank, None)
            .ok()?
    };
    // A non-admin token is refused by this class. Failure leaves the zone unavailable.
    let _ = unsafe {
        CoSetProxyBlanket(
            &services,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
    };
    Some(services)
}

fn hottest_zone(services: &IWbemServices) -> Option<i32> {
    let language = BSTR::from("WQL");
    let query = BSTR::from("SELECT CurrentTemperature FROM MSAcpi_ThermalZoneTemperature");
    // SAFETY: `services` is a connected local namespace. A failed query means the class is unreadable.
    let enumerator = unsafe {
        services
            .ExecQuery(
                &language,
                &query,
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )
            .ok()?
    };
    let mut hottest = None;
    loop {
        let mut objects = [None];
        let mut returned = 0u32;
        let hr = unsafe { enumerator.Next(WMI_QUERY_TIMEOUT_MS, &mut objects, &mut returned) };
        // A timeout is a success code. The zone reading is unavailable rather than a partial result.
        if hr == windows::core::HRESULT(WBEM_S_TIMEDOUT.0) {
            return None;
        }
        if hr.is_err() || returned == 0 {
            break;
        }
        let Some(object) = objects[0].take() else {
            break;
        };
        let mut value = VARIANT::default();
        // SAFETY: CurrentTemperature is a numeric WMI property. The variant is cleared below.
        if unsafe { object.Get(w!("CurrentTemperature"), 0, &mut value, None, None) }.is_ok()
            && let Some(raw) = variant_u32(&value)
            && let Some(dc) = plausible_zone_dc(raw)
        {
            hottest = Some(hottest.map_or(dc, |hot: i32| hot.max(dc)));
        }
        let _ = unsafe { VariantClear(&mut value) };
    }
    hottest
}

fn variant_u32(value: &VARIANT) -> Option<u32> {
    // SAFETY: WMI filled this variant. Only the numeric arm that matches the type tag is read.
    unsafe {
        let inner = &*value.Anonymous.Anonymous;
        match inner.vt {
            VT_I2 => (inner.Anonymous.iVal >= 0).then_some(inner.Anonymous.iVal as u32),
            VT_UI2 => Some(u32::from(inner.Anonymous.uiVal)),
            VT_I4 | VT_INT => (inner.Anonymous.lVal >= 0).then_some(inner.Anonymous.lVal as u32),
            VT_UI4 | VT_UINT => Some(inner.Anonymous.ulVal),
            _ => None,
        }
    }
}

fn hottest_battery() -> Option<i32> {
    // SAFETY: the battery device interface list is destroyed before this function returns.
    let list = unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVICE_BATTERY),
            None,
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
        .ok()?
    };
    let mut hottest = None;
    let mut index = 0u32;
    loop {
        let mut interface = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..SP_DEVICE_INTERFACE_DATA::default()
        };
        if unsafe {
            SetupDiEnumDeviceInterfaces(list, None, &GUID_DEVICE_BATTERY, index, &mut interface)
        }
        .is_err()
        {
            break;
        }
        index += 1;
        if let Some(dc) = battery_dc(list, &interface) {
            hottest = Some(hottest.map_or(dc, |hot: i32| hot.max(dc)));
        }
    }
    let _ = unsafe { SetupDiDestroyDeviceInfoList(list) };
    hottest
}

fn battery_dc(list: HDEVINFO, interface: &SP_DEVICE_INTERFACE_DATA) -> Option<i32> {
    let mut required = 0u32;
    let _ = unsafe {
        SetupDiGetDeviceInterfaceDetailW(list, interface, None, 0, Some(&mut required), None)
    };
    if required < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 {
        return None;
    }
    let mut buffer = vec![0u8; required as usize];
    let detail = buffer
        .as_mut_ptr()
        .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    // SAFETY: the buffer is large enough for the detail header plus the device path SetupAPI asked for.
    unsafe {
        (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        SetupDiGetDeviceInterfaceDetailW(list, interface, Some(detail), required, None, None)
            .ok()?;
    }
    let path = PCWSTR(unsafe { std::ptr::addr_of!((*detail).DevicePath).cast() });
    // SAFETY: `path` points into `buffer`, which stays alive until the handle is closed.
    let handle = unsafe {
        CreateFileW(
            path,
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .ok()?
    };
    let temp = query_battery_temperature(handle);
    let _ = unsafe { CloseHandle(handle) };
    temp
}

fn query_battery_temperature(handle: HANDLE) -> Option<i32> {
    let wait = 0u32;
    let mut tag = 0u32;
    let mut returned = 0u32;
    // SAFETY: the tag and temperature buffers are sized for the documented ULONG results.
    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_BATTERY_QUERY_TAG,
            Some((&wait as *const u32).cast()),
            size_of::<u32>() as u32,
            Some((&mut tag as *mut u32).cast()),
            size_of::<u32>() as u32,
            Some(&mut returned),
            None,
        )
        .ok()?;
    }
    if tag == 0 {
        return None;
    }
    let query = BATTERY_QUERY_INFORMATION {
        BatteryTag: tag,
        InformationLevel: BatteryTemperature,
        AtRate: 0,
    };
    let mut kelvin = 0u32;
    unsafe {
        DeviceIoControl(
            handle,
            IOCTL_BATTERY_QUERY_INFORMATION,
            Some((&query as *const BATTERY_QUERY_INFORMATION).cast()),
            size_of::<BATTERY_QUERY_INFORMATION>() as u32,
            Some((&mut kelvin as *mut u32).cast()),
            size_of::<u32>() as u32,
            Some(&mut returned),
            None,
        )
        .ok()?;
    }
    plausible_battery_dc(kelvin)
}
