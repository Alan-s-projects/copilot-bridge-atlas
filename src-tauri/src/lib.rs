#![warn(unused_crate_dependencies)]

mod auto_launch;
mod codex_config;
mod commands;
mod config;
mod copilot_bridge;
mod database;
mod error;
mod init_status;
mod model_capabilities;
mod panic_hook;
mod provider;
mod proxy;
mod services;
mod settings;
mod store;

mod tray;
mod usage_events;

pub(crate) use database::Database;
pub(crate) use error::AppError;
pub(crate) use provider::{Provider, ProviderMeta};
pub(crate) use store::AppState;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::tray::TrayIconBuilder;
use tauri::RunEvent;
use tauri::{Emitter, Manager};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

fn set_windows_app_user_model_id(app: &tauri::AppHandle) {
    let app_id = app.config().identifier.clone();
    let wide_app_id: Vec<u16> = app_id.encode_utf16().chain(std::iter::once(0)).collect();

    let result = unsafe {
        windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(wide_app_id.as_ptr())
    };

    if result < 0 {
        log::warn!("Failed to set Windows AppUserModelID: 0x{result:08X}");
    } else {
        log::debug!("Windows AppUserModelID set to {app_id}");
    }
}

/// 无 scheme 的裸 authority 形态(如 `user:pass@host/path`)剥掉 userinfo：
/// 仅当 `@` 出现在第一个 `/` 之前时才视为凭据。
fn strip_bare_userinfo(input: &str) -> &str {
    let authority_end = input.find('/').unwrap_or(input.len());
    match input[..authority_end].rfind('@') {
        Some(at) => &input[at + 1..],
        None => input,
    }
}

/// Remove URL user information, query parameters, and fragments before logging.
/// Preserve the endpoint path so routing errors remain diagnosable.
pub(crate) fn redact_url_for_log(url_str: &str) -> String {
    let scheme_relative = url_str.starts_with("//");
    let parsed = if scheme_relative {
        url::Url::parse(&format!("https:{url_str}"))
    } else {
        url::Url::parse(url_str)
    };

    match parsed {
        Ok(mut url) if url.has_host() => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            let rendered = url.as_str();
            if scheme_relative {
                rendered
                    .strip_prefix("https:")
                    .unwrap_or(rendered)
                    .to_string()
            } else {
                rendered.to_string()
            }
        }
        _ => {
            // 解析失败(相对路径、含裸 userinfo 的非法 URL 等)：丢掉 query/fragment，
            // 尽力剥掉 userinfo，其余原样保留。
            let without_tail = url_str.split(['?', '#']).next().unwrap_or(url_str);
            strip_bare_userinfo(without_tail).to_string()
        }
    }
}

fn runtime_log_level_allows(level: log::Level) -> bool {
    level <= log::Level::Info
}

/// 更新托盘菜单的Tauri命令
#[tauri::command]
async fn update_tray_menu(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    match tray::create_tray_menu(&app, state.inner()) {
        Ok(new_menu) => {
            if let Some(tray) = app.tray_by_id(tray::TRAY_ID) {
                tray.set_menu(Some(new_menu))
                    .map_err(|e| format!("Failed to update tray menu: {e}"))?;
                return Ok(true);
            }
            Ok(false)
        }
        Err(err) => {
            log::error!("Failed to create tray menu: {err}");
            Ok(false)
        }
    }
}

