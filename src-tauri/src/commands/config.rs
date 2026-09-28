use crate::config;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub async fn open_generated_model_catalog(handle: AppHandle) -> Result<(), String> {
    let path = crate::copilot_bridge::catalog_path();
    if !path.is_file() {
        return Err("No generated model catalog is available. Refresh models and enable at least one model first.".into());
    }
    handle
        .opener()
        .open_path(path.to_string_lossy().to_string(), None::<String>)
        .map_err(|error| format!("Could not open the generated model catalog: {error}"))
}

#[tauri::command]
pub async fn open_app_config_folder(handle: AppHandle) -> Result<bool, String> {
    let config_dir = config::get_app_config_dir();

    if !config_dir.exists() {
        std::fs::create_dir_all(&config_dir)
            .map_err(|e| format!("Failed to create directory: {e}"))?;
    }

    handle
        .opener()
        .open_path(config_dir.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| format!("Failed to open folder: {e}"))?;

    Ok(true)
}
