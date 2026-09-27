use crate::init_status::InitErrorPayload;
use serde::Deserialize;
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/Alan-s-projects/copilot-bridge-atlas/releases/latest";

#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
}

fn release_version(tag: &str) -> Option<([u64; 3], &str)> {
    let version = tag.strip_prefix("atlas-")?;
    let components = version
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let numbers: [u64; 3] = components.try_into().ok()?;
    Some((numbers, version))
}

fn newer_release_version(tag: &str, current: &str) -> Option<String> {
    let (latest, version) = release_version(tag)?;
    let (installed, _) = release_version(&format!("atlas-{current}"))?;
    (latest > installed).then(|| version.to_string())
}

/// Check the latest published release without opening a browser or modifying settings.
#[tauri::command]
pub async fn get_available_release_version() -> Result<Option<String>, String> {
    let release = crate::proxy::http_client::get()
        .get(LATEST_RELEASE_URL)
        .header(reqwest::header::USER_AGENT, "copilot-bridge-atlas")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        // The shared proxy client leaves upstream compression untouched.
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| format!("Could not check for a new release: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Could not check for a new release: {error}"))?
        .json::<LatestRelease>()
        .await
        .map_err(|error| format!("Could not read the latest release: {error}"))?;
    Ok(newer_release_version(
        &release.tag_name,
        env!("CARGO_PKG_VERSION"),
    ))
}

/// 打开外部链接
#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> Result<bool, String> {
    let url = if url.starts_with("http://") || url.starts_with("https://") {
        url
    } else {
        format!("https://{url}")
    };

    app.opener()
        .open_url(&url, None::<String>)
        .map_err(|e| format!("打开链接失败: {e}"))?;

    Ok(true)
}

#[tauri::command]
pub async fn copy_text_to_clipboard(text: String) -> Result<bool, String> {
    // Use spawn_blocking to avoid blocking the async runtime
    // Clipboard access can block on some platforms and may have thread/loop constraints
    tokio::task::spawn_blocking(move || {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("访问系统剪贴板失败: {e}"))?;
        clipboard
            .set_text(text)
            .map_err(|e| format!("写入系统剪贴板失败: {e}"))?;
        Ok(true)
    })
    .await
    .map_err(|e| format!("剪贴板任务执行失败: {e}"))?
}

/// 检查更新
#[tauri::command]
pub async fn check_for_updates(handle: AppHandle) -> Result<bool, String> {
    handle
        .opener()
        .open_url(
            "https://github.com/Alan-s-projects/copilot-bridge-atlas/releases/latest",
            None::<String>,
        )
        .map_err(|e| format!("打开更新页面失败: {e}"))?;

    Ok(true)
}

/// 获取应用启动阶段的初始化错误（若有）。
/// 用于前端在早期主动拉取，避免事件订阅竞态导致的提示缺失。
#[tauri::command]
pub async fn get_init_error() -> Result<Option<InitErrorPayload>, String> {
    Ok(crate::init_status::get_init_error())
}

/// Set the Windows title-bar theme.
/// theme: "dark" | "light" | "system"
#[tauri::command]
pub async fn set_window_theme(window: tauri::Window, theme: String) -> Result<(), String> {
    use tauri::Theme;

    let tauri_theme = match theme.as_str() {
        "dark" => Some(Theme::Dark),
        "light" => Some(Theme::Light),
        _ => None, // system default
    };

    window.set_theme(tauri_theme).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::newer_release_version;

    #[test]
    fn reminder_requires_a_newer_stable_atlas_release() {
        assert_eq!(
            newer_release_version("atlas-5.0.6", "5.0.5"),
            Some("5.0.6".into())
        );
        assert_eq!(
            newer_release_version("atlas-5.1.0", "5.0.9"),
            Some("5.1.0".into())
        );
        assert_eq!(newer_release_version("atlas-5.0.5", "5.0.5"), None);
        assert_eq!(newer_release_version("atlas-5.0.4", "5.0.5"), None);
        for tag in ["5.0.6", "atlas-5.0.6-rc1", "atlas-5.0", "atlas-5.0.6.1"] {
            assert_eq!(newer_release_version(tag, "5.0.5"), None, "{tag}");
        }
    }
}
