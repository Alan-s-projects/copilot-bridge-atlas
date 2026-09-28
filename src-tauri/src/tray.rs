use crate::AppError;
use tauri::menu::{Menu, MenuBuilder};
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

pub const TRAY_ID: &str = "copilot-bridge-atlas";

pub fn create_tray_menu(app: &tauri::AppHandle) -> Result<Menu<tauri::Wry>, AppError> {
    MenuBuilder::new(app)
        .text("show_main", "Open Copilot Bridge Atlas")
        .text("open_website", "GitHub repository")
        .separator()
        .text("quit", "Quit")
        .build()
        .map_err(|error| AppError::Message(error.to_string()))
}

pub fn handle_tray_menu_event(app: &tauri::AppHandle, id: &str) {
    match id {
        "show_main" => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_skip_taskbar(false);
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        "open_website" => {
            let _ = app.opener().open_url(
                "https://github.com/Alan-s-projects/copilot-bridge-atlas",
                None::<String>,
            );
        }
        "quit" => app.exit(0),
        _ => {}
    }
}
