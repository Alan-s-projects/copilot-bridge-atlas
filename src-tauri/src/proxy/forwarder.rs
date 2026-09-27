//! Forward Codex requests to one authenticated GitHub Copilot account.
use super::{
    body_filter::filter_private_params,
    content_encoding::{decompress_body_with_limit, get_content_encoding},
    json_canonical::{canonicalize_value, short_value_hash},
    providers::{
        codex_chat_history::CodexChatHistoryStore,
        copilot_auth::{build_copilot_request_headers, CopilotAuthError},
        copilot_model_map::{
            is_valid_model_id, CopilotProtocol, CopilotTransport, ResolvedCopilotModel,
        },
        inject_codex_chat_prompt_cache_key, is_codex_responses_endpoint,
    },
    types::{CopilotOptimizerConfig, ProxyStatus, ReasoningEffort},
    upstream_response::{ProxyResponse, MAX_RESPONSE_BODY_BYTES},
    ProxyError,
};
use crate::{commands::CopilotAuthState, provider::Provider};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexUpstreamFormat {
    NativeResponses,
    ChatCompletions,
}

struct CopilotRequestContext {
    classification: super::copilot_optimizer::CopilotClassification,
    interaction_id: Option<String>,
}
impl CopilotRequestContext {
    fn apply_headers(
        &self,
        headers: &mut Vec<(http::HeaderName, http::HeaderValue)>,
        classify: bool,
    ) -> Result<(), ProxyError> {
        if classify {
            for (name, value) in headers.iter_mut() {
                if name.as_str() == "x-initiator" {
                    *value = http::HeaderValue::from_static(self.classification.initiator);
                }
            }
        }
        if let Some(id) = &self.interaction_id {
            headers.push((
                http::HeaderName::from_static("x-interaction-id"),
                http::HeaderValue::from_str(id).map_err(|_| {
                    ProxyError::ConfigError("Invalid Copilot interaction ID".into())
                })?,
            ));
        }
        Ok(())
    }
}

pub struct ForwardResult {
    pub response: ProxyResponse,
    pub codex_upstream_format: Option<CodexUpstreamFormat>,
    pub outbound_model: Option<String>,
    pub(crate) connection_guard: Option<ActiveConnectionGuard>,
}

/// The request remains active until the response body finishes or is dropped.
pub(crate) struct ActiveConnectionGuard {
    status: Arc<RwLock<ProxyStatus>>,
}
impl ActiveConnectionGuard {
    pub(crate) async fn acquire(status: Arc<RwLock<ProxyStatus>>) -> Self {
        {
            let mut s = status.write().await;
            s.active_connections = s.active_connections.saturating_add(1);
        }
        Self { status }
    }
}
impl Drop for ActiveConnectionGuard {
    fn drop(&mut self) {
        // Drop 不能 await：把减量操作调度到 tokio runtime
        let status = self.status.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut s = status.write().await;
                s.active_connections = s.active_connections.saturating_sub(1);
            });
        }
        // 没有 runtime 时静默丢失计数（仅 UI 展示用，可接受最终一致性）
    }
}

/// Replace remote account/catalog dependencies in HTTP integration tests only.
#[cfg(test)]
struct CopilotFixture {
    endpoint: String,
    token: String,
    models: Vec<super::providers::copilot_auth::CopilotModel>,
    client: reqwest::Client,
}

pub struct RequestForwarder {
    status: Arc<RwLock<ProxyStatus>>,
    current_providers: Arc<RwLock<std::collections::HashMap<String, (String, String)>>>,
    codex_chat_history: Arc<CodexChatHistoryStore>,
    app_handle: Option<tauri::AppHandle>,
    session_id: String,
    session_client_provided: bool,
    copilot_optimizer_config: CopilotOptimizerConfig,
    #[cfg(test)]
    copilot_fixture: Option<CopilotFixture>,
}
impl RequestForwarder {
    pub fn new(
        status: Arc<RwLock<ProxyStatus>>,
        current_providers: Arc<RwLock<std::collections::HashMap<String, (String, String)>>>,
        codex_chat_history: Arc<CodexChatHistoryStore>,
        app_handle: Option<tauri::AppHandle>,
        session_id: String,
        session_client_provided: bool,
        copilot_optimizer_config: CopilotOptimizerConfig,
    ) -> Self {
        Self {
            status,
            current_providers,
            codex_chat_history,
            app_handle,
            session_id,
            session_client_provided,
            copilot_optimizer_config,
            #[cfg(test)]
            copilot_fixture: None,
        }
    }

    /// Each request is sent once; retry decisions belong to the Codex client.
    pub async fn forward_request(
        &self,
        method: http::Method,
        endpoint: &str,
        body: Value,
        headers: http::HeaderMap,
        provider: &Provider,
        reasoning_effort: &mut ReasoningEffort,
    ) -> Result<ForwardResult, ProxyError> {
        *reasoning_effort = ReasoningEffort::from_request(&body);
        let guard = ActiveConnectionGuard::acquire(self.status.clone()).await;
        {
            let mut status = self.status.write().await;
            status.total_requests = status.total_requests.saturating_add(1);
            status.last_request_at = Some(chrono::Utc::now().to_rfc3339());
            status.current_provider = Some(provider.name.clone());
            status.current_provider_id = Some(provider.id.clone());
        }
        let result = self
            .forward(provider, method, endpoint, body, &headers, reasoning_effort)
            .await;
        let mut status = self.status.write().await;
        match result {
            Ok((response, outbound_model, codex_upstream_format)) => {
                status.success_requests = status.success_requests.saturating_add(1);
                status.last_error = None;
                status.success_rate =
                    status.success_requests as f32 / status.total_requests as f32 * 100.0;
                drop(status);
                self.current_providers.write().await.insert(
                    "codex".to_string(),
                    (provider.id.clone(), provider.name.clone()),
                );
                Ok(ForwardResult {
                    response,
                    outbound_model,
                    codex_upstream_format,
                    connection_guard: Some(guard),
                })
            }
            Err(error) => {
                status.failed_requests = status.failed_requests.saturating_add(1);
                status.last_error = Some(error.to_string());
                status.success_rate =
                    status.success_requests as f32 / status.total_requests as f32 * 100.0;
                Err(error)
            }
        }
    }

