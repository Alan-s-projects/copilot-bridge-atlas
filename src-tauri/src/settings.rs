use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use crate::{AppError, Database};

/// Database-backed app preferences included in SQL exports and database backups.
/// Missing selections mean all current columns; no browser-local storage is used.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UsageTableColumns {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_logs: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_stats: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UsageDateRange {
    pub preset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_start_date: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_end_date: Option<i64>,
    pub live_end_time: bool,
}

impl Default for UsageDateRange {
    fn default() -> Self {
        Self {
            preset: "today".into(),
            custom_start_date: None,
            custom_end_date: None,
            live_end_time: false,
        }
    }
}

impl UsageDateRange {
    pub fn validate(&self) -> Result<(), AppError> {
        let valid = match self.preset.as_str() {
            "today" | "1d" | "7d" | "14d" | "30d" => true,
            "custom" => self.custom_start_date.is_some_and(|start| {
                chrono::DateTime::from_timestamp(start, 0).is_some()
                    && (self.live_end_time
                        || self.custom_end_date.is_some_and(|end| {
                            end >= start && chrono::DateTime::from_timestamp(end, 0).is_some()
                        }))
            }),
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(AppError::Config("Invalid usage date range".into()))
        }
    }
}

/// Device preferences live alongside Atlas's database. Unknown imported
/// preferences remain opaque, so editing current settings does not erase them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    #[serde(flatten)]
    pub legacy_options: BTreeMap<String, serde_json::Value>,
    pub show_in_tray: bool,
    pub launch_on_startup: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_dashboard_refresh_interval_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Legacy read-only discovery hint; the connection preview owns selection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_config_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_provider_codex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_interval_hours: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_retain_count: Option<u32>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            legacy_options: BTreeMap::new(),
            show_in_tray: true,
            launch_on_startup: false,
            usage_dashboard_refresh_interval_ms: None,
            language: Some("en".into()),
            codex_config_dir: None,
            current_provider_codex: None,
            backup_interval_hours: None,
            backup_retain_count: None,
        }
    }
}

impl AppSettings {
    fn settings_path() -> PathBuf {
        crate::config::get_app_config_dir().join("settings.json")
    }

    fn normalize(&mut self) {
        self.language = Some("en".into());
        self.codex_config_dir = self
            .codex_config_dir
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_owned);
    }

    fn load_from_file() -> Self {
        let path = Self::settings_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(mut settings) => {
                    settings.normalize();
                    settings
                }
                Err(error) => {
                    log::warn!("Cannot parse settings {}: {error}", path.display());
                    Self::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                log::warn!("Cannot read settings {}: {error}", path.display());
                Self::default()
            }
        }
    }
}

static SETTINGS_STORE: OnceLock<RwLock<AppSettings>> = OnceLock::new();

fn settings_store() -> &'static RwLock<AppSettings> {
    SETTINGS_STORE.get_or_init(|| RwLock::new(AppSettings::load_from_file()))
}

pub(crate) fn resolve_override_path(raw: &str) -> PathBuf {
    if raw == "~" {
        return crate::config::get_home_dir();
    }
    if let Some(suffix) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return suffix
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .fold(crate::config::get_home_dir(), |path, part| path.join(part));
    }
    PathBuf::from(raw)
}

pub fn get_settings() -> AppSettings {
    settings_store()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

pub fn get_settings_for_frontend() -> AppSettings {
    let mut settings = get_settings();
    settings.legacy_options.clear();
    settings
}

pub fn update_settings(mut settings: AppSettings) -> Result<(), AppError> {
    settings.normalize();
    let mut guard = settings_store()
        .write()
        .unwrap_or_else(|error| error.into_inner());
    crate::config::write_json_file(&AppSettings::settings_path(), &settings)?;
    *guard = settings;
    Ok(())
}

pub fn reload_settings() -> Result<(), AppError> {
    let settings = AppSettings::load_from_file();
    *settings_store()
        .write()
        .unwrap_or_else(|error| error.into_inner()) = settings;
    Ok(())
}

pub fn get_codex_override_dir() -> Option<PathBuf> {
    get_settings()
        .codex_config_dir
        .as_deref()
        .map(resolve_override_path)
}

pub fn get_current_provider() -> Option<String> {
    get_settings().current_provider_codex
}

pub fn set_current_provider(id: Option<&str>) -> Result<(), AppError> {
    let mut guard = settings_store()
        .write()
        .unwrap_or_else(|error| error.into_inner());
    let mut settings = guard.clone();
    settings.current_provider_codex = id.map(str::to_owned);
    settings.normalize();
    crate::config::write_json_file(&AppSettings::settings_path(), &settings)?;
    *guard = settings;
    Ok(())
}

pub fn get_effective_current_provider(db: &Database) -> Result<Option<String>, AppError> {
    if let Some(id) = get_current_provider() {
        if db.get_provider_by_id(&id, "codex")?.is_some() {
            return Ok(Some(id));
        }
        set_current_provider(None)?;
    }
    db.get_current_provider("codex")
}

pub fn effective_backup_interval_hours() -> u32 {
    get_settings().backup_interval_hours.unwrap_or(24)
}

pub fn effective_backup_retain_count() -> usize {
    get_settings().backup_retain_count.unwrap_or(10).max(1) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inactive_imported_preferences_remain_opaque() {
        let mut settings: AppSettings = serde_json::from_value(json!({
            "launchOnStartup": true,
            "language": "ja",
            "retiredFeature": {"keep": true},
            "localMigrations": {"keep": true}
        }))
        .unwrap();
        settings.normalize();
        assert!(settings.launch_on_startup);
        assert_eq!(settings.language.as_deref(), Some("en"));
        assert_eq!(
            settings.legacy_options["retiredFeature"],
            json!({"keep": true})
        );
        assert_eq!(
            serde_json::to_value(settings).unwrap()["localMigrations"],
            json!({"keep": true})
        );
    }
}