pub fn run() {
    panic_hook::setup_panic_hook();

    let startup_page_handled = AtomicBool::new(false);
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_skip_taskbar(false);
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .on_page_load(move |webview, payload| {
            if webview.label() == "main"
                && payload.event() == tauri::webview::PageLoadEvent::Finished
                && payload.url().scheme() != "about"
                && !startup_page_handled.swap(true, Ordering::Relaxed)
            {
                let _ = webview.window().show();
                log::info!("Main page loaded; the application window is visible");
            }
        })
        // Keep the proxy alive when the user closes its window.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                // 数据库版本过新的恢复模式下没有托盘可唤回，关闭即退出，避免应用隐身后台
                let in_db_recovery = crate::init_status::get_init_error()
                    .map(|p| p.kind.as_deref() == Some("db_version_too_new"))
                    .unwrap_or(false);
                if in_db_recovery {
                    window.app_handle().exit(0);
                    return;
                }

                let _ = window.hide();
                let _ = window.set_skip_taskbar(true);
            }
        })
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(window_state_flags())
                .build(),
        )
        .setup(|app| {
            let _ = rustls::crypto::ring::default_provider().install_default();

            crate::config::initialize_legacy_app_config_dir(&app.path().app_data_dir()?);
            panic_hook::init_app_config_dir(crate::config::get_app_config_dir());

            // 初始化日志（输出到 <app_config_dir>/logs/copilot-bridge-atlas.log）
            {
                use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

                let log_dir = panic_hook::get_log_dir();

                // 确保日志目录存在
                if let Err(e) = std::fs::create_dir_all(&log_dir) {
                    eprintln!("Failed to create log directory: {e}");
                }

                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        // The frontend logging command reaches the plugin directly.
                        // Keep the same fixed Info ceiling there as in Rust.
                        .filter(|metadata| runtime_log_level_allows(metadata.level()))
                        .targets([
                            Target::new(TargetKind::Stdout),
                            Target::new(TargetKind::Folder {
                                path: log_dir,
                                file_name: Some("copilot-bridge-atlas".into()),
                            }),
                        ])
                        // KeepSome(4) 保留 4 个轮转归档，加上当前文件最多约 100 MiB。
                        // 轮转仅按大小触发；跨重启继续追加，不再丢失上一次运行的日志。
                        .rotation_strategy(RotationStrategy::KeepSome(4))
                        .max_file_size(20 * 1024 * 1024)
                        .timezone_strategy(TimezoneStrategy::UseLocal)
                        .build(),
                )?;

                log::set_max_level(log::LevelFilter::Info);
                log::info!("=== Copilot Bridge Atlas v{} started ===", env!("CARGO_PKG_VERSION"));
            }

            set_windows_app_user_model_id(app.handle());

            // 注入 AppHandle 给 usage_events，让无 AppHandle 持有的写日志路径
            // 也能向前端推送 `usage-log-recorded`。
            // 放在日志系统初始化之后，确保 init 的日志能正常输出。
            usage_events::init(app.handle().clone());

            // 初始化数据库
            let app_config_dir = crate::config::get_app_config_dir();
            let db_path = app_config_dir.join("copilot-bridge-atlas.db");

            // Reject databases from a newer app before attempting schema writes.
            match crate::database::Database::stored_user_version_exceeds_supported(&db_path) {
                Ok(Some(version)) => {
                    log::warn!("Database schema v{version} is newer than supported; opening the recovery view");
                    crate::init_status::set_init_error(crate::init_status::InitErrorPayload {
                        path: db_path.display().to_string(),
                        error: format!(
                            "Database schema {version} is newer than this app supports ({}). Install a newer version of Copilot Bridge Atlas.",
                            crate::database::SCHEMA_VERSION
                        ),
                        kind: Some("db_version_too_new".to_string()),
                        db_version: Some(version),
                        supported_version: Some(crate::database::SCHEMA_VERSION),
                    });
                    // 主窗口默认 visible:false，恢复界面必须强制显示
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.set_skip_taskbar(false);
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                    return Ok(());
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!("Database version preflight failed; proceeding to initialization: {e}");
                }
            }

            let db = loop {
                match crate::database::Database::init() {
                    Ok(db) => break Arc::new(db),
                    Err(e) => {
                        log::error!("Failed to init database: {e}");

                        if !show_database_init_error_dialog(app.handle(), &db_path, &e.to_string())
                        {
                            log::info!("The user chose to exit");
                            std::process::exit(1);
                        }

                        log::info!("The user chose to retry database initialization");
                    }
                }
            };

            let app_state = AppState::new(db);

            // Share the app handle for proxy status events.
            app_state.proxy_service.set_app_handle(app.handle().clone());

            if let Err(error) = copilot_bridge::initialize(&app_state) {
                log::warn!("Copilot model catalog initialization failed: {error}");
            }

            // 创建动态托盘菜单
            let menu = tray::create_tray_menu(app.handle(), &app_state)?;

            // 构建托盘
            let mut tray_builder = TrayIconBuilder::with_id(tray::TRAY_ID)
                .menu(&menu)
                .tooltip("Copilot Bridge Atlas") // 鼠标悬停提示
                .on_menu_event(|app, event| {
                    tray::handle_tray_menu_event(app, &event.id.0);
                })
                .show_menu_on_left_click(true);

            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            } else {
                log::warn!("The default window icon is unavailable for the tray");
            }

            let _tray = tray_builder.build(app)?;
            // 将同一个实例注入到全局状态，避免重复创建导致的不一致
            app.manage(app_state);

            // 初始化 CopilotAuthManager
            {
                use crate::proxy::providers::copilot_auth::CopilotAuthManager;
                use commands::CopilotAuthState;
                use tokio::sync::RwLock;

                let app_config_dir = crate::config::get_app_config_dir();
                let copilot_auth_manager = CopilotAuthManager::new(app_config_dir);
                app.manage(CopilotAuthState(Arc::new(RwLock::new(copilot_auth_manager))));
                log::info!("✓ CopilotAuthManager initialized");
            }

            // 初始化全局出站代理 HTTP 客户端
            {
                let db = &app.state::<AppState>().db;
                let proxy_url = db.get_global_proxy_url().ok().flatten();

                if let Err(e) = crate::proxy::http_client::init(proxy_url.as_deref()) {
                    log::error!(
                        "[GlobalProxy] [GP-005] Failed to initialize with saved config: {e}"
                    );

                    // 清除无效的代理配置
                    if proxy_url.is_some() {
                        log::warn!(
                            "[GlobalProxy] [GP-006] Clearing invalid proxy config from database"
                        );
                        if let Err(clear_err) = db.set_global_proxy_url(None) {
                            log::error!(
                                "[GlobalProxy] [GP-007] Failed to clear invalid config: {clear_err}"
                            );
                        }
                    }

                    // 使用直连模式重新初始化
                    if let Err(fallback_err) = crate::proxy::http_client::init(None) {
                        log::error!(
                            "[GlobalProxy] [GP-008] Failed to initialize direct connection: {fallback_err}"
                        );
                    }
                }
            }

            // 异常退出恢复 + 代理状态自动恢复
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = app_handle.state::<AppState>();

                // 检查 settings 表中的代理状态，自动恢复代理服务
                restore_proxy_state_on_startup(&state).await;

                let auth_state = app_handle.state::<commands::CopilotAuthState>();
                let auth = auth_state.0.read().await;
                if let Err(error) = copilot_bridge::refresh_capabilities(&state, &auth).await {
                    log::warn!("Copilot capability refresh failed; keeping saved catalog: {error}");
                }
                drop(auth);
                let _ = app_handle.emit("provider-switched", serde_json::json!({
                    "appType": "codex",
                    "providerId": copilot_bridge::current(&state.db).unwrap_or_default()
                }));

                // Periodic backup check (on startup)
                if let Err(e) = state.db.periodic_backup_if_needed() {
                    log::warn!("Periodic backup failed on startup: {e}");
                }

                // Periodic maintenance timer: run once per day while the app is running
                let db_for_timer = state.db.clone();
                tauri::async_runtime::spawn(async move {
                    const PERIODIC_MAINTENANCE_INTERVAL_SECS: u64 = 24 * 60 * 60;
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                        PERIODIC_MAINTENANCE_INTERVAL_SECS,
                    ));
                    interval.tick().await; // skip immediate first tick (already checked above)
                    loop {
                        interval.tick().await;
                        if let Err(e) = db_for_timer.periodic_backup_if_needed() {
                            log::warn!("Periodic maintenance timer failed: {e}");
                        }
                    }
                });

            });

            // The Windows window is shown after its first page finishes loading.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_decorations(true);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_providers,
            commands::get_current_provider,
            commands::update_provider,
            commands::open_external,
            commands::get_init_error,
            commands::open_app_config_folder,
            commands::open_generated_model_catalog,
            commands::get_settings,
            commands::get_usage_table_columns,
            commands::get_usage_trend_grouping,
            commands::get_usage_date_range,
            commands::set_usage_date_range,
            commands::set_usage_trend_grouping,
            commands::set_usage_table_columns,
            commands::save_settings,
            commands::check_for_updates,
            commands::get_available_release_version,
            commands::copy_text_to_clipboard,
            commands::create_db_backup,
            commands::list_db_backups,
            commands::restore_db_backup,
            commands::rename_db_backup,
            commands::delete_db_backup,
            commands::import_config_from_file,
            update_tray_menu,
            commands::set_auto_launch,
            commands::start_proxy_server,
            commands::stop_proxy_server,
            commands::get_proxy_status,
            commands::get_global_proxy_config,
            commands::update_global_proxy_config,
            commands::get_usage_summary,
            commands::get_usage_trends,
            commands::get_model_stats,
            commands::get_unpriced_model_usage,
            commands::get_request_logs,
            commands::get_model_pricing,
            commands::update_model_pricing,
            commands::delete_model_pricing,
            commands::reset_model_pricing_to_defaults,
            commands::get_global_proxy_url,
            commands::set_global_proxy_url,
            commands::test_proxy_url,
            commands::scan_local_proxies,
            commands::set_window_theme,
            commands::auth_start_login,
            commands::auth_poll_for_account,
            commands::auth_get_status,
            commands::auth_remove_account,
            commands::auth_set_default_account,
            commands::auth_logout,
            commands::copilot_get_models,
            commands::copilot_get_models_for_account,
            commands::copilot_get_usage,
            commands::copilot_get_usage_for_account,
            copilot_bridge::get_codex_setup_suggestion,
        ]);

    let app = builder
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { api, code, .. } = &event {
            match classify_exit_request(*code) {
                ExitRequestAction::StayInTray => {
                    api.prevent_exit();
                    return;
                }
                // Tauri's restart code ignores prevent_exit. Its normal Exit
                // hook saves window state on the main thread; competing async
                // cleanup could deadlock the window-state plugin.
                ExitRequestAction::DeferToTauriRestart => return,
                ExitRequestAction::CleanupAndExit => {}
            }

            log::info!("Requested application exit (code={code:?}); cleaning up");
            api.prevent_exit();
            let app_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                save_window_state_before_exit(&app_handle);
                cleanup_before_exit(&app_handle).await;
                // process::exit bypasses Drop, so explicitly remove the
                // Windows tray icon before terminating the process.
                remove_tray_icon_before_exit(&app_handle);
                log::info!("Cleanup complete; exiting Copilot Bridge Atlas");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                std::process::exit(0);
            });
        }
    });
}

