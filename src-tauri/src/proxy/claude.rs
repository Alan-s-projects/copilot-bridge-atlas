//! Claude-compatible gateway. Both listeners share accounting and Copilot forwarding.
use super::{
    claude_protocol::{streaming_responses, transform_responses},
    handlers,
    server::ProxyState,
    ProxyError,
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClaudeConfig {
    pub address: String,
    pub port: u16,
    pub sonnet: String,
    pub opus: String,
    pub haiku: String,
    pub fable: String,
}
impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".into(),
            port: 15723,
            sonnet: String::new(),
            opus: String::new(),
            haiku: String::new(),
            fable: String::new(),
        }
    }
}
impl ClaudeConfig {
    pub fn validate(&self, codex_port: u16) -> Result<(), crate::AppError> {
        self.address
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .map_err(|_| crate::AppError::Config("Enter a valid Messages API IP address".into()))?;
        if self.port != 0 && self.port == codex_port {
            return Err(crate::AppError::Config(
                "Responses and Messages APIs need different ports".into(),
            ));
        }
        for model in [&self.sonnet, &self.opus, &self.haiku, &self.fable] {
            if !model.is_empty() && !super::providers::copilot_model_map::is_valid_model_id(model) {
                return Err(crate::AppError::Config(
                    "Invalid Claude model mapping".into(),
                ));
            }
        }
        Ok(())
    }
}
const ROLES: [(&str, &str); 4] = [
    ("sonnet", "claude-sonnet-5"),
    ("opus", "claude-opus-5"),
    ("haiku", "claude-haiku-4-5"),
    ("fable", "claude-fable-5"),
];
fn role(id: &str) -> Option<&'static str> {
    let id = id
        .strip_suffix("[1m]")
        .or_else(|| id.strip_suffix("[1M]"))
        .unwrap_or(id);
    let tail = id
        .strip_prefix("anthropic/claude-")
        .or_else(|| id.strip_prefix("claude-"))?;
    ROLES
        .iter()
        .find(|(role, _)| {
            tail.strip_prefix(&format!("{role}-"))
                .is_some_and(|v| !v.is_empty())
        })
        .map(|(r, _)| *r)
}
fn configured_models(provider: &crate::Provider) -> Vec<String> {
    provider
        .settings_config
        .pointer("/modelCatalog/models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|m| {
            m.get("enabled").and_then(Value::as_bool) != Some(false)
                && m.get("available").and_then(Value::as_bool) != Some(false)
        })
        .filter_map(|m| m.get("model").and_then(Value::as_str))
        .filter(|m| super::providers::copilot_model_map::is_valid_model_id(m))
        .map(str::to_owned)
        .collect()
}
fn resolve_model(id: &str, config: &ClaudeConfig, models: &[String]) -> Result<String, ProxyError> {
    let primary = if config.sonnet.is_empty() {
        models.first().map(String::as_str).unwrap_or("")
    } else {
        &config.sonnet
    };
    let target = match role(id) {
        Some("sonnet") => primary,
        Some("opus") => {
            if config.opus.is_empty() {
                primary
            } else {
                &config.opus
            }
        }
        Some("haiku") => {
            if config.haiku.is_empty() {
                primary
            } else {
                &config.haiku
            }
        }
        Some("fable") => {
            if config.fable.is_empty() {
                primary
            } else {
                &config.fable
            }
        }
        _ => id,
    };
    if models.iter().any(|m| m == target) {
        Ok(target.into())
    } else {
        Err(ProxyError::InvalidRequest("Select an available Copilot model in Settings → Models and check the Claude mappings in Routing".into()))
    }
}
fn discovery(config: &ClaudeConfig, models: &[String]) -> Value {
    let data:Vec<Value>=ROLES.iter().filter_map(|(_,id)|resolve_model(id,config,models).ok().map(|model|json!({"id":id,"type":"model","display_name":model,"created_at":"2026-01-01T00:00:00Z"}))).collect();
    json!({"first_id":data.first().and_then(|m|m.get("id")),"last_id":data.last().and_then(|m|m.get("id")),"has_more":false,"data":data})
}
pub fn router(state: ProxyState) -> Router {
    Router::new()
        .route("/health", get(handlers::health_check))
        .route("/v1/models", get(models))
        .route("/v1/models/:id", get(model))
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .fallback(|| async { error(StatusCode::NOT_FOUND, "Endpoint not found") })
        .with_state(state)
}
fn error(status: StatusCode, message: &str) -> Response {
    let kind = match status.as_u16() {
        400 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        413 => "request_too_large",
        429 => "rate_limit_error",
        529 => "overloaded_error",
        _ => "api_error",
    };
    (
        status,
        Json(json!({"type":"error","error":{"type":kind,"message":message}})),
    )
        .into_response()
}
fn proxy_error(e: ProxyError) -> Response {
    let status = super::error_mapper::map_proxy_error_to_status(&e);
    error(
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
        &e.to_string(),
    )
}
async fn available(state: &ProxyState) -> Result<(ClaudeConfig, Vec<String>), ProxyError> {
    let provider = state
        .provider_router
        .select_provider()
        .await
        .map_err(|e| ProxyError::ConfigError(e.to_string()))?;
    let config = state.config.read().await.claude.clone();
    Ok((config, configured_models(&provider)))
}
async fn models(State(state): State<ProxyState>) -> Response {
    match available(&state).await {
        Ok((config, models)) => Json(discovery(&config, &models)).into_response(),
        Err(e) => proxy_error(e),
    }
}
async fn model(State(state): State<ProxyState>, Path(id): Path<String>) -> Response {
    match available(&state).await {
        Ok((c, m)) => {
            let list = discovery(&c, &m);
            list["data"]
                .as_array()
                .and_then(|a| a.iter().find(|v| v["id"] == id))
                .cloned()
                .map(|v| Json(v).into_response())
                .unwrap_or_else(|| error(StatusCode::NOT_FOUND, "Model not found"))
        }
        Err(e) => proxy_error(e),
    }
}
async fn read(request: axum::extract::Request) -> Result<(HeaderMap, Value), ProxyError> {
    let (parts, body) = request.into_parts();
    let mut headers = parts.headers;
    let value = handlers::read_codex_request_body(&mut headers, body, 32 * 1024 * 1024).await?;
    Ok((headers, value))
}
fn validate(body: &Value) -> Result<(), ProxyError> {
    if !body.is_object()
        || body.get("model").and_then(Value::as_str).is_none()
        || body.get("messages").and_then(Value::as_array).is_none()
    {
        return Err(ProxyError::InvalidRequest(
            "model and messages are required".into(),
        ));
    }
    if body.get("stream").is_some_and(|s| !s.is_boolean()) {
        return Err(ProxyError::InvalidRequest(
            "stream must be a boolean".into(),
        ));
    }
    for tool in body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if tool
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| t != "custom" && !t.starts_with("web_search_"))
        {
            return Err(ProxyError::InvalidRequest("This Anthropic server tool is not supported; use client-executed tools with input_schema".into()));
        }
    }
    Ok(())
}
async fn messages(State(state): State<ProxyState>, request: axum::extract::Request) -> Response {
    match dispatch(state, request).await {
        Ok(r) => r,
        Err(e) => proxy_error(e),
    }
}
async fn dispatch(
    state: ProxyState,
    request: axum::extract::Request,
) -> Result<Response, ProxyError> {
    let (mut headers, mut body) = read(request).await?;
    validate(&body)?;
    if body
        .get("max_tokens")
        .and_then(Value::as_u64)
        .is_none_or(|n| n == 0)
    {
        return Err(ProxyError::InvalidRequest(
            "max_tokens must be a positive integer".into(),
        ));
    }
    let alias = body["model"].as_str().unwrap().to_string();
    let (config, models) = available(&state).await?;
    body["model"] = json!(resolve_model(&alias, &config, &models)?);
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let web_name = transform_responses::anthropic_web_search_tool_name(&body).map(str::to_owned);
    let web_limit = transform_responses::anthropic_web_search_max_uses(&body);
    let mut converted = transform_responses::anthropic_to_responses(body, None, false, false)?;
    converted["stream"] = json!(true);
    // Route internally, never call our own listening socket or alter the Codex router.
    for name in [
        "host",
        "content-length",
        "content-encoding",
        "transfer-encoding",
        "anthropic-version",
        "anthropic-beta",
        "x-app",
    ] {
        headers.remove(name);
    }
    let sdk_headers: Vec<_> = headers
        .keys()
        .filter(|name| name.as_str().starts_with("x-stainless-"))
        .cloned()
        .collect();
    for name in sdk_headers {
        headers.remove(name);
    }
    headers.insert("content-type", "application/json".parse().unwrap());
    let mut req = axum::extract::Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .body(Body::from(
            serde_json::to_vec(&converted)
                .map_err(|e| ProxyError::TransformError(e.to_string()))?,
        ))
        .unwrap();
    *req.headers_mut() = headers;
    let response = handlers::handle_responses(State(state), req).await?;
    let status = response.status();
    if !status.is_success() {
        let retry = response.headers().get("retry-after").cloned();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .map_err(|e| ProxyError::Internal(e.to_string()))?;
        let value: Value = serde_json::from_slice(&bytes).unwrap_or_default();
        let msg = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("Upstream request failed");
        let mut result = error(status, msg);
        if let Some(v) = retry {
            result.headers_mut().insert("retry-after", v);
        }
        return Ok(result);
    }
    let sse = response
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|v| v.contains("text/event-stream"));
    if stream && sse {
        return Ok(stream_response(
            response.into_body().into_data_stream(),
            alias,
            web_name,
            web_limit,
        ));
    }
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024)
        .await
        .map_err(|e| ProxyError::TransformError(e.to_string()))?;
    let value: Value = if sse {
        let mut found = None;
        for line in String::from_utf8_lossy(&bytes).lines() {
            if let Some(v) = line
                .strip_prefix("data:")
                .and_then(|d| serde_json::from_str::<Value>(d.trim()).ok())
            {
                match v["type"].as_str() {
                    Some("response.completed" | "response.incomplete") => {
                        found = Some(v["response"].clone())
                    }
                    Some("response.failed" | "error") => {
                        return Err(ProxyError::ForwardFailed("Upstream stream failed".into()))
                    }
                    _ => {}
                }
            }
        }
        found.ok_or_else(|| {
            ProxyError::ForwardFailed("Upstream stream ended without a completed response".into())
        })?
    } else {
        serde_json::from_slice(&bytes).map_err(|e| ProxyError::TransformError(e.to_string()))?
    };
    if stream {
        let event = bytes::Bytes::from(format!(
            "event: response.completed\ndata: {}\n\n",
            json!({"type":"response.completed","response":value})
        ));
        return Ok(stream_response(
            futures::stream::iter([Ok::<_, std::io::Error>(event)]),
            alias,
            web_name,
            web_limit,
        ));
    }
    let mut message = transform_responses::responses_to_anthropic(value)?;
    message["model"] = json!(alias);
    Ok(Json(message).into_response())
}
fn stream_response<E: std::error::Error + Send + 'static>(
    input: impl futures::Stream<Item = Result<bytes::Bytes, E>> + Send + 'static,
    alias: String,
    web_name: Option<String>,
    web_limit: Option<u64>,
) -> Response {
    let output =
        streaming_responses::create_anthropic_sse_stream_from_responses_with_web_search_options(
            input, web_name, web_limit,
        )
        .map(move |item| {
            item.map(|bytes| {
                let text = String::from_utf8_lossy(&bytes);
                if !text.starts_with("event: message_start\n") {
                    return bytes;
                }
                let data = text.lines().find_map(|l| l.strip_prefix("data: "));
                if let Some(mut value) = data.and_then(|d| serde_json::from_str::<Value>(d).ok()) {
                    value["message"]["model"] = json!(alias);
                    bytes::Bytes::from(format!("event: message_start\ndata: {value}\n\n"))
                } else {
                    bytes
                }
            })
        });
    return Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(output))
        .unwrap();
}
async fn count_tokens(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Response {
    let result=async{let (_,body)=read(request).await?;validate(&body)?;let (c,m)=available(&state).await?;resolve_model(body["model"].as_str().unwrap(),&c,&m)?;
        // Copilot exposes no exact tokenizer. Mark this local estimate explicitly.
        let text=json!({"system":body.get("system"),"messages":body.get("messages"),"tools":body.get("tools")});
        Ok::<_,ProxyError>(Json(json!({"input_tokens":estimate_tokens(&text)})).into_response())}.await;
    match result {
        Ok(mut r) => {
            r.headers_mut()
                .insert("x-atlas-token-count", "estimate".parse().unwrap());
            r
        }
        Err(e) => proxy_error(e),
    }
}

fn estimate_tokens(value: &Value) -> u64 {
    if value.get("type").and_then(Value::as_str) == Some("image") {
        return 2048;
    }
    match value {
        Value::String(s) => (s.len() as u64 + 2) / 3,
        Value::Array(a) => a.iter().map(estimate_tokens).sum(),
        Value::Object(o) => o.values().map(estimate_tokens).sum::<u64>() + 4,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::{
        forwarder::{CopilotFixture, TEST_COPILOT},
        provider_router::ProviderRouter,
        providers::{codex_chat_history::CodexChatHistoryStore, copilot_auth::CopilotModel},
        server::ProxyServer,
        types::ProxyConfig,
    };
    use crate::{Database, Provider, ProviderMeta};
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };
    use tokio::sync::RwLock;
    fn db() -> Arc<Database> {
        let db = Arc::new(Database::memory().unwrap());
        let mut p = Provider::with_id(
            "claude-test".into(),
            "Copilot".into(),
            json!({"modelCatalog":{"models":[{"model":"gpt-6-astra"}]}}),
        );
        p.meta = Some(ProviderMeta {
            provider_type: Some("github_copilot".into()),
            ..Default::default()
        });
        db.save_provider("codex", &p).unwrap();
        db.set_current_provider("codex", &p.id).unwrap();
        db
    }
    fn state(db: Arc<Database>) -> ProxyState {
        ProxyState {
            db: db.clone(),
            config: Arc::new(RwLock::new(ProxyConfig::default())),
            status: Arc::new(RwLock::new(Default::default())),
            start_time: Arc::new(RwLock::new(None)),
            current_providers: Arc::new(RwLock::new(HashMap::new())),
            provider_router: Arc::new(ProviderRouter::new(db)),
            codex_chat_history: Arc::new(CodexChatHistoryStore::default()),
            app_handle: None,
        }
    }
    fn request(body: Value) -> axum::extract::Request {
        axum::extract::Request::builder()
            .method("POST")
            .uri("/v1/messages")
            .header("content-type", "application/json")
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "claude-code-20250219")
            .header("x-api-key", "client-placeholder")
            .header("x-stainless-lang", "js")
            .header(
                "user-agent",
                "claude-cli/2.1.220 (external, claude-desktop)",
            )
            .body(Body::from(body.to_string()))
            .unwrap()
    }
    #[test]
    fn aliases_are_separate_from_actual_models_and_invalid_mappings_fail() {
        let config = ClaudeConfig::default();
        let models = vec!["gpt-6-astra".into()];
        let list = discovery(&config, &models);
        assert_eq!(list["data"].as_array().unwrap().len(), 4);
        assert_eq!(list["data"][0]["display_name"], "gpt-6-astra");
        assert_eq!(list["data"][0]["id"], "claude-sonnet-5");
        assert_eq!(
            resolve_model("claude-haiku-4-5-20251001", &config, &models).unwrap(),
            "gpt-6-astra"
        );
        assert!(resolve_model("claude-unknown", &config, &models).is_err());
        assert!(ClaudeConfig {
            port: 15722,
            ..Default::default()
        }
        .validate(15722)
        .is_err());
        assert!(validate(&json!({"model":"a","messages":[],"stream":"true"})).is_err());
        assert!(validate(
            &json!({"model":"a","messages":[],"tools":[{"type":"computer_20251124"}]})
        )
        .is_err());
    }
    #[tokio::test]
    async fn two_real_sockets_have_distinct_contracts_and_stop_together() {
        let db = db();
        let proxy = ProxyServer::new(
            ProxyConfig {
                listen_port: 0,
                claude: ClaudeConfig {
                    port: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
            db,
            None,
        );
        let info = proxy.start().await.unwrap();
        let status = proxy.get_status().await;
        assert!(status.running);
        assert_ne!(info.port, status.claude_port);
        let c = reqwest::Client::builder().no_proxy().build().unwrap();
        let code: Value = c
            .get(format!("http://127.0.0.1:{}/v1/models", info.port))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(code.get("models").is_some());
        assert!(code.get("data").is_none());
        let models: Value = c
            .get(format!("http://127.0.0.1:{}/v1/models", status.claude_port))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(models.get("data").is_some());
        assert!(models.get("models").is_none());
        assert_eq!(
            c.post(format!("http://127.0.0.1:{}/v1/messages", info.port))
                .json(&json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let bad = c
            .post(format!(
                "http://127.0.0.1:{}/v1/messages",
                status.claude_port
            ))
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        assert_eq!(bad.json::<Value>().await.unwrap()["type"], "error");
        let count = c
            .post(format!(
                "http://127.0.0.1:{}/v1/messages/count_tokens",
                status.claude_port
            ))
            .json(
                &json!({"model":"claude-sonnet-5","messages":[{"role":"user","content":"Hello"}]}),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(count.status(), StatusCode::OK);
        assert_eq!(count.headers()["x-atlas-token-count"], "estimate");
        proxy.stop().await.unwrap();
        assert!(!proxy.get_status().await.running);
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", info.port))
            .await
            .is_err());
        assert!(
            tokio::net::TcpStream::connect(("127.0.0.1", status.claude_port))
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn occupied_claude_port_does_not_leave_codex_half_started() {
        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = ProxyServer::new(
            ProxyConfig {
                listen_port: 0,
                claude: ClaudeConfig {
                    port: occupied.local_addr().unwrap().port(),
                    ..Default::default()
                },
                ..Default::default()
            },
            db(),
            None,
        );
        assert!(proxy.start().await.is_err());
        assert!(!proxy.get_status().await.running);
    }
    #[tokio::test]
    async fn messages_round_trip_through_copilot_for_chat_responses_streaming_and_json() {
        for chat in [false, true] {
            for streaming in [false, true] {
                let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
                let capture = requests.clone();
                let mock=Router::new().fallback(post(move |headers:HeaderMap, Json(body):Json<Value>|{let cap=capture.clone();async move{
                assert!(!headers.contains_key("anthropic-version"));
                assert!(!headers.contains_key("anthropic-beta"));
                assert!(!headers.contains_key("x-stainless-lang"));
                assert!(!headers.contains_key("x-api-key"));
                let mut requests=cap.lock().unwrap();let followup=!requests.is_empty();requests.push(body);drop(requests);
                let text=if followup{"Sunny"}else{"Hello"};
                let wire=if chat{format!("data: {}\n\ndata: [DONE]\n\n",json!({"id":format!("chatcmpl_{}",uuid::Uuid::new_v4().simple()),"model":"gpt-6-astra","choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2}}))}else{format!("event: response.completed\ndata: {}\n\n",json!({"type":"response.completed","response":{"id":format!("resp_{}",uuid::Uuid::new_v4().simple()),"model":"gpt-6-astra","status":"completed","output":[{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}],"usage":{"input_tokens":10,"output_tokens":2}}}))};
                ([("content-type","text/event-stream")],wire)
            }}));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let endpoint = format!("http://{}", listener.local_addr().unwrap());
                let task = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
                let fixture = CopilotFixture {
                    endpoint,
                    token: "fixture-token".into(),
                    models: vec![CopilotModel {
                        id: "gpt-6-astra".into(),
                        name: "GPT test".into(),
                        vendor: "OpenAI".into(),
                        model_picker_enabled: true,
                        supported_endpoints: vec![if chat {
                            "/chat/completions"
                        } else {
                            "/responses"
                        }
                        .into()],
                        ..Default::default()
                    }],
                    client: reqwest::Client::builder().no_proxy().build().unwrap(),
                };
                let db = db();
                let state = state(db.clone());
                TEST_COPILOT.scope(fixture,async{
                for followup in [false,true]{
                    let content=if followup{json!([{"role":"user","content":"Weather?"},{"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"weather","input":{"city":"Tokyo"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"Sunny"}]}])}else{json!([{"role":"user","content":[{"type":"text","text":"Hello"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"aGVsbG8="}}]}])};
                    let response=messages(State(state.clone()),request(json!({"model":"claude-sonnet-5","max_tokens":50,"stream":streaming,"system":"Be helpful","messages":content,"tools":[{"name":"weather","input_schema":{"type":"object","properties":{"city":{"type":"string"}}}}]}))).await;
                    assert_eq!(response.status(),StatusCode::OK,"chat={chat}, stream={streaming}");let bytes=axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap();let text=String::from_utf8(bytes.to_vec()).unwrap();assert!(text.contains(if followup{"Sunny"}else{"Hello"}),"{text}");assert!(text.contains("claude-sonnet-5"));
                    if streaming{assert!(text.contains("event: message_start"));assert!(text.contains("event: message_stop"));}else{let v:Value=serde_json::from_str(&text).unwrap();assert_eq!(v["type"],"message");assert_eq!(v["role"],"assistant");}
                }
            }).await;
                let rows = requests.lock().unwrap();
                assert_eq!(rows.len(), 2);
                assert!(rows.iter().all(|r| r["model"] == "gpt-6-astra"));
                assert!(rows[0]
                    .to_string()
                    .contains("data:image/png;base64,aGVsbG8="));
                assert!(rows[1].to_string().contains("Sunny"));
                drop(rows);
                let logs = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    loop {
                        let logs = db
                            .get_request_logs(
                                &crate::services::usage_stats::LogFilters::default(),
                                0,
                                20,
                            )
                            .unwrap();
                        if logs.total == 2 {
                            break logs;
                        }
                        tokio::task::yield_now().await
                    }
                })
                .await
                .unwrap();
                assert!(logs
                    .data
                    .iter()
                    .all(|r| r.client.client_name == "Claude Desktop" && r.model == "gpt-6-astra"));
                task.abort();
            }
        }
    }

    #[tokio::test]
    async fn settings_round_trip_and_live_mapping_edits_keep_both_ports() {
        let db = db();
        let mut config = db.get_global_proxy_config().await.unwrap();
        config.listen_port = 0;
        config.claude.port = 0;
        config.claude.sonnet = "gpt-6-astra".into();
        db.update_global_proxy_config(config.clone()).await.unwrap();
        assert_eq!(
            db.get_global_proxy_config().await.unwrap().claude,
            config.claude
        );
        let service = crate::services::ProxyService::new(db.clone());
        service.start().await.unwrap();
        let before = service.get_status().await.unwrap();
        assert_ne!(before.port, before.claude_port);
        let mut next = db.get_global_proxy_config().await.unwrap();
        assert_eq!(next.listen_port, before.port);
        assert_eq!(next.claude.port, before.claude_port);
        next.claude.opus = "gpt-6-astra".into();
        service.update_global_settings(next.clone()).await.unwrap();
        let after = service.get_status().await.unwrap();
        assert_eq!(after.port, before.port);
        assert_eq!(after.claude_port, before.claude_port);
        next.claude.port = 1;
        assert!(service.update_global_settings(next).await.is_err());
        assert_eq!(
            db.get_global_proxy_config().await.unwrap().claude.port,
            before.claude_port
        );
        let mut mapped = db.get_global_proxy_config().await.unwrap();
        mapped.claude.haiku = "gpt-6-astra".into();
        let (stop, save) = tokio::join!(service.stop(), service.update_global_settings(mapped));
        stop.unwrap();
        save.unwrap();
        assert_eq!(
            db.get_global_proxy_config().await.unwrap().claude.haiku,
            "gpt-6-astra"
        );
        assert!(!db.get_global_proxy_config().await.unwrap().proxy_enabled);
    }

    #[tokio::test]
    async fn upstream_errors_use_anthropic_error_envelopes() {
        let mock = Router::new().fallback(|| async {
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error":{"message":"Fixture rate limit"}})),
            )
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        let fixture = CopilotFixture {
            endpoint,
            token: "fixture-token".into(),
            models: vec![CopilotModel {
                id: "gpt-6-astra".into(),
                vendor: "OpenAI".into(),
                model_picker_enabled: true,
                supported_endpoints: vec!["/responses".into()],
                ..Default::default()
            }],
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
        };
        let state = state(db());
        let response=TEST_COPILOT.scope(fixture,messages(State(state),request(json!({"model":"claude-sonnet-5","messages":[{"role":"user","content":"Hello"}],"max_tokens":20})))).await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let value: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value["type"], "error");
        assert_eq!(value["error"]["type"], "rate_limit_error");
        task.abort();
    }

    #[tokio::test]
    #[ignore = "Requires ATLAS_LIVE_AUTH_DIR; makes three small Copilot requests using temporary auth data"]
    async fn live_copilot_claude_tool_round_trip_and_codex_regression() {
        use crate::proxy::providers::copilot_auth::CopilotAuthManager;
        let source =
            std::env::var("ATLAS_LIVE_AUTH_DIR").expect("Set ATLAS_LIVE_AUTH_DIR explicitly");
        let home = tempfile::tempdir().unwrap();
        let auth = home.path().join("copilot_auth.json");
        std::fs::copy(
            std::path::Path::new(&source).join("copilot_auth.json"),
            &auth,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&auth, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let manager = CopilotAuthManager::new(home.path().to_owned());
        let token = manager
            .get_valid_token()
            .await
            .expect("Copilot authentication failed");
        let models = manager
            .fetch_models()
            .await
            .expect("Copilot catalog unavailable");
        assert!(models.iter().any(|m| m.id == "gpt-6-astra"));
        let fixture = CopilotFixture {
            endpoint: manager.get_default_api_endpoint().await,
            token,
            models,
            client: crate::proxy::http_client::get().unwrap(),
        };
        let state = state(db());
        TEST_COPILOT.scope(fixture,async {
            let tool=json!({"name":"echo","description":"Echo the supplied text","input_schema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}});
            let first_user=json!({"role":"user","content":"Call echo with text ATLAS_CLAUDE7_OK."});
            let first=messages(State(state.clone()),request(json!({"model":"claude-sonnet-5","messages":[first_user.clone()],"tools":[tool.clone()],"tool_choice":{"type":"tool","name":"echo"},"max_tokens":512,"output_config":{"effort":"low"}}))).await;
            assert_eq!(first.status(),StatusCode::OK,"Claude tool request failed");
            let first:Value=serde_json::from_slice(&axum::body::to_bytes(first.into_body(),1024*1024).await.unwrap()).unwrap();
            let content=first["content"].as_array().expect("Anthropic content missing");
            let call=content.iter().find(|b|b["type"]=="tool_use").expect("No tool_use returned");
            assert_eq!(call["name"],"echo");
            let second=messages(State(state.clone()),request(json!({"model":"claude-sonnet-5","stream":true,"max_tokens":512,"output_config":{"effort":"low"},"system":"After receiving the tool result, reply with exactly ATLAS_CLAUDE7_OK.","tools":[tool],"messages":[first_user,{"role":"assistant","content":content},{"role":"user","content":[{"type":"tool_result","tool_use_id":call["id"],"content":"ATLAS_CLAUDE7_OK"}]}]}))).await;
            assert_eq!(second.status(),StatusCode::OK);
            let text=String::from_utf8(axum::body::to_bytes(second.into_body(),2*1024*1024).await.unwrap().to_vec()).unwrap();
            let joined: String = text.lines().filter_map(|l|l.strip_prefix("data: ")).filter_map(|d|serde_json::from_str::<Value>(d).ok()).filter_map(|v|v.pointer("/delta/text").and_then(Value::as_str).map(str::to_owned)).collect();
            assert!(joined.contains("ATLAS_CLAUDE7_OK"), "Unexpected streamed text: {joined}");assert!(text.contains("event: message_stop"));
            let mut native=request(json!({"model":"gpt-6-astra","input":"Reply with exactly ATLAS_CODEX_OK.","stream":true,"reasoning":{"effort":"low"}}));
            native.headers_mut().insert("user-agent","codex_exec/0.163.0".parse().unwrap());
            let response=handlers::handle_responses(State(state),native).await.unwrap();assert_eq!(response.status(),StatusCode::OK);
            let text=String::from_utf8(axum::body::to_bytes(response.into_body(),2*1024*1024).await.unwrap().to_vec()).unwrap();assert!(text.contains("ATLAS_CODEX_OK"));assert!(text.contains("response.completed"));
            println!("Live check passed: Claude tool_use, tool_result + Anthropic SSE, and native Codex Responses.");
        }).await;
    }
}
