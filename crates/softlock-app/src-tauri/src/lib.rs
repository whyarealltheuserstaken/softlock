use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let window = app
                .get_webview_window("main")
                .expect("the config declares the main window");
            window.set_title(softlock_core::PRODUCT_DISPLAY_NAME)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the application");
}
