//! Per-request state for the selected Codex Copilot provider.
use crate::provider::Provider;
use crate::proxy::{
    extract_session_id,
    forwarder::RequestForwarder,
    server::ProxyState,
    types::{CopilotOptimizerConfig, ReasoningEffort},
    ProxyError,
};
use axum::http::HeaderMap;
use std::time::Instant;

pub struct RequestContext {
    pub start_time: Instant,
    pub provider: Provider,
    pub request_model: String,
    /// Actual outbound identity anchors request-based pricing.
    pub outbound_model: Option<String>,
    pub reasoning_effort: ReasoningEffort,
    pub tag: &'static str,
    pub app_type_str: &'static str,
    pub session_id: String,
    pub session_client_provided: bool,
    pub copilot_optimizer_config: CopilotOptimizerConfig,
}

impl RequestContext {
    pub async fn new(
        state: &ProxyState,
        body: &serde_json::Value,
        headers: &HeaderMap,
    ) -> Result<Self, ProxyError> {
        let start_time = Instant::now();
        let copilot_optimizer_config = state.db.get_copilot_optimizer_config().unwrap_or_default();
        let request_model = body
            .get("model")
            .and_then(|model| model.as_str())
            .unwrap_or("unknown")
            .to_string();
        let session = extract_session_id(headers, body);
        let provider =
            state
                .provider_router
                .select_provider()
                .await
                .map_err(|error| match error {
                    crate::error::AppError::NoProvidersConfigured => {
                        ProxyError::NoProvidersConfigured
                    }
                    _ => ProxyError::DatabaseError(error.to_string()),
                })?;
        log::debug!(
            "[Codex] Provider: {}, model: {}, session: {} (source: {:?}, client_provided: {})",
            provider.name,
            request_model,
            session.session_id,
            session.source,
            session.client_provided
        );
        Ok(Self {
            start_time,
            provider,
            request_model,
            outbound_model: None,
            reasoning_effort: ReasoningEffort::from_request(body),
            tag: "Codex",
            app_type_str: "codex",
            session_id: session.session_id,
            session_client_provided: session.client_provided,
            copilot_optimizer_config,
        })
    }

    pub fn create_forwarder(&self, state: &ProxyState) -> RequestForwarder {
        RequestForwarder::new(
            state.status.clone(),
            state.current_providers.clone(),
            state.codex_chat_history.clone(),
            state.app_handle.clone(),
            self.session_id.clone(),
            self.session_client_provided,
            self.copilot_optimizer_config.clone(),
        )
    }

    #[inline]
    pub fn latency_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }
}
