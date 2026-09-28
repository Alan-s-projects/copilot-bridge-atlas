#[tauri::command]
pub async fn get_usage_date_range(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::settings::UsageDateRange, String> {
    state
        .db
        .get_usage_date_range()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_usage_date_range(
    state: tauri::State<'_, crate::AppState>,
    range: crate::settings::UsageDateRange,
) -> Result<crate::settings::UsageDateRange, String> {
    state
        .db
        .set_usage_date_range(range.clone())
        .map_err(|error| error.to_string())?;
    Ok(range)
}

#[tauri::command]
pub async fn get_usage_trend_grouping(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::services::usage_stats::TrendGrouping, String> {
    state
        .db
        .get_usage_trend_grouping()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_usage_trend_grouping(
    state: tauri::State<'_, crate::AppState>,
    grouping: crate::services::usage_stats::TrendGrouping,
) -> Result<crate::services::usage_stats::TrendGrouping, String> {
    state
        .db
        .set_usage_trend_grouping(grouping)
        .map_err(|error| error.to_string())?;
    Ok(grouping)
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
    Ok(crate::settings::get_settings())
}

#[tauri::command]
pub async fn save_settings(settings: crate::settings::AppSettings) -> Result<bool, String> {
    crate::settings::update_settings(settings).map_err(|error| error.to_string())?;
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