    async fn forward(
        &self,
        provider: &Provider,
        method: http::Method,
        endpoint: &str,
        mut body: Value,
        headers: &http::HeaderMap,
        reasoning_effort: &mut ReasoningEffort,
    ) -> Result<(ProxyResponse, Option<String>, Option<CodexUpstreamFormat>), ProxyError> {
        crate::copilot_bridge::require_copilot(provider)
            .map_err(|error| ProxyError::ConfigError(error.to_string()))?;
        if let Some(model) = body.get("model") {
            if !model.as_str().is_some_and(is_valid_model_id) {
                return Err(ProxyError::InvalidRequest(
                    "Model must be a non-empty identifier without whitespace".into(),
                ));
            }
        }
        let is_responses = is_codex_responses_endpoint(endpoint);
        let context = self.codex_copilot_request_context(is_responses, &body);
        let mut upstream_format = None;
        let mut effective_endpoint = endpoint.to_string();
        if is_responses {
            let resolved = self.resolve_codex_copilot_model(provider, &body).await?;
            let transport =
                apply_codex_copilot_model(&mut body, resolved, &provider.settings_config)?;
            effective_endpoint = rewrite_codex_endpoint_for_copilot(endpoint, &transport.endpoint);
            upstream_format = Some(match transport.protocol {
                CopilotProtocol::Responses => CodexUpstreamFormat::NativeResponses,
                CopilotProtocol::Chat => CodexUpstreamFormat::ChatCompletions,
            });
        }
        let chat = upstream_format == Some(CodexUpstreamFormat::ChatCompletions);
        if chat {
            let explicit_cache_key = body
                .get("prompt_cache_key")
                .and_then(Value::as_str)
                .map(str::to_string);
            self.codex_chat_history.enrich_request(&mut body).await;
            body = super::providers::transform_codex_chat::responses_to_chat_completions(body)?;
            inject_codex_chat_prompt_cache_key(
                &mut body,
                explicit_cache_key.as_deref(),
                self.session_client_provided
                    .then_some(self.session_id.as_str()),
            );
        } else if upstream_format == Some(CodexUpstreamFormat::NativeResponses) {
            super::providers::transform_codex_chat::normalize_legacy_chat_message_ids(&mut body);
        }
        let body = prepare_upstream_request_body(body);
        let outbound_model = body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string);
        let streaming = is_streaming_request(&body, headers);
        let (mut auth_headers, base_url) = self.copilot_identity(provider).await?;
        if let Some(context) = context {
            context.apply_headers(
                &mut auth_headers,
                self.copilot_optimizer_config.request_classification,
            )?;
        }
        // Standalone Codex APIs keep their OpenAI /v1 prefix. Responses transport
        // paths come directly from the authenticated Copilot model catalog.
        let url = if is_responses {
            build_copilot_unversioned_url(&base_url, &effective_endpoint)
        } else {
            build_codex_standalone_url(&base_url, &effective_endpoint)?
        };
        let outgoing = upstream_headers(headers, auth_headers, chat || streaming, &url);
        if log::log_enabled!(log::Level::Debug) {
            log::debug!(
                "[Copilot] endpoint={}, model={}, tools_hash={}, input_hash={}, body_hash={}",
                crate::redact_url_for_log(&effective_endpoint),
                outbound_model.as_deref().unwrap_or("none"),
                short_value_hash(body.get("tools")),
                short_value_hash(body.get("input")),
                short_value_hash(Some(&body))
            );
        }
        let body_bytes = if matches!(method, http::Method::GET | http::Method::HEAD) {
            Vec::new()
        } else {
            serde_json::to_vec(&body).map_err(|error| ProxyError::Internal(error.to_string()))?
        };
        // Reuse the pooled transport, including the configured HTTP/SOCKS proxy.
        #[cfg(not(test))]
        let client = super::http_client::get();
        #[cfg(test)]
        let client = self
            .copilot_fixture
            .as_ref()
            .map(|fixture| fixture.client.clone())
            .unwrap_or_else(super::http_client::get);
        let mut request = client.request(method, &url);
        if streaming {
            request = request.timeout(std::time::Duration::from_secs(24 * 60 * 60));
        }
        for (name, value) in &outgoing {
            request = request.header(name, value);
        }
        let request = request.body(body_bytes);
        // Retain the sent value even when the upstream returns an error.
        reasoning_effort.record_applied(&body);
        let response = if streaming {
            let timeout = std::time::Duration::from_secs(600);
            tokio::time::timeout(timeout, request.send())
                .await
                .map_err(|_| {
                    ProxyError::Timeout(format!(
                        "Timed out waiting for Copilot response headers after {}s",
                        timeout.as_secs()
                    ))
                })?
        } else {
            request.send().await
        }
        .map_err(map_reqwest_send_error)?;
        let response = ProxyResponse::Reqwest(response);
        if response.status().is_success() {
            Ok((response, outbound_model, upstream_format))
        } else {
            let status = response.status().as_u16();
            let encoding = get_content_encoding(response.headers());
            let raw = response.bytes_with_limit(MAX_RESPONSE_BODY_BYTES).await?;
            let decoded = encoding
                .and_then(|encoding| {
                    decompress_body_with_limit(&encoding, &raw, MAX_RESPONSE_BODY_BYTES)
                        .ok()
                        .flatten()
                })
                .unwrap_or_else(|| raw.to_vec());
            Err(ProxyError::UpstreamError {
                status,
                body: String::from_utf8(decoded).ok(),
            })
        }
    }

    async fn copilot_identity(
        &self,
        provider: &Provider,
    ) -> Result<(Vec<(http::HeaderName, http::HeaderValue)>, String), ProxyError> {
        #[cfg(test)]
        if let Some(fixture) = &self.copilot_fixture {
            return Ok((
                build_copilot_request_headers(&fixture.token)?,
                fixture.endpoint.clone(),
            ));
        }
        let app = self.app_handle.as_ref().ok_or_else(|| {
            ProxyError::AuthError("GitHub Copilot authentication is unavailable".into())
        })?;
        let state = app.state::<CopilotAuthState>();
        let manager = state.0.read().await;
        let account = provider
            .meta
            .as_ref()
            .and_then(|meta| meta.managed_account_id_for("github_copilot"));
        let token = match account.as_deref() {
            Some(id) => manager.get_valid_token_for_account(id).await,
            None => manager.get_valid_token().await,
        }
        .map_err(|error| {
            ProxyError::AuthError(format!("GitHub Copilot authentication failed: {error}"))
        })?;
        let endpoint = match account.as_deref() {
            Some(id) => manager.get_api_endpoint(id).await,
            None => manager.get_default_api_endpoint().await,
        };
        Ok((build_copilot_request_headers(&token)?, endpoint))
    }

    fn codex_copilot_request_context(
        &self,
        is_codex_copilot_responses: bool,
        body: &Value,
    ) -> Option<CopilotRequestContext> {
        if !is_codex_copilot_responses || !self.copilot_optimizer_config.enabled {
            return None;
        }
        Some(CopilotRequestContext {
            classification: super::copilot_optimizer::classify_responses_request(body),
            // Keep Codex request IDs per-request; only the interaction ID groups a session.
            interaction_id: self
                .session_client_provided
                .then(|| super::copilot_optimizer::deterministic_interaction_id(&self.session_id))
                .flatten(),
        })
    }

    async fn resolve_codex_copilot_model(
        &self,
        provider: &Provider,
        body: &Value,
    ) -> Result<Option<ResolvedCopilotModel>, ProxyError> {
        let model_id = body
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .ok_or_else(|| {
                ProxyError::InvalidRequest(
                    "Codex Copilot requests require a non-empty string model".to_string(),
                )
            })?;
        #[cfg(test)]
        if let Some(fixture) = &self.copilot_fixture {
            return Ok(super::providers::copilot_model_map::resolve_model(
                model_id,
                &fixture.models,
            ));
        }
        let app_handle = self.app_handle.as_ref().ok_or_else(|| {
            ProxyError::ConfigError(format!(
                "GitHub Copilot capabilities for model {model_id} cannot be resolved: AppHandle unavailable"
            ))
        })?;

        let copilot_state = app_handle.state::<CopilotAuthState>();
        let copilot_auth = copilot_state.0.read().await;
        let account_id = provider
            .meta
            .as_ref()
            .and_then(|meta| meta.managed_account_id_for("github_copilot"));
        let resolved = match account_id.as_deref() {
            Some(id) => copilot_auth.resolve_model_for_account(id, model_id).await,
            None => copilot_auth.resolve_model(model_id).await,
        };

        resolved.map_err(|error| codex_copilot_lookup_error(model_id, error))
    }
}

fn split_endpoint_and_query(endpoint: &str) -> (&str, Option<&str>) {
    endpoint
        .split_once('?')
        .map_or((endpoint, None), |(path, query)| (path, Some(query)))
}

fn codex_copilot_lookup_error(model_id: &str, error: CopilotAuthError) -> ProxyError {
    let message =
        format!("Failed to resolve GitHub Copilot capabilities for model {model_id}: {error}");
    match error {
        CopilotAuthError::AuthorizationPending
        | CopilotAuthError::AccessDenied
        | CopilotAuthError::ExpiredToken
        | CopilotAuthError::GitHubTokenInvalid
        | CopilotAuthError::NoCopilotSubscription
        | CopilotAuthError::AccountNotFound(_) => ProxyError::AuthError(message),
        CopilotAuthError::NetworkError(_) | CopilotAuthError::CopilotTokenFetchFailed(_) => {
            ProxyError::ForwardFailed(message)
        }
        CopilotAuthError::ParseError(_)
        | CopilotAuthError::IoError(_)
        | CopilotAuthError::InvalidDomain(_) => ProxyError::ConfigError(message),
    }
}

