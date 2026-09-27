use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::proxy::types::{GlobalProxyConfig, ProxyConfig};
use rusqlite::params;
use rust_decimal::Decimal;
use std::str::FromStr;

pub(crate) const PRICING_SOURCE_RESPONSE: &str = "response";
pub(crate) const PRICING_SOURCE_REQUEST: &str = "request";

pub(crate) fn validate_cost_multiplier(value: &str) -> Result<Decimal, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::Message("Multiplier cannot be empty".into()));
    }
    let parsed = Decimal::from_str(trimmed)
        .map_err(|error| AppError::Message(format!("Invalid multiplier: {value} - {error}")))?;
    if parsed < Decimal::ZERO {
        return Err(AppError::Message("Multiplier cannot be negative".into()));
    }
    Ok(parsed)
}

pub(crate) fn validate_pricing_source(value: &str) -> Result<&str, AppError> {
    match value.trim() {
        PRICING_SOURCE_RESPONSE => Ok(PRICING_SOURCE_RESPONSE),
        PRICING_SOURCE_REQUEST => Ok(PRICING_SOURCE_REQUEST),
        _ => Err(AppError::Message(format!("Invalid pricing mode: {value}"))),
    }
}

impl Database {
    fn ensure_proxy_config_row_exists(&self, app_type: &str) -> Result<(), AppError> {
        crate::copilot_bridge::require_codex(app_type)?;
        lock_conn!(self.conn).execute(
            "INSERT OR IGNORE INTO proxy_config (app_type, listen_port) VALUES ('codex', 15722)",
            [],
        ).map_err(|error| AppError::Database(error.to_string()))?;
        Ok(())
    }

    pub async fn get_global_proxy_config(&self) -> Result<GlobalProxyConfig, AppError> {
        // Keep the retired enable_logging column intact, but never use it to gate usage.
        self.ensure_proxy_config_row_exists("codex")?;
        lock_conn!(self.conn)
            .query_row(
                "SELECT proxy_enabled, listen_address, listen_port
             FROM proxy_config WHERE app_type = 'codex'",
                [],
                |row| {
                    Ok(GlobalProxyConfig {
                        proxy_enabled: row.get(0)?,
                        listen_address: row.get(1)?,
                        listen_port: row.get(2)?,
                    })
                },
            )
            .map_err(|error| AppError::Database(error.to_string()))
    }

