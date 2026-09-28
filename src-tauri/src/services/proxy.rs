//! Atlas owns the local listener and application data. There are no client-file writers.
use crate::database::Database;
use crate::proxy::server::ProxyServer;
use crate::proxy::types::*;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct ProxyService {
    db: Arc<Database>,
    server: Arc<RwLock<Option<ProxyServer>>>,
    app_handle: Arc<RwLock<Option<tauri::AppHandle>>>,
}
impl ProxyService {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            server: Arc::new(RwLock::new(None)),
            app_handle: Arc::new(RwLock::new(None)),
        }
    }
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        futures::executor::block_on(async {
            *self.app_handle.write().await = Some(handle);
        });
    }

    pub async fn start(&self) -> Result<ProxyServerInfo, String> {
        let mut server_guard = self.server.write().await;
        // Persist the server preference in Atlas data only.
        let mut global_config = self
            .db
            .get_global_proxy_config()
            .await
            .map_err(|e| format!("Failed to obtain global proxy configuration: {e}"))?;

        if !global_config.proxy_enabled {
            global_config.proxy_enabled = true;
            self.db
                .update_global_proxy_config(global_config.clone())
                .await
                .map_err(|e| format!("Update agent master switch failed: {e}"))?;
        }

        // 2. Get configuration
        let config = self
            .db
            .get_proxy_config()
            .await
            .map_err(|e| format!("Failed to get proxy configuration: {e}"))?;

        // 3. If already running: ensure persistent state (if necessary) and return current information
        if let Some(server) = server_guard.as_ref() {
            let status = server.get_status().await;
            return Ok(ProxyServerInfo {
                address: status.address,
                port: status.port,
                // The first startup time cannot be retrieved accurately. Just return the current time for UI display.
                started_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        // 4. Create and start the server
        let app_handle = self.app_handle.read().await.clone();
        let server = ProxyServer::new(config.clone(), self.db.clone(), app_handle);
        let info = server
            .start()
            .await
            .map_err(|e| format!("Failed to start proxy server: {e}"))?;
        if let Err(e) = self
            .persist_ephemeral_listen_port_if_needed(&config, info.port)
            .await
        {
            let _ = server.stop().await;
            return Err(e);
        }

        // 5. Save the server instance
        *server_guard = Some(server);

        log::info!("Proxy server started: {}:{}", info.address, info.port);
        Ok(info)
    }

    async fn persist_ephemeral_listen_port_if_needed(
        &self,
        config: &ProxyConfig,
        actual_port: u16,
    ) -> Result<(), String> {
        if config.listen_port != 0 {
            return Ok(());
        }

        // The port is a global field and cannot be used to write back independent retry and timeout configurations for each application through the old interface.
        let mut resolved_config = self
            .db
            .get_global_proxy_config()
            .await
            .map_err(|e| format!("Failed to obtain global proxy configuration: {e}"))?;
        resolved_config.listen_port = actual_port;
        self.db
            .update_global_proxy_config(resolved_config)
            .await
            .map_err(|e| format!("Failed to save dynamic proxy port: {e}"))
    }

    /// Stop this instance's listener without changing the user's saved switch.
    pub async fn shutdown(&self) -> Result<(), String> {
        if let Some(server) = self.server.write().await.take() {
            server
                .stop()
                .await
                .map_err(|e| format!("Cannot stop the proxy listener: {e}"))?;
        }
        Ok(())
    }

    /// A manual Off action also persists the preference for the next launch.
    pub async fn stop(&self) -> Result<(), String> {
        self.shutdown().await?;
        let mut config = self
            .db
            .get_global_proxy_config()
            .await
            .map_err(|e| e.to_string())?;
        if config.proxy_enabled {
            config.proxy_enabled = false;
            self.db
                .update_global_proxy_config(config)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub async fn get_status(&self) -> Result<ProxyStatus, String> {
        if let Some(server) = self.server.read().await.as_ref() {
            Ok(server.get_status().await)
        } else {
            // Return to default state when server is not running
            Ok(ProxyStatus {
                running: false,
                ..Default::default()
            })
        }
    }

    pub async fn get_config(&self) -> Result<ProxyConfig, String> {
        self.db
            .get_proxy_config()
            .await
            .map_err(|e| format!("Failed to get proxy configuration: {e}"))
    }

    pub async fn update_config(&self, config: &ProxyConfig) -> Result<(), String> {
        let new_config = config.clone();

        self.db
            .update_proxy_config(new_config.clone())
            .await
            .map_err(|e| format!("Failed to save agent configuration: {e}"))?;

        // Check the current status of the server
        let mut server_guard = self.server.write().await;
        if server_guard.is_none() {
            return Ok(());
        }

        // Determine whether a restart is required (address or port change)
        let status = server_guard.as_ref().unwrap().get_status().await;
        let require_restart =
            new_config.listen_address != status.address || new_config.listen_port != status.port;

        if require_restart {
            if let Some(server) = server_guard.take() {
                server
                    .stop()
                    .await
                    .map_err(|e| format!("Failed to stop proxy server before restarting: {e}"))?;
            }

            let app_handle = self.app_handle.read().await.clone();
            let new_server = ProxyServer::new(new_config.clone(), self.db.clone(), app_handle);
            let info = new_server
                .start()
                .await
                .map_err(|e| format!("Failed to restart proxy server: {e}"))?;
            if let Err(e) = self
                .persist_ephemeral_listen_port_if_needed(&new_config, info.port)
                .await
            {
                let _ = new_server.stop().await;
                return Err(e);
            }

            *server_guard = Some(new_server);
            log::info!("The proxy configuration has been updated and the server has automatically restarted to apply the latest configuration.");

            // Connection changes are exposed as suggestions; client files stay read-only.
            return Ok(());
        } else if let Some(server) = server_guard.as_ref() {
            server.apply_runtime_config(&new_config).await;
            log::info!(
                "Proxy configuration is applied in real time, no need to restart the proxy server"
            );
        }

        Ok(())
    }

    pub async fn is_running(&self) -> bool {
        self.server.read().await.is_some()
    }
}