// ============================================================
// 应用退出清理
// ============================================================

/// Stop the listener without changing its saved switch or any client files.
pub(crate) async fn cleanup_before_exit(app_handle: &tauri::AppHandle) {
    if let Some(state) = app_handle.try_state::<store::AppState>() {
        if let Err(error) = state.proxy_service.shutdown().await {
            log::warn!("Stopping Copilot bridge failed: {error}");
        }
    }
}

/// 主动从系统托盘移除托盘图标。
///
/// `std::process::exit` 会绕过 Tauri 运行时，触发不了 `TrayIcon::drop()`，
/// 也就不会向 Windows Shell 发 `NIM_DELETE`。结果是进程退出后托盘里
/// 仍保留一个死图标的缓存占位（Shell 不会主动重绘，需要鼠标悬停才刷新）。
///
/// 通过 `set_visible(false)` 走 `WM_USER_HIDE_TRAYICON` 消息路径，
/// 触发 tray-icon 内部的 `remove_tray_icon` → `Shell_NotifyIconW(NIM_DELETE)`，
/// 在进程结束前干净地把图标摘掉。
pub(crate) fn remove_tray_icon_before_exit(app_handle: &tauri::AppHandle) {
    if let Some(tray) = app_handle.tray_by_id(tray::TRAY_ID) {
        if let Err(e) = tray.set_visible(false) {
            log::warn!("Failed to remove the tray icon during exit: {e}");
        } else {
            log::info!("Removed the tray icon");
        }
    }
}

