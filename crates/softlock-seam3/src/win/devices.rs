use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_DEVNODE_STATUS_FLAGS, CM_Get_DevNode_Status, CM_PROB, CM_PROB_DISABLED, CR_SUCCESS,
    DIGCF_ALLCLASSES, DIGCF_PRESENT, SP_DEVINFO_DATA, SetupDiDestroyDeviceInfoList,
    SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW,
};

use crate::model::DeviceSnapshot;

use super::error::{HarnessError, explain};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub instance_id: String,
    pub enabled: bool,
}

pub fn snapshot_devices() -> Result<DeviceSnapshot, HarnessError> {
    let list = unsafe {
        SetupDiGetClassDevsW(None, None, None, DIGCF_ALLCLASSES | DIGCF_PRESENT)
            .map_err(|error| explain(error, "SetupDiGetClassDevsW"))?
    };
    let mut devices = DeviceSnapshot::default();
    let mut index = 0u32;
    loop {
        let mut info = SP_DEVINFO_DATA {
            cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
            ..SP_DEVINFO_DATA::default()
        };
        if unsafe { SetupDiEnumDeviceInfo(list, index, &mut info) }.is_err() {
            break;
        }
        index += 1;
        let Some(instance_id) = instance_id(list, &info)? else {
            continue;
        };
        let mut status = CM_DEVNODE_STATUS_FLAGS(0);
        let mut problem = CM_PROB(0);
        let code = unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, info.DevInst, 0) };
        if code != CR_SUCCESS {
            continue;
        }
        let enabled = problem != CM_PROB_DISABLED;
        devices.devices.insert(instance_id, enabled);
    }
    unsafe {
        let _ = SetupDiDestroyDeviceInfoList(list);
    }
    if devices.devices.is_empty() {
        return Err(HarnessError::new("device snapshot was empty"));
    }
    Ok(devices)
}

fn instance_id(
    list: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
    info: &SP_DEVINFO_DATA,
) -> Result<Option<String>, HarnessError> {
    let mut chars = vec![0u16; 512];
    let mut required = 0u32;
    let read =
        unsafe { SetupDiGetDeviceInstanceIdW(list, info, Some(&mut chars), Some(&mut required)) };
    if read.is_err() && required as usize > chars.len() {
        chars.resize(required as usize, 0);
        unsafe {
            SetupDiGetDeviceInstanceIdW(list, info, Some(&mut chars), Some(&mut required))
                .map_err(|error| explain(error, "SetupDiGetDeviceInstanceIdW"))?;
        }
    } else {
        read.map_err(|error| explain(error, "SetupDiGetDeviceInstanceIdW"))?;
    }
    let end = chars
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(chars.len());
    let id = String::from_utf16_lossy(&chars[..end]);
    if id.is_empty() {
        Ok(None)
    } else {
        Ok(Some(id))
    }
}
