fn merge_settings_for_save(
    mut incoming: crate::settings::AppSettings,
    existing: &crate::settings::AppSettings,
) -> crate::settings::AppSettings {
    incoming.legacy_options = existing.legacy_options.clone();
    incoming.codex_config_dir = existing.codex_config_dir.clone();
    incoming
}

#[tauri::command]
pub async fn get_usage_table_columns(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::settings::UsageTableColumns, String> {
    state
        .db
        .get_usage_table_columns()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_usage_table_columns(
    state: tauri::State<'_, crate::AppState>,
    table: String,
    columns: Vec<String>,
) -> Result<crate::settings::UsageTableColumns, String> {
    state
        .db
        .set_usage_table_columns(&table, columns)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_settings() -> Result<crate::settings::AppSettings, String> {
    Ok(crate::settings::get_settings_for_frontend())
}

#[tauri::command]
pub async fn save_settings(settings: crate::settings::AppSettings) -> Result<bool, String> {
    let existing = crate::settings::get_settings();
    crate::settings::update_settings(merge_settings_for_save(settings, &existing))
        .map_err(|error| error.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn set_auto_launch(enabled: bool) -> Result<bool, String> {
    if enabled {
        crate::auto_launch::enable_auto_launch()
    } else {
        crate::auto_launch::disable_auto_launch()
    }
    .map_err(|error| error.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn get_log_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::LogConfig, String> {
    state.db.get_log_config().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_log_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::LogConfig,
) -> Result<bool, String> {
    state
        .db
        .set_log_config(&config)
        .map_err(|error| error.to_string())?;
    log::set_max_level(config.to_level_filter());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;

    #[test]
    fn saving_visible_preferences_preserves_opaque_data_and_discovery_hint() {
        let existing = AppSettings {
            codex_config_dir: Some("D:/Codex".into()),
            legacy_options: [(
                "retiredPreference".into(),
                serde_json::json!({"keep": true}),
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let incoming = AppSettings {
            launch_on_startup: true,
            ..Default::default()
        };
        let saved = merge_settings_for_save(incoming, &existing);
        assert!(saved.launch_on_startup);
        assert_eq!(saved.codex_config_dir, existing.codex_config_dir);
        assert_eq!(saved.legacy_options, existing.legacy_options);
    }
}