fn apply_codex_copilot_model(
    body: &mut Value,
    resolved: Option<ResolvedCopilotModel>,
    settings: &Value,
) -> Result<CopilotTransport, ProxyError> {
    let requested = "a supported chat endpoint";
    let Some(resolved) = resolved else {
        let model = body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        return Err(ProxyError::ConfigError(format!(
            "GitHub Copilot model {model} is unavailable for {requested} in this provider's catalogue"
        )));
    };
    let Some(transport) = resolved.transport else {
        return Err(ProxyError::ConfigError(format!(
            "GitHub Copilot model {} does not advertise {requested}",
            resolved.id
        )));
    };
    if let (Some(limit), Some(requested)) = (
        resolved.max_output_tokens,
        body.get("max_output_tokens").and_then(Value::as_u64),
    ) {
        if requested > limit {
            return Err(ProxyError::InvalidRequest(format!(
                "Model {} supports at most {limit} output tokens; requested {requested}",
                resolved.id
            )));
        }
    }
    if resolved.supports_tool_calls == Some(false)
        && body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| !tools.is_empty())
    {
        return Err(ProxyError::InvalidRequest(format!(
            "Model {} does not support tool calls",
            resolved.id
        )));
    }
    if resolved.supports_parallel_tool_calls == Some(false)
        && body.get("parallel_tool_calls").is_some()
    {
        body["parallel_tool_calls"] = json!(false);
    }
    // An explicit empty list means no reasoning support. Do not manufacture a
    // GPT-style effort for this model or forward a disabled-reasoning parameter.
    if resolved
        .reasoning_efforts
        .as_ref()
        .is_some_and(Vec::is_empty)
    {
        if let Some(object) = body.as_object_mut() {
            object.remove("reasoning");
        }
    } else if let Some(supported) = resolved.reasoning_efforts.as_ref() {
        if let Some(effort) = body.pointer("/reasoning/effort").and_then(Value::as_str) {
            if let Some(level) = supported
                .iter()
                .find(|level| level.eq_ignore_ascii_case(effort))
            {
                body["reasoning"]["effort"] = json!(level);
            } else {
                let selected = configured_reasoning_efforts(settings, &resolved.id);
                let allowed = |level: &str| {
                    selected.as_ref().is_none_or(|levels| {
                        levels
                            .iter()
                            .any(|selected| selected.eq_ignore_ascii_case(level))
                    })
                };
                // A synthetic Ultra option must not bypass the model's saved
                // choices. Live capabilities still bound every fallback.
                const CANONICAL_EFFORTS: &[&str] = &[
                    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
                ];
                let fallback = CANONICAL_EFFORTS
                    .iter()
                    .rev()
                    .find(|candidate| {
                        allowed(candidate)
                            && supported.iter().any(|s| s.eq_ignore_ascii_case(candidate))
                    })
                    .copied()
                    .or_else(|| {
                        supported
                            .iter()
                            .rev()
                            .find(|level| allowed(level))
                            .map(String::as_str)
                    });
                if let Some(level) = fallback.filter(|level| *level != "none") {
                    body["reasoning"]["effort"] = json!(level);
                } else if let Some(object) = body.as_object_mut() {
                    object.remove("reasoning");
                }
            }
        }
    }
    body["model"] = Value::String(resolved.id);
    Ok(transport)
}

fn configured_reasoning_efforts(settings: &Value, model: &str) -> Option<Vec<String>> {
    let row = settings
        .pointer("/modelCatalog/models")?
        .as_array()?
        .iter()
        .find(|row| {
            row.get("model")
                .and_then(Value::as_str)
                .is_some_and(|id| id.trim().eq_ignore_ascii_case(model))
        })?;
    ["reasoningLevels", "reasoning_levels"]
        .into_iter()
        .find_map(|key| {
            let levels: Vec<String> = row
                .get(key)?
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|level| !level.is_empty())
                .map(str::to_string)
                .collect();
            (!levels.is_empty()).then_some(levels)
        })
}

fn rewrite_codex_endpoint_for_copilot(inbound_endpoint: &str, supported_endpoint: &str) -> String {
    let (inbound_path, query) = split_endpoint_and_query(inbound_endpoint);
    let mut target_path = supported_endpoint.trim_end_matches('/').to_string();
    if inbound_path.ends_with("/responses/compact")
        && matches!(target_path.as_str(), "/responses" | "/v1/responses")
    {
        target_path.push_str("/compact");
    }
    match query {
        Some(query) if !query.is_empty() => format!("{target_path}?{query}"),
        _ => target_path,
    }
}

fn build_copilot_unversioned_url(base_url: &str, endpoint: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        endpoint.trim_start_matches('/')
    )
}

fn map_reqwest_send_error(error: reqwest::Error) -> ProxyError {
    if error.is_timeout() {
        ProxyError::Timeout(format!("上游请求超时: {}", error.without_url()))
    } else if error.is_connect() {
        ProxyError::ForwardFailed(format!("上游连接失败: {}", error.without_url()))
    } else {
        ProxyError::ForwardFailed(format!("上游请求发送失败: {}", error.without_url()))
    }
}

fn prepare_upstream_request_body(request_body: Value) -> Value {
    canonicalize_value(filter_private_params(request_body))
}

fn is_streaming_request(body: &Value, headers: &http::HeaderMap) -> bool {
    body.get("stream").and_then(Value::as_bool).unwrap_or(false)
        || headers
            .get(http::header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|accept| accept.contains("text/event-stream"))
}

/// Replace client credentials and fingerprints with the selected Copilot account.
fn upstream_headers(
    incoming: &http::HeaderMap,
    auth: Vec<(http::HeaderName, http::HeaderValue)>,
    identity_encoding: bool,
    url: &str,
) -> http::HeaderMap {
    const COPILOT_FINGERPRINT_HEADERS: &[&str] = &[
        "user-agent",
        "editor-version",
        "editor-plugin-version",
        "copilot-integration-id",
        "x-github-api-version",
        "openai-intent",
        "x-initiator",
        "x-interaction-type",
        "x-interaction-id",
        "x-vscode-user-agent-library-version",
        "x-request-id",
        "x-agent-task-id",
    ];
    let host = url
        .parse::<http::Uri>()
        .ok()
        .and_then(|uri| uri.authority().map(|authority| authority.to_string()));
    let mut headers = http::HeaderMap::new();
    let mut saw_auth = false;
    let mut saw_accept_encoding = false;
    for (name, value) in incoming {
        let key = name.as_str();
        if key == "host" {
            if let Some(value) = host
                .as_deref()
                .and_then(|host| http::HeaderValue::from_str(host).ok())
            {
                headers.append(name.clone(), value);
            }
            continue;
        }
        if matches!(
            key,
            "content-length"
                | "transfer-encoding"
                | "x-forwarded-host"
                | "x-forwarded-port"
                | "x-forwarded-proto"
                | "forwarded"
                | "cf-connecting-ip"
                | "cf-ipcountry"
                | "cf-ray"
                | "cf-visitor"
                | "true-client-ip"
                | "fastly-client-ip"
                | "x-azure-clientip"
                | "x-azure-fdid"
                | "x-azure-ref"
                | "akamai-origin-hop"
                | "x-akamai-config-log-detail"
                | "x-request-id"
                | "x-correlation-id"
                | "x-trace-id"
                | "x-amzn-trace-id"
                | "x-b3-traceid"
                | "x-b3-spanid"
                | "x-b3-parentspanid"
                | "x-b3-sampled"
                | "traceparent"
                | "tracestate"
        ) {
            continue;
        }
        if matches!(key, "authorization" | "x-api-key" | "x-goog-api-key") {
            if !saw_auth {
                saw_auth = true;
                for (name, value) in &auth {
                    headers.append(name.clone(), value.clone());
                }
            }
            continue;
        }
        if key == "accept-encoding" {
            if !saw_accept_encoding {
                saw_accept_encoding = true;
                headers.append(
                    name.clone(),
                    if identity_encoding {
                        http::HeaderValue::from_static("identity")
                    } else {
                        value.clone()
                    },
                );
            }
            continue;
        }
        // These belong to another wire protocol and were never forwarded by
        // the existing Codex/Copilot path.
        if matches!(key, "anthropic-beta" | "anthropic-version")
            || COPILOT_FINGERPRINT_HEADERS.contains(&key)
        {
            continue;
        }
        headers.append(name.clone(), value.clone());
    }
    if !saw_auth {
        for (name, value) in auth {
            headers.append(name, value);
        }
    }
    if identity_encoding && !saw_accept_encoding {
        headers.append(
            http::header::ACCEPT_ENCODING,
            http::HeaderValue::from_static("identity"),
        );
    }
    if !headers.contains_key(http::header::CONTENT_TYPE) {
        headers.insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
    }
    headers
}

#[derive(Clone, Copy)]
enum CodexStandaloneEndpoint {
    AlphaSearch,
    ImagesGenerations,
    ImagesEdits,
}

