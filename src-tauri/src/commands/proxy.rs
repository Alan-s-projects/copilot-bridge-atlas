//! Tauri commands related to proxy services
//!
//! Provides API interface for front-end calls

use crate::proxy::types::*;
use crate::store::AppState;

/// Start the proxy server (only starts the service, does not take over the Live configuration)
#[tauri::command]
pub async fn start_proxy_server(
    state: tauri::State<'_, AppState>,
) -> Result<ProxyServerInfo, String> {
    state.proxy_service.start().await
}

/// Stop the proxy server (only stops the service, does not restore/clean up the Live takeover state)
#[tauri::command]
pub async fn stop_proxy_server(state: tauri::State<'_, AppState>) -> Result<(), String> {
    if state.proxy_service.is_running().await {
        state.proxy_service.stop().await?;
    } else {
        let mut config = state
            .db
            .get_global_proxy_config()
            .await
            .map_err(|e| e.to_string())?;
        config.proxy_enabled = false;
        state
            .db
            .update_global_proxy_config(config)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Get proxy server status
#[tauri::command]
pub async fn get_proxy_status(state: tauri::State<'_, AppState>) -> Result<ProxyStatus, String> {
    state.proxy_service.get_status().await
}

// ==================== Global & Per-App Config ====================

/// Get global proxy configuration
///
/// Returns the proxy switch, listener address, and port. Usage recording is always on.
#[tauri::command]
pub async fn get_global_proxy_config(
    state: tauri::State<'_, AppState>,
) -> Result<GlobalProxyConfig, String> {
    let db = &state.db;
    db.get_global_proxy_config()
        .await
        .map_err(|e| e.to_string())
}

/// Update global proxy configuration
///
/// Update the one local listener's configuration.
#[tauri::command]
pub async fn update_global_proxy_config(
    state: tauri::State<'_, AppState>,
    config: GlobalProxyConfig,
) -> Result<(), String> {
    let db = &state.db;
    db.update_global_proxy_config(config)
        .await
        .map_err(|e| e.to_string())?;
    let current = state.proxy_service.get_config().await?;
    state.proxy_service.update_config(&current).await
}
