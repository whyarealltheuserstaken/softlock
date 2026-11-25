//! Display changes for seam 3: the runner's identity, monitor rectangles, a
//! resolution change that restores itself, and attaching or detaching an
//! output that the image already has. Nothing here installs a driver.

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    CDS_NORESET, CDS_TYPE, CDS_UPDATEREGISTRY, ChangeDisplaySettingsExW, DEVMODEW,
    DISP_CHANGE_SUCCESSFUL, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICE_MIRRORING_DRIVER,
    DISPLAY_DEVICEW, DM_PELSHEIGHT, DM_PELSWIDTH, DM_POSITION, ENUM_CURRENT_SETTINGS,
    ENUM_DISPLAY_SETTINGS_MODE, EnumDisplayDevicesW, EnumDisplayMonitors, EnumDisplaySettingsW,
    HDC, HMONITOR,
};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
use windows::core::{BOOL, PCWSTR, w};

use super::error::HarnessError;
use crate::coverage::ScreenRect;

/// Product name and build from the registry, printed so a log says which
/// family (Windows 10 or Windows 11 server base) a run covered.
#[derive(Clone, Debug)]
pub struct OsIdentity {
    pub product: String,
    pub display_version: String,
    pub build: String,
}

pub fn os_identity() -> OsIdentity {
    const KEY: PCWSTR = w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
    OsIdentity {
        product: read_sz(KEY, w!("ProductName")).unwrap_or_default(),
        display_version: read_sz(KEY, w!("DisplayVersion")).unwrap_or_default(),
        build: read_sz(KEY, w!("CurrentBuild")).unwrap_or_default(),
    }
}

fn read_sz(key: PCWSTR, value: PCWSTR) -> Option<String> {
    let mut buffer = [0u16; 256];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key,
            value,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let units = (size as usize / 2).min(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..units]);
    Some(text.trim_end_matches('\0').to_string())
}

pub fn monitor_rects() -> Vec<ScreenRect> {
    let mut rects: Vec<ScreenRect> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(monitor_proc),
            LPARAM(&mut rects as *mut Vec<ScreenRect> as isize),
        );
    }
    rects
}

unsafe extern "system" fn monitor_proc(
    _monitor: HMONITOR,
    _hdc: HDC,
    rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    unsafe {
        if let Some(rect) = rect.as_ref() {
            (*(data.0 as *mut Vec<ScreenRect>)).push(screen(rect));
        }
    }
    BOOL(1)
}

pub fn window_rect(hwnd: isize) -> Option<ScreenRect> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(HWND(hwnd as *mut _), &mut rect) }.ok()?;
    Some(screen(&rect))
}

fn screen(rect: &RECT) -> ScreenRect {
    ScreenRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

/// A dynamic (not saved) mode change on the primary display. Dropping it
/// restores the mode stored in the registry.
pub struct ResolutionChange {
    pub from: (u32, u32),
    pub to: (u32, u32),
}

impl Drop for ResolutionChange {
    fn drop(&mut self) {
        unsafe {
            let _ = ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE(0), None);
        }
    }
}

/// Switches the primary display to another listed mode with the same colour
/// depth. `Ok(None)` means the driver lists no other usable mode.
pub fn change_primary_resolution() -> Result<Option<ResolutionChange>, HarnessError> {
    let current = current_mode(PCWSTR::null())
        .ok_or_else(|| HarnessError::new("EnumDisplaySettingsW current mode failed"))?;
    let from = (current.dmPelsWidth, current.dmPelsHeight);
    let mut index = 0u32;
    let mut target = None;
    loop {
        let mut mode = blank_mode();
        let listed = unsafe {
            EnumDisplaySettingsW(PCWSTR::null(), ENUM_DISPLAY_SETTINGS_MODE(index), &mut mode)
        };
        if !listed.as_bool() {
            break;
        }
        index += 1;
        let size = (mode.dmPelsWidth, mode.dmPelsHeight);
        if size != from
            && mode.dmBitsPerPel == current.dmBitsPerPel
            && mode.dmPelsWidth >= 800
            && mode.dmPelsHeight >= 600
        {
            target = Some(mode);
            break;
        }
    }
    let Some(mut mode) = target else {
        return Ok(None);
    };
    mode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT;
    let result =
        unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), Some(&mode), None, CDS_TYPE(0), None) };
    if result != DISP_CHANGE_SUCCESSFUL {
        return Err(HarnessError::new(format!(
            "ChangeDisplaySettingsExW {}x{} returned {}",
            mode.dmPelsWidth, mode.dmPelsHeight, result.0
        )));
    }
    Ok(Some(ResolutionChange {
        from,
        to: (mode.dmPelsWidth, mode.dmPelsHeight),
    }))
}