// ============================================================
// 启动时恢复代理状态
// ============================================================

/// Read the one saved listener switch. No client files are written.
async fn proxy_enabled_on_startup(db: &database::Database) -> bool {
    match db.get_global_proxy_config().await {
        Ok(config) => config.proxy_enabled,
        Err(error) => {
            log::warn!("Could not read the saved proxy switch: {error}");
            false
        }
    }
}

async fn restore_proxy_state_on_startup(state: &store::AppState) {
    if proxy_enabled_on_startup(&state.db).await {
        if let Err(error) = state.proxy_service.start().await {
            log::warn!("Starting Copilot bridge failed: {error}");
        }
    }
}

fn show_database_init_error_dialog(
    app: &tauri::AppHandle,
    db_path: &std::path::Path,
    error: &str,
) -> bool {
    app.dialog()
        .message(format!("Database initialization failed: {error}\n\nDatabase: {}\n\nYour database has been preserved. Back up the data folder before trying a compatible app version.", db_path.display()))
        .title("Database Initialization Failed")
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::OkCancelCustom("Retry".into(), "Exit".into()))
        .blocking_show()
}

// ============================================================
// 退出请求分类
// ============================================================

/// `RunEvent::ExitRequested` 的三类来源，处理方式必须区分。
///
/// 关键约束：重启请求（`code == RESTART_EXIT_CODE`）上 `prevent_exit()` 会被
/// Tauri 静默忽略（见 `ExitRequestApi::prevent_exit` 文档），事件循环必定继续
/// 退出并触发各插件的 `RunEvent::Exit` 钩子；任何与之并发的自定义清理任务都
/// 可能与插件退出钩子争用同一状态而死锁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitRequestAction {
    /// `code` 为 `None`：运行时自动触发（如隐藏窗口的 WebView 被回收导致无存活
    /// 窗口），阻止退出、保持托盘后台运行。
    StayInTray,
    /// `code` 为 `RESTART_EXIT_CODE`：`app.restart()` / 自更新 relaunch 发起的
    /// 重启，不拦截、不做自定义清理，交还 Tauri 默认 re-exec 流程。
    DeferToTauriRestart,
    /// 其它 `Some(_)`：用户主动退出（托盘「退出」等），执行完整异步清理后结束进程。
    CleanupAndExit,
}