    pub async fn update_global_proxy_config(
        &self,
        config: GlobalProxyConfig,
    ) -> Result<(), AppError> {
        self.ensure_proxy_config_row_exists("codex")?;
        lock_conn!(self.conn)
            .execute(
                "UPDATE proxy_config SET proxy_enabled = ?1, listen_address = ?2,
             listen_port = ?3, updated_at = datetime('now')
             WHERE app_type = 'codex'",
                params![
                    config.proxy_enabled,
                    config.listen_address,
                    config.listen_port
                ],
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        Ok(())
    }

    pub async fn get_proxy_config(&self) -> Result<ProxyConfig, AppError> {
        self.ensure_proxy_config_row_exists("codex")?;
        lock_conn!(self.conn)
            .query_row(
                "SELECT listen_address, listen_port
             FROM proxy_config WHERE app_type = 'codex'",
                [],
                |row| {
                    Ok(ProxyConfig {
                        listen_address: row.get(0)?,
                        listen_port: row.get(1)?,
                    })
                },
            )
            .map_err(|error| AppError::Database(error.to_string()))
    }

    pub async fn update_proxy_config(&self, config: ProxyConfig) -> Result<(), AppError> {
        self.ensure_proxy_config_row_exists("codex")?;
        lock_conn!(self.conn)
            .execute(
                "UPDATE proxy_config SET listen_address = ?1, listen_port = ?2,
             updated_at = datetime('now')
             WHERE app_type = 'codex'",
                params![config.listen_address, config.listen_port],
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        Ok(())
    }

    pub async fn get_default_cost_multiplier(&self, app_type: &str) -> Result<String, AppError> {
        self.ensure_proxy_config_row_exists(app_type)?;
        lock_conn!(self.conn)
            .query_row(
                "SELECT default_cost_multiplier FROM proxy_config WHERE app_type = 'codex'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| AppError::Database(error.to_string()))
    }

    pub async fn set_default_cost_multiplier(
        &self,
        app_type: &str,
        value: &str,
    ) -> Result<(), AppError> {
        validate_cost_multiplier(value)?;
        self.ensure_proxy_config_row_exists(app_type)?;
        lock_conn!(self.conn).execute(
            "UPDATE proxy_config SET default_cost_multiplier = ?1, updated_at = datetime('now') WHERE app_type = 'codex'",
            [value.trim()],
        ).map_err(|error| AppError::Database(error.to_string()))?;
        Ok(())
    }

    pub async fn get_pricing_model_source(&self, app_type: &str) -> Result<String, AppError> {
        self.ensure_proxy_config_row_exists(app_type)?;
        lock_conn!(self.conn)
            .query_row(
                "SELECT pricing_model_source FROM proxy_config WHERE app_type = 'codex'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| AppError::Database(error.to_string()))
    }

    pub async fn set_pricing_model_source(
        &self,
        app_type: &str,
        value: &str,
    ) -> Result<(), AppError> {
        let value = validate_pricing_source(value)?;
        self.ensure_proxy_config_row_exists(app_type)?;
        lock_conn!(self.conn).execute(
            "UPDATE proxy_config SET pricing_model_source = ?1, updated_at = datetime('now') WHERE app_type = 'codex'",
            [value],
        ).map_err(|error| AppError::Database(error.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn legacy_logging_flag_is_ignored_and_preserved() {
        let db = Database::memory().unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE proxy_config SET enable_logging = 0 WHERE app_type = 'codex'",
                [],
            )
            .unwrap();
        let mut global = db.get_global_proxy_config().await.unwrap();
        let mut listener = db.get_proxy_config().await.unwrap();
        assert!(serde_json::to_value(&global)
            .unwrap()
            .get("enableLogging")
            .is_none());
        assert!(serde_json::to_value(&listener)
            .unwrap()
            .get("enable_logging")
            .is_none());
        global.listen_port = 15822;
        db.update_global_proxy_config(global).await.unwrap();
        listener.listen_port = 15922;
        db.update_proxy_config(listener).await.unwrap();
        let legacy_flag: bool = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT enable_logging FROM proxy_config WHERE app_type = 'codex'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !legacy_flag,
            "Retired settings are preserved, not rewritten"
        );
        let old_client: GlobalProxyConfig = serde_json::from_value(serde_json::json!({
            "proxyEnabled": true,
            "listenAddress": "127.0.0.1",
            "listenPort": 15922,
            "enableLogging": false
        }))
        .unwrap();
        db.update_global_proxy_config(old_client).await.unwrap();
        assert_eq!(db.get_proxy_config().await.unwrap().listen_port, 15922);
    }

    #[tokio::test]
    async fn pricing_preferences_round_trip_and_reject_invalid_values() {
        let db = Database::memory().unwrap();
        assert_eq!(db.get_default_cost_multiplier("codex").await.unwrap(), "1");
        assert_eq!(
            db.get_pricing_model_source("codex").await.unwrap(),
            "response"
        );
        db.set_default_cost_multiplier("codex", " 1.5 ")
            .await
            .unwrap();
        db.set_pricing_model_source("codex", "request")
            .await
            .unwrap();
        assert_eq!(
            db.get_default_cost_multiplier("codex").await.unwrap(),
            "1.5"
        );
        assert_eq!(
            db.get_pricing_model_source("codex").await.unwrap(),
            "request"
        );
        for invalid in ["", "-1", "not-a-number"] {
            assert!(db
                .set_default_cost_multiplier("codex", invalid)
                .await
                .is_err());
        }
        assert!(db
            .set_pricing_model_source("codex", "invalid")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn listener_updates_preserve_opaque_imported_application_rows() {
        let db = Database::memory().unwrap();
        // Model an imported database whose retired tuning columns must remain
        // intact even though new databases and the runtime DTO omit them.
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "ALTER TABLE proxy_config ADD COLUMN max_retries INTEGER DEFAULT 9;
             ALTER TABLE proxy_config ADD COLUMN streaming_first_byte_timeout INTEGER DEFAULT 111;
             ALTER TABLE proxy_config ADD COLUMN streaming_idle_timeout INTEGER DEFAULT 222;
             ALTER TABLE proxy_config ADD COLUMN non_streaming_timeout INTEGER DEFAULT 333;",
            )
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO proxy_config (app_type, listen_port) VALUES ('retired-client', 9876)",
                [],
            )
            .unwrap();
        let mut config = db.get_global_proxy_config().await.unwrap();
        assert_eq!(config.listen_port, 15722);
        config.proxy_enabled = true;
        config.listen_port = 15822;
        db.update_global_proxy_config(config).await.unwrap();
        assert_eq!(db.get_proxy_config().await.unwrap().listen_port, 15822);
        let mut listener = db.get_proxy_config().await.unwrap();
        listener.listen_port = 15922;
        db.update_proxy_config(listener).await.unwrap();
        let saved = db.get_proxy_config().await.unwrap();
        assert_eq!(saved.listen_port, 15922);
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT listen_port FROM proxy_config WHERE app_type = 'retired-client'",
                    [],
                    |row| row.get::<_, u16>(0)
                )
                .unwrap(),
            9876
        );
        let tuning: (i64, i64, i64, i64) = db.conn.lock().unwrap().query_row(
            "SELECT max_retries, streaming_first_byte_timeout, streaming_idle_timeout, non_streaming_timeout
             FROM proxy_config WHERE app_type = 'codex'",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        ).unwrap();
        assert_eq!(tuning, (9, 111, 222, 333));
    }
}
