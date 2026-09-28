use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::proxy::types::{GlobalProxyConfig, ProxyConfig};
use rusqlite::params;
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
}
