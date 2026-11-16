//! Windows Hello consent verifier, called on a worker thread in the
//! signed-in user's context.

use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
use windows::core::HSTRING;
use windows_future::IAsyncOperation;

pub fn request(hwnd: isize) -> String {
    match request_inner(hwnd) {
        Ok(text) => text,
        Err(error) => format!("consent call failed: {error}"),
    }
}

fn request_inner(hwnd: isize) -> windows::core::Result<String> {
    let hwnd = HWND(hwnd as *mut core::ffi::c_void);
    // SAFETY: this worker thread initializes COM for itself.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    }
    let availability = UserConsentVerifier::CheckAvailabilityAsync()?.join()?;
    if availability != UserConsentVerifierAvailability::Available {
        return Ok(format!(
            "hello availability: {}",
            availability_name(availability)
        ));
    }
    let message = HSTRING::from("Confirm it is you");
    // SAFETY: RequestVerificationForWindowAsync shows the system consent dialog
    // owned by the overlay window. The operation is joined on this worker.
    let operation: IAsyncOperation<UserConsentVerificationResult> = unsafe {
        let interop = windows::core::factory::<UserConsentVerifier, IUserConsentVerifierInterop>()?;
        interop.RequestVerificationForWindowAsync(hwnd, &message)?
    };
    let result = operation.join()?;
    Ok(format!("hello result: {}", result_name(result)))
}

fn availability_name(value: UserConsentVerifierAvailability) -> &'static str {
    if value == UserConsentVerifierAvailability::Available {
        "Available"
    } else if value == UserConsentVerifierAvailability::DeviceNotPresent {
        "DeviceNotPresent"
    } else if value == UserConsentVerifierAvailability::NotConfiguredForUser {
        "NotConfiguredForUser"
    } else if value == UserConsentVerifierAvailability::DisabledByPolicy {
        "DisabledByPolicy"
    } else if value == UserConsentVerifierAvailability::DeviceBusy {
        "DeviceBusy"
    } else {
        "Unknown"
    }
}

fn result_name(value: UserConsentVerificationResult) -> &'static str {
    if value == UserConsentVerificationResult::Verified {
        "Verified"
    } else if value == UserConsentVerificationResult::DeviceNotPresent {
        "DeviceNotPresent"
    } else if value == UserConsentVerificationResult::NotConfiguredForUser {
        "NotConfiguredForUser"
    } else if value == UserConsentVerificationResult::DisabledByPolicy {
        "DisabledByPolicy"
    } else if value == UserConsentVerificationResult::DeviceBusy {
        "DeviceBusy"
    } else if value == UserConsentVerificationResult::RetriesExhausted {
        "RetriesExhausted"
    } else if value == UserConsentVerificationResult::Canceled {
        "Canceled"
    } else {
        "Unknown"
    }
}