impl CodexStandaloneEndpoint {
    fn from_effective_endpoint(endpoint: &str) -> Option<Self> {
        match split_endpoint_and_query(endpoint).0 {
            "/alpha/search" => Some(Self::AlphaSearch),
            "/images/generations" => Some(Self::ImagesGenerations),
            "/images/edits" => Some(Self::ImagesEdits),
            _ => None,
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::AlphaSearch => "/alpha/search",
            Self::ImagesGenerations => "/images/generations",
            Self::ImagesEdits => "/images/edits",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::AlphaSearch => "Codex Alpha Search",
            Self::ImagesGenerations => "Codex Images generations",
            Self::ImagesEdits => "Codex Images edits",
        }
    }

    fn full_url_hint(self) -> &'static str {
        match self {
            Self::AlphaSearch => "/responses",
            Self::ImagesGenerations | Self::ImagesEdits => {
                "/responses, /chat/completions, /images/generations, or /images/edits"
            }
        }
    }

    /// Full-URL suffixes that unambiguously locate this endpoint's sibling.
    ///
    /// Order matters: a longer suffix must precede any suffix it ends with
    /// (`/responses/compact` before `/responses`), otherwise the shorter one
    /// wins and the rewrite keeps a stray `/compact` segment.
    fn source_suffixes(self) -> &'static [&'static str] {
        match self {
            Self::AlphaSearch => &["/responses/compact", "/responses"],
            // Both Images routes live next to each other, so a full URL pasted
            // for either one is a valid source for the other.
            Self::ImagesGenerations | Self::ImagesEdits => &[
                "/images/generations",
                "/images/edits",
                "/chat/completions",
                "/responses/compact",
                "/responses",
            ],
        }
    }

    fn source_suffix(self, parsed_path: &str) -> Option<&'static str> {
        // Match the case-insensitive pasted-endpoint check. Only normalize for
        // matching; the rewrite keeps the original URL prefix and query intact.
        let parsed_path = parsed_path.to_ascii_lowercase();
        self.source_suffixes()
            .iter()
            .copied()
            .find(|suffix| parsed_path.ends_with(suffix))
    }

    /// Whether a base URL (full-URL switch off) already ends in one of this
    /// endpoint's source suffixes, i.e. was pasted as a complete endpoint URL.
    fn base_url_is_source_endpoint(self, base_url: &str) -> bool {
        self.source_suffixes()
            .iter()
            .any(|suffix| base_url_is_full_endpoint(base_url, suffix))
    }
}

fn rewrite_codex_standalone_full_url(
    base_url: &str,
    request_query: Option<&str>,
    endpoint: CodexStandaloneEndpoint,
) -> Result<String, ProxyError> {
    let trimmed = base_url.trim();
    let parsed = url::Url::parse(trimmed).map_err(|_| {
        ProxyError::ConfigError(format!("{} requires a valid full URL", endpoint.label()))
    })?;

    // Fragments are never sent in HTTP requests. Drop one before splitting the
    // query so an accidental fragment cannot move the incoming query behind `#`.
    let without_fragment = trimmed
        .split_once('#')
        .map_or(trimmed, |(head, _fragment)| head);
    let (url_without_query, base_query) = without_fragment
        .split_once('?')
        .map_or((without_fragment, None), |(head, query)| {
            (head, Some(query))
        });
    let url_without_query = url_without_query.trim_end_matches('/');

    let parsed_path = parsed.path().trim_end_matches('/').to_string();
    let suffix = endpoint.source_suffix(&parsed_path).ok_or_else(|| {
        ProxyError::ConfigError(format!(
            "{} cannot derive {} from an opaque full URL; use a base URL or a full URL ending in {}",
            endpoint.label(),
            endpoint.path(),
            endpoint.full_url_hint()
        ))
    })?;

    let prefix_len = url_without_query
        .len()
        .checked_sub(suffix.len())
        .ok_or_else(|| ProxyError::ConfigError("Invalid Codex full URL".to_string()))?;
    let mut rewritten = format!("{}{}", &url_without_query[..prefix_len], endpoint.path());

    let request_query = request_query.filter(|query| !query.is_empty());
    let base_query = base_query.filter(|query| !query.is_empty());
    match (base_query, request_query) {
        (Some(base), Some(request)) => rewritten.push_str(&format!("?{base}&{request}")),
        (Some(base), None) => rewritten.push_str(&format!("?{base}")),
        (None, Some(request)) => rewritten.push_str(&format!("?{request}")),
        (None, None) => {}
    }

    Ok(rewritten)
}

fn base_url_is_full_endpoint(base_url: &str, endpoint_suffix: &str) -> bool {
    let trimmed = base_url.trim();
    // Match against the path only: a `?query`/`#fragment` on a full endpoint URL must not
    // hide the suffix (`.../v1/messages?beta=true` still ends in the endpoint).
    let path = match trimmed.split_once(['?', '#']) {
        Some((head, _)) => head,
        None => trimmed,
    };
    path.trim_end_matches('/')
        .to_ascii_lowercase()
        .ends_with(endpoint_suffix)
}

