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

    pub async fn set_proxy_enabled(&self, enabled: bool) -> Result<(), AppError> {
        self.ensure_proxy_config_row_exists("codex")?;
        lock_conn!(self.conn).execute("UPDATE proxy_config SET proxy_enabled=?1, updated_at=datetime('now') WHERE app_type='codex'", [enabled])?;
        Ok(())
    }

    pub async fn get_global_proxy_config(&self) -> Result<GlobalProxyConfig, AppError> {
        self.ensure_proxy_config_row_exists("codex")?;
        let mut config = lock_conn!(self.conn)
            .query_row(
                "SELECT proxy_enabled, listen_address, listen_port
             FROM proxy_config WHERE app_type = 'codex'",
                [],
                |row| {
                    Ok(GlobalProxyConfig {
                        proxy_enabled: row.get(0)?,
                        listen_address: row.get(1)?,
                        listen_port: row.get(2)?,
                        claude: Default::default(),
                    })
                },
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        config.claude = self
            .get_setting("claude_gateway")?
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| AppError::Config(e.to_string()))?
            .unwrap_or_default();
        Ok(config)
    }

    pub async fn update_global_proxy_config(
        &self,
        config: GlobalProxyConfig,
    ) -> Result<(), AppError> {
        config.claude.validate(config.listen_port)?;
        config
            .listen_address
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .map_err(|_| AppError::Config("Enter a valid Responses API IP address".into()))?;
        self.ensure_proxy_config_row_exists("codex")?;
        let mut conn = lock_conn!(self.conn);
        let tx = conn.transaction()?;
        tx.execute("UPDATE proxy_config SET proxy_enabled=?1, listen_address=?2, listen_port=?3, updated_at=datetime('now') WHERE app_type='codex'", params![config.proxy_enabled,config.listen_address,config.listen_port])?;
        tx.execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES ('claude_gateway',?1)",
            [
                serde_json::to_string(&config.claude)
                    .map_err(|e| AppError::Config(e.to_string()))?,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub async fn get_proxy_config(&self) -> Result<ProxyConfig, AppError> {
        let global = self.get_global_proxy_config().await?;
        Ok(ProxyConfig {
            listen_address: global.listen_address,
            listen_port: global.listen_port,
            claude: global.claude,
        })
    }

    #[cfg(test)]
    pub async fn update_proxy_config(&self, config: ProxyConfig) -> Result<(), AppError> {
        let mut global = self.get_global_proxy_config().await?;
        global.listen_address = config.listen_address;
        global.listen_port = config.listen_port;
        global.claude = config.claude;
        self.update_global_proxy_config(global).await
    }
}
