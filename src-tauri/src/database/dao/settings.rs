//! Common Settings Data Access Object
//!
//! Provides a common settings store in the form of key-value pairs.

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::params;

impl Database {
    pub fn get_usage_date_range(&self) -> Result<crate::settings::UsageDateRange, AppError> {
        Ok(self
            .get_setting("usage_date_range")?
            .and_then(|json| serde_json::from_str::<crate::settings::UsageDateRange>(&json).ok())
            .filter(|range| range.validate().is_ok())
            .unwrap_or_default())
    }

    pub fn set_usage_date_range(
        &self,
        range: crate::settings::UsageDateRange,
    ) -> Result<(), AppError> {
        range.validate()?;
        self.set_setting(
            "usage_date_range",
            &serde_json::to_string(&range).map_err(|error| AppError::Config(error.to_string()))?,
        )
    }

    pub fn get_usage_trend_grouping(
        &self,
    ) -> Result<crate::services::usage_stats::TrendGrouping, AppError> {
        Ok(self
            .get_setting("usage_trend_grouping")?
            .and_then(|value| {
                serde_json::from_str::<crate::services::usage_stats::TrendGrouping>(&value).ok()
            })
            .filter(|value| value.validate().is_ok())
            .unwrap_or_default())
    }

    pub fn set_usage_trend_grouping(
        &self,
        grouping: crate::services::usage_stats::TrendGrouping,
    ) -> Result<(), AppError> {
        grouping.validate()?;
        let value = serde_json::to_string(&grouping)
            .map_err(|error| AppError::Config(error.to_string()))?;
        self.set_setting("usage_trend_grouping", &value)
    }

    pub fn get_usage_table_columns(&self) -> Result<crate::settings::UsageTableColumns, AppError> {
        Ok(self
            .get_setting("usage_table_columns")?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default())
    }

    pub fn set_usage_table_columns(
        &self,
        table: &str,
        columns: Vec<String>,
    ) -> Result<crate::settings::UsageTableColumns, AppError> {
        use rusqlite::OptionalExtension;
        let conn = lock_conn!(self.conn);
        let saved: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'usage_table_columns'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let mut settings: crate::settings::UsageTableColumns = saved
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        match table {
            "requestLogs" => settings.request_logs = Some(columns),
            "modelStats" => settings.model_stats = Some(columns),
            _ => return Err(AppError::Config("Unknown usage table".into())),
        }
        let json = serde_json::to_string(&settings)
            .map_err(|error| AppError::Database(error.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('usage_table_columns', ?1)",
            [json],
        )?;
        Ok(settings)
    }

    /// Get setting value
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, AppError> {
        let conn = lock_conn!(self.conn);
        let mut stmt = conn
            .prepare("SELECT value FROM settings WHERE key = ?1")
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut rows = stmt
            .query(params![key])
            .map_err(|e| AppError::Database(e.to_string()))?;

        if let Some(row) = rows.next().map_err(|e| AppError::Database(e.to_string()))? {
            Ok(Some(
                row.get(0).map_err(|e| AppError::Database(e.to_string()))?,
            ))
        } else {
            Ok(None)
        }
    }

    /// Set value
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    // --- Global outbound proxy ---

    /// Storage key name for the global proxy URL
    const GLOBAL_PROXY_URL_KEY: &'static str = "global_proxy_url";

    /// Get global outbound proxy URL
    ///
    /// Return None to indicate that the proxy is not configured or has been cleared (direct connection)
    /// Returns Some(url) indicating that the proxy has been configured
    pub fn get_global_proxy_url(&self) -> Result<Option<String>, AppError> {
        self.get_setting(Self::GLOBAL_PROXY_URL_KEY)
    }

    /// Set global outbound proxy URL
    ///
    /// - Pass in a non-empty string: enable proxy
    /// - Pass in empty string or None: clear proxy settings (direct connection)
    pub fn set_global_proxy_url(&self, url: Option<&str>) -> Result<(), AppError> {
        match url {
            Some(u) if !u.trim().is_empty() => {
                self.set_setting(Self::GLOBAL_PROXY_URL_KEY, u.trim())
            }
            _ => {
                // Clear proxy settings
                let conn = lock_conn!(self.conn);
                conn.execute(
                    "DELETE FROM settings WHERE key = ?1",
                    params![Self::GLOBAL_PROXY_URL_KEY],
                )
                .map_err(|e| AppError::Database(e.to_string()))?;
                Ok(())
            }
        }
    }

    // --- Copilot optimizer configuration ---

    /// Get Copilot optimizer configuration
    ///
    /// Returns the configuration, or returns the default value if it does not exist (enabled by default)
    pub fn get_copilot_optimizer_config(
        &self,
    ) -> Result<crate::proxy::types::CopilotOptimizerConfig, AppError> {
        match self.get_setting("copilot_optimizer_config")? {
            Some(json) => serde_json::from_str(&json).map_err(|e| {
                AppError::Database(format!(
                    "Failed to parse Copilot optimizer configuration: {e}"
                ))
            }),
            None => Ok(crate::proxy::types::CopilotOptimizerConfig::default()),
        }
    }
}
