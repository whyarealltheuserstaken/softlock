use tauri::Manager;

struct PlatformStatus {
    supported: bool,
    message: Option<String>,
}

fn platform_status() -> PlatformStatus {
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

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let status = platform_status();
            if status.supported {
                assert!(status.message.is_none());
            } else {
                assert_eq!(
                    status.message.as_deref(),
                    Some(softlock_core::UNSUPPORTED_PLATFORM_MESSAGE)
                );
            }
            app.manage(status);
            let window = app
                .get_webview_window("main")
                .expect("the config declares the main window");
            window.set_title(softlock_core::PRODUCT_DISPLAY_NAME)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the application");
}

#[cfg(test)]
mod tests {
    #[test]
    fn stub_backend_is_linked_for_platform_status() {
        let status = super::platform_status();
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
