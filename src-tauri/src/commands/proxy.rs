//! 代理服务相关的 Tauri 命令
//!
//! 提供前端调用的 API 接口

use crate::proxy::types::*;
use crate::store::AppState;

/// 启动代理服务器（仅启动服务，不接管 Live 配置）
#[tauri::command]
pub async fn start_proxy_server(
    state: tauri::State<'_, AppState>,
) -> Result<ProxyServerInfo, String> {
    state.proxy_service.start().await
}

/// 停止代理服务器（仅停止服务，不恢复/清理 Live 接管状态）
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

/// 获取代理服务器状态
#[tauri::command]
pub async fn get_proxy_status(state: tauri::State<'_, AppState>) -> Result<ProxyStatus, String> {
    state.proxy_service.get_status().await
}

// ==================== Global & Per-App Config ====================

/// 获取全局代理配置
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

/// 更新全局代理配置
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
