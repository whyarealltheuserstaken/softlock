mod actions;
mod client;
#[cfg(windows)]
mod owner;

use tauri::Manager;
use zeroize::Zeroize;

use actions::{pin_field_error, service_line, status_line};
use client::OwnerView;
use softlock_core::Secret;

#[derive(Clone, serde::Serialize)]
struct PlatformStatus {
    supported: bool,
    message: Option<String>,
}

#[derive(Clone, serde::Serialize)]
struct OwnerViewDto {
    service_running: bool,
    locked: bool,
    state: String,
    lock_source: Option<String>,
    status_line: String,
    service_line: String,
    notice: String,
    config_errors: String,
    config_offer: bool,
    config_hotkey: String,
}

#[derive(Clone, serde::Serialize)]
struct PinOutcome {
    accepted: bool,
    field_error: Option<String>,
}

fn current_platform_status() -> PlatformStatus {
    #[cfg(target_os = "linux")]
    {
        let report = softlock_linux::capability_report();
        PlatformStatus {
            supported: report.is_supported(),
            message: Some(report.message.to_string()),
        }
    }
    #[cfg(target_os = "macos")]
    {
        let report = softlock_macos::capability_report();
        PlatformStatus {
            supported: report.is_supported(),
            message: Some(report.message.to_string()),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        PlatformStatus {
            supported: true,
            message: None,
        }
    }
}

fn view_dto(view: &OwnerView) -> OwnerViewDto {
    OwnerViewDto {
        service_running: view.service_running,
        locked: view.locked(),
        state: view
            .state
            .map(|state| format!("{state:?}"))
            .unwrap_or_else(|| "Locked".to_string()),
        lock_source: view
            .lock_source
            .map(client::lock_source_name)
            .map(str::to_string),
        status_line: status_line(view.service_running, view.state),
        service_line: service_line(view.service_running).to_string(),
        notice: shown_notice(),
        config_errors: shown_config_errors(),
        config_offer: shown_config_offer(),
        config_hotkey: shown_config_hotkey(),
    }
}

fn shown_config_errors() -> String {
    #[cfg(windows)]
    {
        owner::config_errors()
    }
    #[cfg(not(windows))]
    {
        String::new()
    }
}

fn shown_config_offer() -> bool {
    #[cfg(windows)]
    {
        owner::config_offer()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn shown_config_hotkey() -> String {
    #[cfg(windows)]
    {
        owner::config_hotkey()
    }
    #[cfg(not(windows))]
    {
        String::new()
    }
}

fn shown_notice() -> String {
    #[cfg(windows)]
    {
        owner::current_notice()
    }
    #[cfg(not(windows))]
    {
        String::new()
    }
}

fn shown_view() -> OwnerView {
    #[cfg(windows)]
    {
        owner::current_view()
    }
    #[cfg(not(windows))]
    {
        OwnerView::unknown()
    }
}

#[tauri::command]
fn platform_status() -> PlatformStatus {
    current_platform_status()
}

#[tauri::command]
fn owner_view() -> OwnerViewDto {
    view_dto(&shown_view())
}

#[tauri::command]
fn lock_from_app() -> OwnerViewDto {
    #[cfg(windows)]
    owner::lock_from_app();
    view_dto(&shown_view())
}

#[tauri::command]
fn lock_for_real() -> OwnerViewDto {
    #[cfg(windows)]
    owner::lock_for_real();
    view_dto(&shown_view())
}

#[tauri::command]
fn set_pin(mut pin: String) -> PinOutcome {
    if let Some(code) = pin_field_error(&pin) {
        pin.zeroize();
        return PinOutcome {
            accepted: false,
            field_error: Some(code.to_string()),
        };
    }
    let secret = Secret::new(pin.as_bytes());
    pin.zeroize();
    #[cfg(windows)]
    {
        pin_outcome(owner::send_pin(secret))
    }
    #[cfg(not(windows))]
    {
        drop(secret);
        PinOutcome {
            accepted: false,
            field_error: None,
        }
    }
}

#[cfg(windows)]
fn pin_outcome(outcome: owner::PinSend) -> PinOutcome {
    PinOutcome {
        accepted: matches!(outcome, owner::PinSend::Stored),
        field_error: if matches!(outcome, owner::PinSend::NeedsConfirmation) {
            Some("confirmation-needed".to_string())
        } else {
            None
        },
    }
}

#[tauri::command]
fn accept_config() -> OwnerViewDto {
    #[cfg(windows)]
    {
        let _ = owner::accept_config();
    }
    view_dto(&shown_view())
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    #[cfg(windows)]
    if owner::request_quit() {
        return;
    }
    app.exit(0);
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let status = current_platform_status();
            if status.supported && status.message.is_some() {
                eprintln!("error: platform_status reported support together with a message");
            }
            if !status.supported
                && status.message.as_deref() != Some(softlock_core::UNSUPPORTED_PLATFORM_MESSAGE)
            {
                eprintln!("error: platform_status is unsupported without the documented message");
            }
            app.manage(status);
            let window = app
                .get_webview_window("main")
                .expect("the config declares the main window");
            window.set_title(softlock_core::PRODUCT_DISPLAY_NAME)?;
            #[cfg(windows)]
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    owner::run(Some(handle));
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            platform_status,
            owner_view,
            lock_from_app,
            lock_for_real,
            set_pin,
            accept_config,
            quit_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}

pub fn run_owner_session() -> i32 {
    #[cfg(windows)]
    {
        owner::run(None)
    }
    #[cfg(not(windows))]
    {
        eprintln!("the owner session runs on Windows");
        1
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stub_backend_is_linked_for_platform_status() {
        let status = super::current_platform_status();
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            assert!(!status.supported);
            assert_eq!(
                status.message.as_deref(),
                Some(softlock_core::UNSUPPORTED_PLATFORM_MESSAGE)
            );
        } else {
            assert!(status.supported);
            assert!(status.message.is_none());
        }
    }
}
