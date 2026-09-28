//! Local OpenAI-compatible listener for Codex.

use super::{
    handlers, log_codes::srv as log_srv, provider_router::ProviderRouter,
    providers::codex_chat_history::CodexChatHistoryStore, types::*, ProxyError,
};
use crate::database::Database;
use axum::{
    routing::{get, post},
    Router,
};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};
use tokio::task::JoinHandle;

/// Proxy server status (shared)
#[derive(Clone)]
pub struct ProxyState {
    pub db: Arc<Database>,
    pub config: Arc<RwLock<ProxyConfig>>,
    pub status: Arc<RwLock<ProxyStatus>>,
    pub start_time: Arc<RwLock<Option<std::time::Instant>>>,
    /// The provider currently used by each app type (app_type -> (provider_id, provider_name))
    pub current_providers: Arc<RwLock<std::collections::HashMap<String, (String, String)>>>,
    /// The selected Copilot provider is resolved once per request.
    pub provider_router: Arc<ProviderRouter>,
    /// Codex Chat bridge history, used to restore the tool call pointed to by previous_response_id
    pub codex_chat_history: Arc<CodexChatHistoryStore>,
    /// AppHandle, used to fire events and update the tray menu
    pub app_handle: Option<tauri::AppHandle>,
}

/// Proxy HTTP server
pub struct ProxyServer {
    config: ProxyConfig,
    state: ProxyState,
    shutdown_tx: Arc<RwLock<Option<oneshot::Sender<()>>>>,
    /// Server task handle, used to wait for the server to actually shut down
    server_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl ProxyServer {
    pub fn new(
        config: ProxyConfig,
        db: Arc<Database>,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        // Share the provider selector across requests.
        let provider_router = Arc::new(ProviderRouter::new(db.clone()));

        let state = ProxyState {
            db,
            config: Arc::new(RwLock::new(config.clone())),
            status: Arc::new(RwLock::new(ProxyStatus::default())),
            start_time: Arc::new(RwLock::new(None)),
            current_providers: Arc::new(RwLock::new(std::collections::HashMap::new())),
            provider_router,
            codex_chat_history: Arc::new(CodexChatHistoryStore::default()),
            app_handle,
        };

        Self {
            config,
            state,
            shutdown_tx: Arc::new(RwLock::new(None)),
            server_handle: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn start(&self) -> Result<ProxyServerInfo, ProxyError> {
        // Check if it is already running
        if self.shutdown_tx.read().await.is_some() {
            return Err(ProxyError::AlreadyRunning);
        }

        let addr: SocketAddr =
            format!("{}:{}", self.config.listen_address, self.config.listen_port)
                .parse()
                .map_err(|e| ProxyError::BindFailed(format!("Invalid address: {e}")))?;

        // Create a closed channel
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        // Build route
        let app = self.build_router();

        // Bind listener
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| ProxyError::BindFailed(e.to_string()))?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| ProxyError::BindFailed(e.to_string()))?;
        let actual_port = local_addr.port();

        log::info!(
            "[{}] Proxy server started at {local_addr}",
            log_srv::STARTED
        );

        // Update global proxy port for system proxy detection
        crate::proxy::http_client::set_proxy_port(actual_port);

        // Save close handle
        *self.shutdown_tx.write().await = Some(shutdown_tx);

        // update status
        let mut status = self.state.status.write().await;
        status.running = true;
        status.address = self.config.listen_address.clone();
        status.port = actual_port;
        drop(status);

        // Record startup time
        *self.state.start_time.write().await = Some(std::time::Instant::now());

        // Starting the server - using manual hyper HTTP/1.1 accept loop
        // Turn on preserve_header_case to capture the original case of client request headers
        let state = self.state.clone();
        let handle = tokio::spawn(async move {
            let mut shutdown_rx = shutdown_rx;
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (stream, _remote_addr) = match result {
                            Ok(v) => v,
                            Err(e) => {
                                log::error!("[{SRV}] accept failed: {e}", SRV = log_srv::ACCEPT_ERR);
                                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                                continue;
                            }
                        };

                        let app = app.clone();
                        tokio::spawn(async move {
                            // service_fn bridges axum Router (tower::Service) to hyper
                            let service = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                                let mut router = app.clone();
                                async move {
                                    // Convert hyper::body::Incoming to axum::body::Body and keep extensions
                                    let (parts, body) = req.into_parts();


                                    let body = axum::body::Body::new(body);
                                    let axum_req = http::Request::from_parts(parts, body);
                                    <Router as tower::Service<http::Request<axum::body::Body>>>::call(&mut router, axum_req).await
                                }
                            });

                            if let Err(e) = hyper::server::conn::http1::Builder::new()
                                .preserve_header_case(true)
                                .serve_connection(TokioIo::new(stream), service)
                                .await
                            {
                                // Connection reset / broken pipe, etc. are very common in proxy scenarios, debug level
                                log::debug!("[{SRV}] connection error: {e}", SRV = log_srv::CONN_ERR);
                            }
                        });
                    }
                    _ = &mut shutdown_rx => {
                        break;
                    }
                }
            }

            // Update status after server is stopped
            state.status.write().await.running = false;
            *state.start_time.write().await = None;
        });

        // Save server task handle
        *self.server_handle.write().await = Some(handle);

        Ok(ProxyServerInfo {
            address: self.config.listen_address.clone(),
            port: actual_port,
            started_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub async fn stop(&self) -> Result<(), ProxyError> {
        // 1. Send a shutdown signal
        if let Some(tx) = self.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        } else {
            return Err(ProxyError::NotRunning);
        }

        // 2. Wait for the server task to end (with 5-second timeout protection)
        if let Some(handle) = self.server_handle.write().await.take() {
            match tokio::time::timeout(std::time::Duration::from_secs(5), handle).await {
                Ok(Ok(())) => {
                    log::info!(
                        "[{}] The proxy server has completely stopped",
                        log_srv::STOPPED
                    );
                    Ok(())
                }
                Ok(Err(e)) => {
                    log::warn!(
                        "[{}] Proxy server task terminated abnormally: {e}",
                        log_srv::TASK_ERROR
                    );
                    Err(ProxyError::StopFailed(e.to_string()))
                }
                Err(_) => {
                    log::warn!(
                        "[{}] Proxy server stop timeout (5 seconds), forced to continue",
                        log_srv::STOP_TIMEOUT
                    );
                    Err(ProxyError::StopTimeout)
                }
            }
        } else {
            Ok(())
        }
    }

    pub async fn get_status(&self) -> ProxyStatus {
        let mut status = self.state.status.read().await.clone();
        status.active_requests.truncate(5);

        // Calculate running time
        if let Some(start) = *self.state.start_time.read().await {
            status.uptime_seconds = start.elapsed().as_secs();
        }

        // Get the providers currently used by each application type from the current_providers HashMap
        let current_providers = self.state.current_providers.read().await;
        status.active_targets = current_providers
            .iter()
            .map(|(app_type, (provider_id, provider_name))| ActiveTarget {
                app_type: app_type.clone(),
                provider_id: provider_id.clone(),
                provider_name: provider_name.clone(),
            })
            .collect();

        status
    }

    fn build_router(&self) -> Router {
        Router::new()
            // health check
            .route("/health", get(handlers::health_check))
            .route("/status", get(handlers::get_status))
            // OpenAI Models API (Codex CLI reachability check)
            .route("/models", get(handlers::handle_models))
            .route("/v1/models", get(handlers::handle_models))
            // OpenAI Responses API (Codex CLI, supports prefix and non-prefix)
            .route("/responses", post(handlers::handle_responses))
            .route("/v1/responses", post(handlers::handle_responses))
            .route("/v1/v1/responses", post(handlers::handle_responses))
            .route("/codex/v1/responses", post(handlers::handle_responses))
            // OpenAI Responses Compact API (Codex CLI remote compression, transparent transmission)
            .route(
                "/responses/compact",
                post(handlers::handle_responses_compact),
            )
            .route(
                "/v1/responses/compact",
                post(handlers::handle_responses_compact),
            )
            .route(
                "/v1/v1/responses/compact",
                post(handlers::handle_responses_compact),
            )
            .route(
                "/codex/v1/responses/compact",
                post(handlers::handle_responses_compact),
            )
            // Codex standalone Alpha Search API. All local aliases normalize to
            // the selected provider's canonical sibling `/alpha/search` route.
            .route("/alpha/search", post(handlers::handle_alpha_search))
            .route("/v1/alpha/search", post(handlers::handle_alpha_search))
            .route("/v1/v1/alpha/search", post(handlers::handle_alpha_search))
            .route(
                "/codex/v1/alpha/search",
                post(handlers::handle_alpha_search),
            )
            // Codex built-in ImageGen still calls the legacy OpenAI Images API.
            .route(
                "/images/generations",
                post(handlers::handle_images_generations),
            )
            .route(
                "/v1/images/generations",
                post(handlers::handle_images_generations),
            )
            .route(
                "/v1/v1/images/generations",
                post(handlers::handle_images_generations),
            )
            .route(
                "/codex/v1/images/generations",
                post(handlers::handle_images_generations),
            )
            // Codex ImageGen posts to `/images/edits` when it references existing images.
            .route("/images/edits", post(handlers::handle_images_edits))
            .route("/v1/images/edits", post(handlers::handle_images_edits))
            .route("/v1/v1/images/edits", post(handlers::handle_images_edits))
            .route(
                "/codex/v1/images/edits",
                post(handlers::handle_images_edits),
            )
            .with_state(self.state.clone())
    }

    /// Update runtime configuration without restarting the service
    pub async fn apply_runtime_config(&self, config: &ProxyConfig) {
        *self.state.config.write().await = config.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Provider;
    use axum::http::StatusCode;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn codex_routes_reject_malformed_json_as_client_error() {
        let proxy = ProxyServer::new(
            ProxyConfig {
                listen_port: 0,
                ..Default::default()
            },
            Arc::new(Database::memory().unwrap()),
            None,
        );
        let info = proxy.start().await.unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for path in [
            "/v1/responses",
            "/v1/responses/compact",
            "/v1/alpha/search",
            "/v1/images/generations",
            "/v1/images/edits",
        ] {
            let response = client
                .post(format!("http://127.0.0.1:{}{path}", info.port))
                .header("content-type", "application/json")
                .body("{invalid json")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        }
        proxy.stop().await.unwrap();
    }

    #[tokio::test]
    async fn codex_routes_reject_legacy_non_copilot_providers_before_forwarding() {
        let requests = Arc::new(AtomicUsize::new(0));
        let capture = requests.clone();
        let upstream = Router::new().fallback(move || {
            let capture = capture.clone();
            async move {
                capture.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }
        });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let upstream_task =
            tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        let db = Arc::new(Database::memory().unwrap());
        let legacy = Provider::with_id(
            "legacy".into(),
            "Legacy custom API".into(),
            json!({
                "base_url": format!("http://{address}/v1"),
                "auth": { "OPENAI_API_KEY": "test-key" }
            }),
        );
        db.save_provider("codex", &legacy).unwrap();
        db.set_current_provider("codex", &legacy.id).unwrap();
        let proxy = ProxyServer::new(
            ProxyConfig {
                listen_port: 0,
                ..Default::default()
            },
            db,
            None,
        );
        let info = proxy.start().await.unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for path in [
            "/v1/responses",
            "/v1/responses/compact",
            "/v1/alpha/search",
            "/v1/images/generations",
            "/v1/images/edits",
        ] {
            let response = client
                .post(format!("http://127.0.0.1:{}{path}", info.port))
                .json(&json!({ "model": "test-model", "input": "hello" }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
        }
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        proxy.stop().await.unwrap();
        upstream_task.abort();
    }
}
