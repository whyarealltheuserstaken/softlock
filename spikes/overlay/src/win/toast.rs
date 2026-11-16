//! A short banner so the toast step has something to photograph.

use std::sync::Mutex;

use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{IShellLinkW, SetCurrentProcessExplicitAppUserModelID, ShellLink};
use windows::core::{GUID, HSTRING, Interface, PCWSTR, PWSTR};

use crate::PRODUCT_DISPLAY_NAME;
use crate::tour::toast_xml;
use crate::win::wide;

const AUMID: &str = "com.softlock.spike.overlay";

static SHORTCUT: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

pub fn show() -> Result<(), String> {
    register_shortcut()?;
    let xml_text = toast_xml(PRODUCT_DISPLAY_NAME);
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID));
        let document = XmlDocument::new().map_err(|error| error.to_string())?;
        document
            .LoadXml(&HSTRING::from(xml_text))
            .map_err(|error| error.to_string())?;
        let toast = ToastNotification::CreateToastNotification(&document)
            .map_err(|error| error.to_string())?;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
            .map_err(|error| error.to_string())?;
        notifier.Show(&toast).map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn remove_shortcut() {
    if let Ok(mut slot) = SHORTCUT.lock()
        && let Some(path) = slot.take()
    {
        let _ = std::fs::remove_file(path);
    }
}

fn register_shortcut() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let appdata = std::env::var("APPDATA").map_err(|_| "APPDATA is not set".to_string())?;
    let mut path = std::path::PathBuf::from(appdata);
    path.push("Microsoft");
    path.push("Windows");
    path.push("Start Menu");
    path.push("Programs");
    std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    path.push("spike-overlay.lnk");
    let exe_w = wide(&exe.to_string_lossy());
    let link_w = wide(&path.to_string_lossy());
    let aumid = wide(AUMID);
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(
            &ShellLink,
            None::<&windows::core::IUnknown>,
            CLSCTX_INPROC_SERVER,
        )
        .map_err(|error| error.to_string())?;
        link.SetPath(PCWSTR(exe_w.as_ptr()))
            .map_err(|error| error.to_string())?;
        let store: IPropertyStore = link.cast().map_err(|error| error.to_string())?;
        let key = PROPERTYKEY {
            fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
            pid: 5,
        };
        let mut variant = PROPVARIANT::default();
        {
            let header = &mut *variant.Anonymous.Anonymous;
            header.vt = VT_LPWSTR;
            header.Anonymous.pwszVal = PWSTR(aumid.as_ptr() as *mut u16);
        }
        store
            .SetValue(&key, &variant)
            .map_err(|error| error.to_string())?;
        store.Commit().map_err(|error| error.to_string())?;
        (*variant.Anonymous.Anonymous).vt = windows::Win32::System::Variant::VT_EMPTY;
        let file: IPersistFile = link.cast().map_err(|error| error.to_string())?;
        file.Save(PCWSTR(link_w.as_ptr()), true)
            .map_err(|error| error.to_string())?;
    }
    if let Ok(mut slot) = SHORTCUT.lock() {
        *slot = Some(path);
    }
    std::thread::sleep(std::time::Duration::from_millis(400));
    Ok(())
}