fn is_origin_only_url(value: &str) -> bool {
    let trimmed = value.trim_end_matches('/');
    match trimmed.split_once("://") {
        Some((_scheme, rest)) => !rest.contains('/'),
        None => !trimmed.contains('/'),
    }
}
fn build_codex_standalone_url(base_url: &str, endpoint: &str) -> Result<String, ProxyError> {
    if let Some(kind) = CodexStandaloneEndpoint::from_effective_endpoint(endpoint)
        .filter(|kind| kind.base_url_is_source_endpoint(base_url))
    {
        return rewrite_codex_standalone_full_url(
            base_url,
            split_endpoint_and_query(endpoint).1,
            kind,
        );
    }
    let base = base_url.trim_end_matches('/');
    let path = endpoint.trim_start_matches('/');
    let mut url = if base.ends_with("/v1") || !is_origin_only_url(base) {
        format!("{base}/{path}")
    } else {
        format!("{base}/v1/{path}")
    };
    while url.contains("/v1/v1") {
        url = url.replace("/v1/v1", "/v1");
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http::{HeaderMap, HeaderValue, StatusCode};
    use serde_json::json;
    use std::{collections::HashMap, time::Duration};
    fn test_forwarder() -> RequestForwarder {
        RequestForwarder {
            status: Arc::new(RwLock::new(ProxyStatus::default())),
            current_providers: Arc::new(RwLock::new(HashMap::new())),
            codex_chat_history: Arc::new(CodexChatHistoryStore::default()),
            app_handle: None,
            session_id: String::new(),
            session_client_provided: false,
            copilot_optimizer_config: CopilotOptimizerConfig::default(),
            copilot_fixture: None,
        }
    }
    #[test]
    fn prepare_upstream_request_body_filters_private_fields_and_canonicalizes_order() {
        let body = json!({
            "z": 1,
            "_internal": "drop",
            "tools": [
                {
                    "name": "lookup",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "_id": {
                                "_private_note": "drop",
                                "type": "string"
                            },
                            "b": {"type": "number"},
                            "a": {"type": "string"}
                        }
                    }
                }
            ],
            "a": 2
        });

        let prepared = prepare_upstream_request_body(body);

        assert!(prepared.get("_internal").is_none());
        assert!(prepared["tools"][0]["parameters"]["properties"]
            .get("_id")
            .is_some());
        assert!(prepared["tools"][0]["parameters"]["properties"]["_id"]
            .get("_private_note")
            .is_none());
        assert_eq!(
            serde_json::to_string(&prepared).unwrap(),
            r#"{"a":2,"tools":[{"name":"lookup","parameters":{"properties":{"_id":{"type":"string"},"a":{"type":"string"},"b":{"type":"number"}},"type":"object"}}],"z":1}"#
        );
    }
    #[test]
    fn codex_copilot_tool_continuations_receive_agent_and_session_headers() {
        let mut forwarder = test_forwarder();
        forwarder.session_id = "codex_12345678-1234-1234-1234-123456789abc".to_string();
        forwarder.session_client_provided = true;
        for item_type in [
            "function_call_output",
            "custom_tool_call_output",
            "tool_search_output",
        ] {
            for stream in [false, true] {
                let body = json!({
                    "input": [{"type": item_type, "call_id": "call-1", "output": "done"}],
                    "stream": stream
                });
                let headers = codex_copilot_headers(&forwarder, &body);
                assert_eq!(headers["x-initiator"], "agent", "{body}");
                assert_eq!(
                    headers["x-interaction-id"].to_str().unwrap(),
                    super::super::copilot_optimizer::deterministic_interaction_id(
                        &forwarder.session_id
                    )
                    .unwrap()
                );
                assert_eq!(headers["x-interaction-type"], "conversation-agent");
            }
        }
    }
    #[test]
    fn codex_copilot_headers_reuse_client_session_across_turns() {
        let first = json!({"input": "Read the file"});
        let continuation = json!({
            "input": [{"type": "function_call_output", "call_id": "call-1", "output": "done"}]
        });
        for source in ["session_id", "x-session-id", "metadata"] {
            let mut headers = HeaderMap::new();
            let mut body = first.clone();
            let session_id = "12345678-1234-1234-1234-123456789abc";
            if source == "metadata" {
                body["metadata"] = json!({"session_id": session_id});
            } else {
                headers.insert(source, HeaderValue::from_static(session_id));
            }
            let session = super::super::session::extract_session_id(&headers, &body);
            let mut forwarder = test_forwarder();
            forwarder.session_id = session.session_id;
            forwarder.session_client_provided = session.client_provided;
            let first_headers = codex_copilot_headers(&forwarder, &first);
            let next_headers = codex_copilot_headers(&forwarder, &continuation);
            assert_eq!(first_headers["x-initiator"], "user");
            assert_eq!(next_headers["x-initiator"], "agent");
            assert_eq!(
                first_headers["x-interaction-id"], next_headers["x-interaction-id"],
                "{source}"
            );
        }
    }
    fn codex_copilot_headers(forwarder: &RequestForwarder, body: &Value) -> HeaderMap {
        let mut headers =
            super::super::providers::copilot_auth::build_copilot_request_headers("test-token")
                .unwrap();
        let request_id = headers
            .iter()
            .find(|(name, _)| name.as_str() == "x-request-id")
            .unwrap()
            .1
            .clone();
        if let Some(context) = forwarder.codex_copilot_request_context(true, body) {
            context
                .apply_headers(
                    &mut headers,
                    forwarder.copilot_optimizer_config.request_classification,
                )
                .unwrap();
        }
        let headers: HeaderMap = headers.into_iter().collect();
        assert_eq!(headers["x-request-id"], request_id);
        assert_eq!(headers["x-agent-task-id"], request_id);
        headers
    }

    #[tokio::test]
    async fn reasoning_metadata_does_not_claim_an_applied_effort_before_forwarding() {
        let forwarder = test_forwarder();
        let mut provider = Provider::with_id("copilot".into(), "Copilot".into(), json!({}));
        provider.meta = Some(crate::ProviderMeta {
            provider_type: Some("github_copilot".into()),
            ..Default::default()
        });
        let mut reasoning_effort = ReasoningEffort::default();
        let result = forwarder
            .forward_request(
                http::Method::POST,
                "/responses",
                json!({"model":"invalid model","reasoning":{"effort":"ultra"}}),
                HeaderMap::new(),
                &provider,
                &mut reasoning_effort,
            )
            .await;
        assert!(matches!(result, Err(ProxyError::InvalidRequest(_))));
        assert_eq!(reasoning_effort.requested.as_deref(), Some("ultra"));
        assert_eq!(reasoning_effort.applied, None);
    }

    #[tokio::test]
    async fn rejects_invalid_model_identifiers_before_authentication_or_forwarding() {
        let forwarder = test_forwarder();
        let mut provider = Provider::with_id("copilot".into(), "Copilot".into(), json!({}));
        provider.meta = Some(crate::ProviderMeta {
            provider_type: Some("github_copilot".into()),
            ..Default::default()
        });
        let error = forwarder
            .forward(
                &provider,
                http::Method::POST,
                "/responses",
                json!({"model":"invalid model","input":"hello"}),
                &HeaderMap::new(),
                &mut ReasoningEffort::default(),
            )
            .await;
        assert!(matches!(error, Err(ProxyError::InvalidRequest(_))));
    }

    #[test]
    fn respects_advertised_output_tool_and_reasoning_capabilities() {
        let model = super::super::providers::copilot_auth::CopilotModel {
            id: "future/model".into(),
            model_picker_enabled: true,
            supported_endpoints: vec!["/responses".into()],
            max_output_tokens: Some(512),
            supports_tool_calls: Some(false),
            supports_parallel_tool_calls: Some(false),
            reasoning_efforts: Some(vec![]),
            ..Default::default()
        };
        let resolved =
            super::super::providers::copilot_model_map::resolve_model("future/model", &[model])
                .unwrap();
        for mut body in [
            json!({"model":"future/model", "max_output_tokens":513}),
            json!({"model":"future/model", "tools":[{"type":"function","name":"test"}]}),
        ] {
            assert!(matches!(
                apply_codex_copilot_model(&mut body, Some(resolved.clone()), &json!({})),
                Err(ProxyError::InvalidRequest(_))
            ));
        }
        let mut body = json!({"model":"future/model", "max_output_tokens":512, "parallel_tool_calls":true, "reasoning":{"effort":"none"}});
        apply_codex_copilot_model(&mut body, Some(resolved), &json!({})).unwrap();
        assert_eq!(body["parallel_tool_calls"], false);
        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn saved_reasoning_aliases_match_catalog_precedence() {
        let settings = json!({"modelCatalog":{"models":[
            {"model":" canonical ", "reasoningLevels":[" high "], "reasoning_levels":["max"]},
            {"model":"legacy", "reasoningLevels":[], "reasoning_levels":["low","high"]},
            {"model":"unset", "reasoningLevels":[]},
            {"model":"invalid", "reasoningLevels":[null, false, " "]}
        ]}});
        assert_eq!(
            configured_reasoning_efforts(&settings, "CANONICAL"),
            Some(vec!["high".into()])
        );
        assert_eq!(
            configured_reasoning_efforts(&settings, "legacy"),
            Some(vec!["low".into(), "high".into()])
        );
        for model in ["unset", "invalid", "missing"] {
            assert_eq!(configured_reasoning_efforts(&settings, model), None);
        }
    }

    #[test]
    fn clamps_unsupported_reasoning_effort_to_highest_supported_level() {
        let model = super::super::providers::copilot_auth::CopilotModel {
            id: "gpt-6-astra".into(),
            name: "GPT-6 Astra".into(),
            vendor: "OpenAI".into(),
            model_picker_enabled: true,
            model_type: "chat".into(),
            policy_state: None,
            context_window: Some(1_050_000),
            max_context_window_tokens: Some(1_050_000),
            max_output_tokens: Some(128_000),
            supports_tool_calls: Some(true),
            supported_endpoints: vec!["/responses".into()],
            supports_parallel_tool_calls: Some(true),
            supports_vision: Some(true),
            reasoning_efforts: Some(vec![
                "low".into(),
                "medium".into(),
                "high".into(),
                "xhigh".into(),
                "max".into(),
            ]),
        };
        let resolved =
            super::super::providers::copilot_model_map::resolve_model("gpt-6-astra", &[model]);
        let mut body = json!({
            "model": "gpt-6-astra",
            "reasoning": { "effort": "ultra" }
        });
        let transport = apply_codex_copilot_model(&mut body, resolved, &json!({})).unwrap();
        assert_eq!(transport.endpoint, "/responses");
        assert_eq!(body["reasoning"]["effort"], "max");
    }
    #[test]
    fn replaces_client_fingerprints_and_preserves_payload_headers() {
        let mut incoming = HeaderMap::new();
        incoming.insert(
            "authorization",
            "Bearer local-client-token".parse().unwrap(),
        );
        incoming.insert("x-session-id", "client-session".parse().unwrap());
        incoming.insert(
            "x-vscode-user-agent-library-version",
            "client-fingerprint".parse().unwrap(),
        );
        incoming.insert("x-interaction-id", "wrong-interaction".parse().unwrap());
        incoming.insert("user-agent", "client-app".parse().unwrap());
        incoming.insert("host", "127.0.0.1:15722".parse().unwrap());
        incoming.insert(
            "content-type",
            "application/json; charset=utf-8".parse().unwrap(),
        );
        incoming.insert("accept-encoding", "gzip".parse().unwrap());
        let outgoing = upstream_headers(
            &incoming,
            build_copilot_request_headers("selected-account-token").unwrap(),
            true,
            "https://api.githubcopilot.com/responses",
        );
        assert_eq!(outgoing["authorization"], "Bearer selected-account-token");
        assert_eq!(outgoing["x-session-id"], "client-session");
        assert_eq!(outgoing["accept-encoding"], "identity");
        assert_eq!(outgoing["content-type"], "application/json; charset=utf-8");
        assert_eq!(outgoing["host"], "api.githubcopilot.com");
        assert_eq!(
            outgoing["x-vscode-user-agent-library-version"],
            "electron-fetch"
        );
        assert_ne!(
            outgoing["x-vscode-user-agent-library-version"],
            incoming["x-vscode-user-agent-library-version"]
        );
        assert!(!outgoing.contains_key("x-interaction-id"));
        let passthrough = upstream_headers(
            &incoming,
            build_copilot_request_headers("selected-account-token").unwrap(),
            false,
            "https://api.githubcopilot.com/responses",
        );
        assert_eq!(passthrough["accept-encoding"], "gzip");
    }
    #[test]
    fn standalone_urls_preserve_api_prefixes_and_queries() {
        for endpoint in ["/alpha/search", "/images/generations", "/images/edits"] {
            for base in ["https://copilot.example", "https://copilot.example/v1"] {
                assert_eq!(
                    build_codex_standalone_url(base, endpoint).unwrap(),
                    format!("https://copilot.example/v1{endpoint}")
                );
            }
            assert_eq!(
                build_codex_standalone_url("https://copilot.example/api", endpoint).unwrap(),
                format!("https://copilot.example/api{endpoint}")
            );
        }
        assert_eq!(
            build_codex_standalone_url(
                "https://copilot.example/api/responses?version=1",
                "/images/edits?extra=1"
            )
            .unwrap(),
            "https://copilot.example/api/images/edits?version=1&extra=1"
        );
        assert_eq!(
            rewrite_codex_endpoint_for_copilot("/responses/compact?x=1", "/responses"),
            "/responses/compact?x=1"
        );
    }

    mod http_integration {
        use super::*;
        use crate::proxy::{
            handler_config::codex_stream_usage_event_filter,
            providers::copilot_auth::{CopilotModel, COPILOT_EDITOR_VERSION, COPILOT_USER_AGENT},
            response_processor::{create_logged_passthrough_stream, SseUsageCollector},
            usage::parser::TokenUsage,
        };
        use futures::TryStreamExt;
        use std::io::Write;

        const USER_IMAGE: &str = "data:image/png;base64,VVNFUl9JTUFHRQ==";
        const TOOL_IMAGE: &str = "data:image/png;base64,VE9PTF9JTUFHRQ==";
        const TEST_TOKEN: &str = "local-http-test-copilot-token";

        #[derive(Clone)]
        struct CapturedRequest {
            uri: http::Uri,
            headers: HeaderMap,
            body: Value,
        }

        struct MockUpstream {
            endpoint: String,
            requests: Arc<tokio::sync::Mutex<Vec<CapturedRequest>>>,
            task: tokio::task::JoinHandle<()>,
        }

        impl Drop for MockUpstream {
            fn drop(&mut self) {
                self.task.abort();
            }
        }

        async fn mock_upstream(
            status: StatusCode,
            response_headers: HeaderMap,
            reply: Bytes,
        ) -> MockUpstream {
            let requests = Arc::new(tokio::sync::Mutex::new(Vec::new()));
            let capture = requests.clone();
            let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
                let capture = capture.clone();
                let response_headers = response_headers.clone();
                let reply = reply.clone();
                async move {
                    let (parts, body) = request.into_parts();
                    let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
                    capture.lock().await.push(CapturedRequest {
                        uri: parts.uri,
                        headers: parts.headers,
                        body: serde_json::from_slice(&bytes).unwrap(),
                    });
                    let mut response = http::Response::builder()
                        .status(status)
                        .body(axum::body::Body::from(reply))
                        .unwrap();
                    *response.headers_mut() = response_headers;
                    response
                }
            });
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            MockUpstream {
                endpoint,
                requests,
                task,
            }
        }

        fn fixture(
            upstream: &MockUpstream,
            protocol: CopilotProtocol,
        ) -> (RequestForwarder, Provider) {
            let mut forwarder = test_forwarder();
            forwarder.session_id = "codex_12345678-1234-1234-1234-123456789abc".into();
            forwarder.session_client_provided = true;
            let endpoints = match protocol {
                CopilotProtocol::Responses => vec!["/responses".into(), "/chat/completions".into()],
                CopilotProtocol::Chat => vec!["/chat/completions".into()],
            };
            forwarder.copilot_fixture = Some(CopilotFixture {
                endpoint: upstream.endpoint.clone(),
                token: TEST_TOKEN.into(),
                models: vec![CopilotModel {
                    id: "gpt-6-astra".into(),
                    name: "GPT test model".into(),
                    vendor: "OpenAI".into(),
                    model_picker_enabled: true,
                    supported_endpoints: endpoints,
                    // Even an explicit text-only declaration must never cause
                    // the proxy to strip an image and silently retry as text.
                    supports_vision: Some(false),
                    ..Default::default()
                }],
                // Use the production client builder with an explicit loopback
                // HTTP proxy. This isolates system proxy settings and also
                // proves forwarding still honors the configured proxy.
                client: crate::proxy::http_client::loopback_test_client(&upstream.endpoint)
                    .unwrap(),
            });
            let mut provider =
                Provider::with_id("copilot-http-test".into(), "Copilot".into(), json!({}));
            provider.meta = Some(crate::ProviderMeta {
                provider_type: Some("github_copilot".into()),
                ..Default::default()
            });
            (forwarder, provider)
        }

        fn image_request(stream: bool) -> Value {
            json!({
                "model": "gpt-6-astra", "stream": stream, "_private": "must not leave the proxy",
                "instructions": "Inspect the images.", "reasoning": { "effort": "high" },
                "prompt_cache_key": "explicit-stable-cache-key",
                "tools": [{"type":"function", "name":"capture", "parameters":{
                    "type":"object", "properties":{"_path":{"type":"string"}}
                }}],
                "input": [
                    {"role":"user", "content":[
                        {"type":"input_text", "text":"Compare these pictures."},
                        {"type":"input_image", "image_url":USER_IMAGE, "detail":"high"}
                    ]},
                    {"type":"function_call", "call_id":"call_image", "name":"capture", "arguments":"{}"},
                    {"type":"function_call_output", "call_id":"call_image", "output":[
                        {"type":"image", "mimeType":"image/png", "data":"VE9PTF9JTUFHRQ=="}
                    ]}
                ]
            })
        }

        #[tokio::test]
        async fn reasoning_metadata_survives_upstream_errors_in_both_protocols() {
            for protocol in [CopilotProtocol::Responses, CopilotProtocol::Chat] {
                for stream in [false, true] {
                    let upstream = mock_upstream(
                        StatusCode::BAD_REQUEST,
                        HeaderMap::new(),
                        Bytes::from_static(br#"{"error":{"message":"fixture rejection"}}"#),
                    )
                    .await;
                    let (mut forwarder, provider) = fixture(&upstream, protocol);
                    forwarder.copilot_fixture.as_mut().unwrap().models[0].reasoning_efforts =
                        Some(vec!["low".into(), "high".into()]);
                    let mut reasoning_effort = ReasoningEffort::default();
                    let result = forwarder.forward_request(
                        http::Method::POST, "/responses",
                        json!({"model":"gpt-6-astra","input":"Hello","stream":stream,"reasoning":{"effort":"ultra"}}),
                        HeaderMap::new(), &provider, &mut reasoning_effort,
                    ).await;
                    assert!(matches!(
                        result,
                        Err(ProxyError::UpstreamError { status: 400, .. })
                    ));
                    assert_eq!(reasoning_effort.requested.as_deref(), Some("ultra"));
                    assert_eq!(reasoning_effort.applied.as_deref(), Some("high"));
                    let requests = upstream.requests.lock().await;
                    assert_eq!(requests.len(), 1);
                    let path = if protocol == CopilotProtocol::Chat {
                        "/reasoning_effort"
                    } else {
                        "/reasoning/effort"
                    };
                    assert_eq!(
                        requests[0].body.pointer(path).and_then(Value::as_str),
                        Some("high")
                    );
                }
            }
        }

        #[tokio::test]
        async fn review_native_forwarding_repairs_old_chat_ids_without_changing_history() {
            let upstream =
                mock_upstream(StatusCode::OK, HeaderMap::new(), Bytes::from_static(b"{}")).await;
            let (forwarder, provider) = fixture(&upstream, CopilotProtocol::Responses);
            let old_id = "resp_YAW4aoOxCrLasbwPwIq4oQU_msg";
            for endpoint in ["/responses", "/responses/compact"] {
                for stream in [false, true] {
                    let input = json!([
                        {"type":"message", "role":"assistant", "id":old_id, "content":[
                            {"type":"output_text", "text":"Keep the complete answer.", "annotations":[]}
                        ]},
                        {"type":"message", "role":"assistant", "id":"msg_native", "content":[]},
                        {"role":"user", "content":"Continue here."},
                        {"type":"reasoning", "id":"rs_native", "encrypted_content":"opaque"},
                        {"type":"function_call", "id":"fc_native", "call_id":"call_1", "name":"read", "arguments":"{}"},
                        {"type":"function_call_output", "call_id":"call_1", "output":old_id},
                        {"type":"item_reference", "id":old_id}
                    ]);
                    let result = forwarder
                        .forward_request(
                            http::Method::POST,
                            endpoint,
                            json!({"model":"gpt-6-astra", "input":input, "stream":stream}),
                            HeaderMap::new(),
                            &provider,
                            &mut ReasoningEffort::default(),
                        )
                        .await
                        .unwrap();
                    assert_eq!(
                        result.codex_upstream_format,
                        Some(CodexUpstreamFormat::NativeResponses)
                    );
                    let requests = upstream.requests.lock().await;
                    let sent = &requests.last().unwrap().body["input"];
                    assert_eq!(
                        sent.as_array().unwrap().len(),
                        input.as_array().unwrap().len()
                    );
                    assert_eq!(sent[0]["id"], "msg_resp_YAW4aoOxCrLasbwPwIq4oQU");
                    assert_eq!(sent[0]["content"], input[0]["content"]);
                    assert_eq!(sent[6]["id"], sent[0]["id"]);
                    for index in 1..6 {
                        assert_eq!(sent[index], input[index]);
                    }
                }
            }
        }

        #[tokio::test]
        async fn review_reasoning_fallback_respects_saved_choices_for_both_protocols() {
            for protocol in [CopilotProtocol::Responses, CopilotProtocol::Chat] {
                for (supported, selected, requested, expected) in [
                    (
                        vec!["low", "high", "max"],
                        json!(["low", "high"]),
                        "ultra",
                        Some("high"),
                    ),
                    (
                        vec!["low", "high", "max"],
                        json!(["low", "high"]),
                        "unknown",
                        Some("high"),
                    ),
                    (
                        vec!["low", "high", "max"],
                        Value::Null,
                        "ultra",
                        Some("max"),
                    ),
                    (
                        vec!["low", "high", "ultra"],
                        Value::Null,
                        "unknown",
                        Some("ultra"),
                    ),
                    (
                        vec!["low", "high"],
                        json!(["low", "high"]),
                        "low",
                        Some("low"),
                    ),
                    (vec!["low", "high"], json!(["retired"]), "ultra", None),
                    (vec![], Value::Null, "ultra", None),
                ] {
                    let upstream =
                        mock_upstream(StatusCode::OK, HeaderMap::new(), Bytes::from_static(b"{}"))
                            .await;
                    let (mut forwarder, mut provider) = fixture(&upstream, protocol);
                    forwarder.copilot_fixture.as_mut().unwrap().models[0].reasoning_efforts =
                        Some(supported.iter().map(|effort| effort.to_string()).collect());
                    provider.settings_config = json!({"modelCatalog":{"models":[{
                        "model":"GPT-6-ASTRA", "reasoningLevels":selected
                    }]}});
                    let mut reasoning_effort = ReasoningEffort::default();
                    forwarder.forward_request(
                        http::Method::POST, "/responses",
                        json!({"model":"gpt-6-astra", "input":"Hello", "reasoning":{"effort":requested}}),
                        HeaderMap::new(), &provider, &mut reasoning_effort,
                    ).await.unwrap();
                    let requests = upstream.requests.lock().await;
                    let body = &requests[0].body;
                    let effort = match protocol {
                        CopilotProtocol::Responses => body.pointer("/reasoning/effort"),
                        CopilotProtocol::Chat => body.get("reasoning_effort"),
                    }
                    .and_then(Value::as_str);
                    assert_eq!(effort, expected, "{protocol:?} {supported:?} {selected}");
                    assert_eq!(reasoning_effort.requested.as_deref(), Some(requested));
                    assert_eq!(reasoning_effort.applied.as_deref(), expected);
                }
            }
        }

        #[tokio::test]
        async fn automatically_routes_gemini_grok_and_unseen_vendors_despite_retired_overrides() {
            for (model_id, protocol) in [
                ("gemini-future", CopilotProtocol::Chat),
                ("grok-future", CopilotProtocol::Responses),
                ("mai-future", CopilotProtocol::Responses),
                ("new-vendor/agent", CopilotProtocol::Chat),
            ] {
                let upstream =
                    mock_upstream(StatusCode::OK, HeaderMap::new(), Bytes::from_static(b"{}"))
                        .await;
                let (mut forwarder, mut provider) = fixture(&upstream, protocol);
                forwarder.copilot_fixture.as_mut().unwrap().models[0].id = model_id.into();
                provider.meta = Some(
                    serde_json::from_value(json!({
                        "providerType":"github_copilot", "codexCopilotApiFormat":"openai_responses"
                    }))
                    .unwrap(),
                );
                let mut body = image_request(false);
                body["model"] = json!(model_id);
                let result = forwarder
                    .forward_request(
                        http::Method::POST,
                        "/v1/responses",
                        body,
                        client_headers(),
                        &provider,
                        &mut ReasoningEffort::default(),
                    )
                    .await
                    .unwrap();
                assert_eq!(result.outbound_model.as_deref(), Some(model_id));
                let expected = match protocol {
                    CopilotProtocol::Responses => {
                        ("/responses", CodexUpstreamFormat::NativeResponses)
                    }
                    CopilotProtocol::Chat => {
                        ("/chat/completions", CodexUpstreamFormat::ChatCompletions)
                    }
                };
                assert_eq!(result.codex_upstream_format, Some(expected.1));
                let requests = upstream.requests.lock().await;
                assert_eq!(requests.len(), 1);
                assert_eq!(requests[0].uri.path(), expected.0);
                assert_eq!(requests[0].body["model"], model_id);
                if protocol == CopilotProtocol::Chat {
                    assert!(requests[0].body["messages"].is_array());
                    assert!(requests[0].body["tools"][0]["function"].is_object());
                } else {
                    assert!(requests[0].body["input"].is_array());
                }
            }
        }

        fn client_headers() -> HeaderMap {
            HeaderMap::from_iter([
                (
                    http::header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer PROXY_MANAGED"),
                ),
                (
                    http::header::USER_AGENT,
                    HeaderValue::from_static("local-codex-client"),
                ),
                (
                    http::header::ACCEPT_ENCODING,
                    HeaderValue::from_static("gzip"),
                ),
                (
                    http::HeaderName::from_static("x-interaction-id"),
                    HeaderValue::from_static("stale-client-interaction"),
                ),
            ])
        }

        fn assert_sent_request(
            captured: &CapturedRequest,
            original: &Value,
            protocol: CopilotProtocol,
        ) {
            // Absolute-form request targets demonstrate the production client's
            // explicit HTTP proxy was used, even for this loopback destination.
            assert_eq!(captured.uri.scheme_str(), Some("http"));
            let expected_path = match protocol {
                CopilotProtocol::Responses => "/responses?fixture=images",
                CopilotProtocol::Chat => "/chat/completions?fixture=images",
            };
            assert_eq!(
                captured.uri.path_and_query().unwrap().as_str(),
                expected_path
            );
            assert_eq!(
                captured.headers["authorization"].to_str().unwrap(),
                format!("Bearer {TEST_TOKEN}")
            );
            assert_eq!(captured.headers["user-agent"], COPILOT_USER_AGENT);
            assert_eq!(captured.headers["editor-version"], COPILOT_EDITOR_VERSION);
            assert_eq!(captured.headers["x-initiator"], "agent");
            assert_eq!(
                captured.headers["x-request-id"],
                captured.headers["x-agent-task-id"]
            );
            assert_ne!(
                captured.headers["x-interaction-id"],
                "stale-client-interaction"
            );
            assert_eq!(captured.body["model"], "gpt-6-astra");
            assert_eq!(
                captured.body["prompt_cache_key"],
                "explicit-stable-cache-key"
            );
            assert!(captured.body.get("_private").is_none());
            assert!(!captured.body.to_string().contains("[Unsupported Image]"));
            match protocol {
                CopilotProtocol::Responses => {
                    assert_eq!(captured.body["input"], original["input"]);
                    assert_eq!(captured.body["reasoning"], original["reasoning"]);
                    assert_eq!(
                        captured.body["tools"][0]["parameters"]["properties"]["_path"]["type"],
                        "string"
                    );
                }
                CopilotProtocol::Chat => {
                    assert!(captured.body.get("input").is_none());
                    assert_eq!(captured.body["reasoning_effort"], "high");
                    assert_eq!(
                        captured.body["tools"][0]["function"]["parameters"]["properties"]["_path"]
                            ["type"],
                        "string"
                    );
                    let messages = captured.body["messages"].as_array().unwrap();
                    let image_urls: Vec<&str> = messages
                        .iter()
                        .filter(|message| message["role"] == "user")
                        .filter_map(|message| message["content"].as_array())
                        .flatten()
                        .filter_map(|part| part.pointer("/image_url/url").and_then(Value::as_str))
                        .collect();
                    assert_eq!(image_urls, vec![USER_IMAGE, TOOL_IMAGE]);
                    assert!(messages.iter().any(|message| message["role"] == "tool"
                        && message["tool_call_id"] == "call_image"));
                    assert!(messages.iter().any(|message| message["tool_calls"]
                        .as_array()
                        .is_some_and(|calls| calls.iter().any(|call| call["id"] == "call_image"
                            && call["function"]["name"] == "capture"))));
                }
            }
        }

        async fn wait_until_inactive(forwarder: &RequestForwarder) {
            tokio::time::timeout(Duration::from_secs(1), async {
                while forwarder.status.read().await.active_connections != 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("the response guard should release its active connection");
        }

        #[tokio::test]
        async fn copilot_http_native_and_chat_preserve_images_headers_usage_and_active_guard() {
            for protocol in [CopilotProtocol::Responses, CopilotProtocol::Chat] {
                let reply = match protocol {
                    CopilotProtocol::Responses => format!(
                        "event: response.completed\ndata: {}\n\n",
                        json!({
                            "type":"response.completed", "response":{"id":"resp_fixture", "model":"gpt-6-astra", "usage":{
                                "input_tokens":20, "output_tokens":2, "input_tokens_details":{"cached_tokens":10}
                            }}
                        })
                    ),
                    CopilotProtocol::Chat => format!(
                        "data: {}\n\ndata: [DONE]\n\n",
                        json!({
                            "id":"chatcmpl_fixture", "model":"gpt-6-astra", "choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}],
                            "usage":{"prompt_tokens":20,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":10}}
                        })
                    ),
                };
                let upstream = mock_upstream(
                    StatusCode::OK,
                    HeaderMap::from_iter([(
                        http::header::CONTENT_TYPE,
                        HeaderValue::from_static("text/event-stream"),
                    )]),
                    Bytes::from(reply.clone()),
                )
                .await;
                let (forwarder, provider) = fixture(&upstream, protocol);
                let request = image_request(true);
                let result = match forwarder
                    .forward_request(
                        http::Method::POST,
                        "/responses?fixture=images",
                        request.clone(),
                        client_headers(),
                        &provider,
                        &mut ReasoningEffort::default(),
                    )
                    .await
                {
                    Ok(result) => result,
                    Err(error) => panic!("forwarding failed: {error}"),
                };
                assert_eq!(
                    result.codex_upstream_format,
                    Some(match protocol {
                        CopilotProtocol::Responses => CodexUpstreamFormat::NativeResponses,
                        CopilotProtocol::Chat => CodexUpstreamFormat::ChatCompletions,
                    })
                );
                let status = forwarder.status.read().await.clone();
                assert_eq!(
                    (
                        status.total_requests,
                        status.success_requests,
                        status.failed_requests,
                        status.active_connections
                    ),
                    (1, 1, 0, 1)
                );
                let captured = upstream.requests.lock().await.clone();
                assert_eq!(captured.len(), 1);
                assert_sent_request(&captured[0], &request, protocol);
                assert_eq!(captured[0].headers["accept-encoding"], "identity");
                let usage_events = Arc::new(std::sync::Mutex::new(Vec::new()));
                let recorded = usage_events.clone();
                let collector = SseUsageCollector::new(
                    std::time::Instant::now(),
                    Some(codex_stream_usage_event_filter),
                    move |events, _| {
                        *recorded.lock().unwrap() = events;
                    },
                );
                let stream = create_logged_passthrough_stream(
                    result.response.bytes_stream(),
                    "Codex",
                    Some(collector),
                    result.connection_guard,
                );
                let chunks: Vec<Bytes> = stream.try_collect().await.unwrap();
                assert_eq!(chunks.concat().as_slice(), reply.as_bytes());
                wait_until_inactive(&forwarder).await;
                let events = usage_events.lock().unwrap().clone();
                let usage = TokenUsage::from_codex_stream_events_auto(&events).unwrap();
                assert_eq!(
                    (
                        usage.input_tokens,
                        usage.output_tokens,
                        usage.cache_read_tokens
                    ),
                    (20, 2, 10)
                );
                assert_eq!(forwarder.status.read().await.success_rate, 100.0);
            }
        }

        #[tokio::test]
        async fn copilot_http_unsupported_images_keep_original_error_and_never_retry() {
            const ERROR: &str = r#"{"error":{"code":"unsupported_image","message":"This model does not support images.","details":{"request_id":"upstream-error-id"}}}"#;
            for protocol in [CopilotProtocol::Responses, CopilotProtocol::Chat] {
                for compressed in [false, true] {
                    let mut headers = HeaderMap::from_iter([(
                        http::header::CONTENT_TYPE,
                        HeaderValue::from_static("application/json"),
                    )]);
                    let bytes = if compressed {
                        let mut encoder = flate2::write::GzEncoder::new(
                            Vec::new(),
                            flate2::Compression::default(),
                        );
                        encoder.write_all(ERROR.as_bytes()).unwrap();
                        headers.insert(
                            http::header::CONTENT_ENCODING,
                            HeaderValue::from_static("gzip"),
                        );
                        encoder.finish().unwrap()
                    } else {
                        ERROR.as_bytes().to_vec()
                    };
                    let upstream =
                        mock_upstream(StatusCode::BAD_REQUEST, headers, Bytes::from(bytes)).await;
                    let (forwarder, provider) = fixture(&upstream, protocol);
                    let request = image_request(false);
                    let error = match forwarder
                        .forward_request(
                            http::Method::POST,
                            "/responses?fixture=images",
                            request.clone(),
                            client_headers(),
                            &provider,
                            &mut ReasoningEffort::default(),
                        )
                        .await
                    {
                        Ok(_) => panic!("an unsupported-image response must remain an error"),
                        Err(error) => error,
                    };
                    match error {
                        ProxyError::UpstreamError { status, body } => {
                            assert_eq!(status, 400);
                            assert_eq!(body.as_deref(), Some(ERROR));
                        }
                        other => panic!("unexpected error: {other}"),
                    }
                    wait_until_inactive(&forwarder).await;
                    let status = forwarder.status.read().await.clone();
                    assert_eq!(
                        (
                            status.total_requests,
                            status.success_requests,
                            status.failed_requests,
                            status.active_connections
                        ),
                        (1, 0, 1, 0)
                    );
                    let captured = upstream.requests.lock().await.clone();
                    assert_eq!(
                        captured.len(),
                        1,
                        "unsupported images must not be removed and retried"
                    );
                    assert_sent_request(&captured[0], &request, protocol);
                }
            }
        }
    }
}