fn classify_exit_request(code: Option<i32>) -> ExitRequestAction {
    match code {
        None => ExitRequestAction::StayInTray,
        Some(tauri::RESTART_EXIT_CODE) => ExitRequestAction::DeferToTauriRestart,
        Some(_) => ExitRequestAction::CleanupAndExit,
    }
}

// ============================================================
// 在应用主动退出前显式持久化窗口状态
// ============================================================

fn window_state_flags() -> StateFlags {
    StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED
}

/// 当前应用的退出路径会拦截 `ExitRequested` 并最终直接 `std::process::exit(0)`，
/// 这里需要在真正结束进程前手动落盘，避免 window-state 插件的默认退出钩子被绕过。
pub(crate) fn save_window_state_before_exit(app_handle: &tauri::AppHandle) {
    if let Err(err) = app_handle.save_window_state(window_state_flags()) {
        log::error!("Failed to save window state before exit: {err}");
    } else {
        log::info!("Saved window state before exit");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_exit_request, proxy_enabled_on_startup, redact_url_for_log,
        runtime_log_level_allows, ExitRequestAction,
    };
    use crate::database::Database;

    #[test]
    fn log_url_redaction_strips_credentials_and_query_keeps_path() {
        // userinfo 与整个 query 剥离，path 保留用于诊断 base_url 配错。
        assert_eq!(
            redact_url_for_log(
                "https://user:secret@example.com:8443/v1/models?key=top-secret&alt=sse#private"
            ),
            "https://example.com:8443/v1/models"
        );
        // scheme-relative 保持形态，userinfo 去掉。
        assert_eq!(
            redact_url_for_log("//user:sk-secret@gw.example.com/v1"),
            "//gw.example.com/v1"
        );
        // 无 scheme 的裸 userinfo。
        assert_eq!(
            redact_url_for_log("user:sk-secret@gw.example.com/v1?token=hidden#private"),
            "gw.example.com/v1"
        );
        // 无法解析为绝对 URL 时：丢 query，其余原样保留。
        assert_eq!(
            redact_url_for_log("not-a-url?token=secret#private"),
            "not-a-url"
        );
        // 不再对 path 段做“看起来像密钥”的形状猜测，正常路径完整保留。
        assert_eq!(
            redact_url_for_log("https://host.example/v1/models/gpt-6-astra"),
            "https://host.example/v1/models/gpt-6-astra"
        );
    }

    #[test]
    fn runtime_log_filter_honors_dynamic_max_level() {
        assert!(runtime_log_level_allows(log::Level::Error));
        assert!(runtime_log_level_allows(log::Level::Warn));
        assert!(runtime_log_level_allows(log::Level::Info));
        assert!(!runtime_log_level_allows(log::Level::Debug));
    }

    #[test]
    fn no_code_keeps_app_alive_in_tray() {
        assert_eq!(classify_exit_request(None), ExitRequestAction::StayInTray);
    }

    #[test]
    fn restart_exit_code_defers_to_tauri_default_restart() {
        assert_eq!(
            classify_exit_request(Some(tauri::RESTART_EXIT_CODE)),
            ExitRequestAction::DeferToTauriRestart
        );
    }

    #[test]
    fn user_exit_codes_run_cleanup_then_exit() {
        assert_eq!(
            classify_exit_request(Some(0)),
            ExitRequestAction::CleanupAndExit
        );
        assert_eq!(
            classify_exit_request(Some(1)),
            ExitRequestAction::CleanupAndExit
        );
    }

    #[tokio::test]
    async fn startup_respects_the_saved_global_proxy_switch() {
        let db = Database::memory().expect("initialize database");
        assert!(!proxy_enabled_on_startup(&db).await);

        let mut config = db.get_global_proxy_config().await.unwrap();
        config.proxy_enabled = true;
        db.update_global_proxy_config(config.clone()).await.unwrap();
        assert!(proxy_enabled_on_startup(&db).await);

        config.proxy_enabled = false;
        db.update_global_proxy_config(config).await.unwrap();
        assert!(!proxy_enabled_on_startup(&db).await);
    }
}