/// An output the image has that is not part of the desktop. On a hosted
/// runner this is normally absent: there is no virtual display driver.
pub fn detached_output() -> Option<String> {
    let mut index = 0u32;
    loop {
        let mut device = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &mut device, 0) }.as_bool() {
            return None;
        }
        index += 1;
        let flags = device.StateFlags;
        if flags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP == DISPLAY_DEVICE_ATTACHED_TO_DESKTOP
            || flags & DISPLAY_DEVICE_MIRRORING_DRIVER == DISPLAY_DEVICE_MIRRORING_DRIVER
        {
            continue;
        }
        let end = device
            .DeviceName
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(device.DeviceName.len());
        return Some(String::from_utf16_lossy(&device.DeviceName[..end]));
    }
}

/// An output attached to the right of the desktop. Dropping it detaches the
/// output again.
pub struct AttachedOutput {
    name: Vec<u16>,
    detached: bool,
}

impl AttachedOutput {
    /// Takes the output off the desktop, as unplugging would.
    pub fn detach(&mut self) -> Result<(), HarnessError> {
        let mut mode = blank_mode();
        mode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_POSITION;
        apply(&self.name, &mode)?;
        self.detached = true;
        Ok(())
    }
}

impl Drop for AttachedOutput {
    fn drop(&mut self) {
        if !self.detached {
            let _ = self.detach();
        }
    }
}

pub fn attach_output(name: &str, right_of: i32) -> Result<AttachedOutput, HarnessError> {
    let mut wide: Vec<u16> = name.encode_utf16().collect();
    wide.push(0);
    let mut mode = blank_mode();
    mode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_POSITION;
    mode.dmPelsWidth = 1024;
    mode.dmPelsHeight = 768;
    mode.Anonymous1.Anonymous2.dmPosition.x = right_of;
    mode.Anonymous1.Anonymous2.dmPosition.y = 0;
    apply(&wide, &mode)?;
    Ok(AttachedOutput {
        name: wide,
        detached: false,
    })
}

fn apply(name: &[u16], mode: &DEVMODEW) -> Result<(), HarnessError> {
    let staged = unsafe {
        ChangeDisplaySettingsExW(
            PCWSTR(name.as_ptr()),
            Some(mode),
            None,
            CDS_UPDATEREGISTRY | CDS_NORESET,
            None,
        )
    };
    if staged != DISP_CHANGE_SUCCESSFUL {
        return Err(HarnessError::new(format!(
            "ChangeDisplaySettingsExW stage returned {}",
            staged.0
        )));
    }
    let applied =
        unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE(0), None) };
    if applied != DISP_CHANGE_SUCCESSFUL {
        return Err(HarnessError::new(format!(
            "ChangeDisplaySettingsExW apply returned {}",
            applied.0
        )));
    }
    Ok(())
}

fn current_mode(device: PCWSTR) -> Option<DEVMODEW> {
    let mut mode = blank_mode();
    unsafe { EnumDisplaySettingsW(device, ENUM_CURRENT_SETTINGS, &mut mode) }
        .as_bool()
        .then_some(mode)
}

fn blank_mode() -> DEVMODEW {
    DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    }
}
