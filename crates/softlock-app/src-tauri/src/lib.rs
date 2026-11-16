use tauri::Manager;

#[derive(Clone, serde::Serialize)]
struct PlatformStatus {
    supported: bool,
    message: Option<String>,
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

#[tauri::command]
fn platform_status() -> PlatformStatus {
    current_platform_status()
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![platform_status])
        .run(tauri::generate_context!())
        .expect("error while running the application");
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
