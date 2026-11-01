//! Native Responses WebSocket relay. An upgrade is not evidence of model support:
//! every response.create is authorized against the pinned account's raw catalog.
//! No reconnect, replay, HTTP fallback, client-selected origin, or unbounded queues.
use super::{
    forwarder::ActiveConnectionGuard,
    handler_context::RequestContext,
    providers::{
        copilot_auth::build_copilot_request_headers, copilot_model_map::model_enabled_in_settings,
    },
    response_processor::log_usage,
    server::ProxyState,
    usage::parser::TokenUsage,
    ProxyError,
};
use crate::{commands::CopilotAuthState, provider::Provider};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Request, State,
    },
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use http::{HeaderMap, HeaderValue, StatusCode};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tauri::Manager;
use tokio::{
    sync::watch,
    time::{timeout, Instant},
};
use tokio_tungstenite::{
    tungstenite::{
        handshake::derive_accept_key,
        protocol::{Role, WebSocketConfig},
        Message as UpMessage,
    },
    WebSocketStream,
};

const MAX_MESSAGE: usize = 16 * 1024 * 1024;
const MAX_RESPONSES: usize = 4096;
const MAX_RESPONSE_ID_BYTES: usize = 4096;
const MAX_RETAINED_ID_BYTES: usize = 8 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(15);
const AUTH_TIMEOUT: Duration = Duration::from_secs(30);
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const CONNECTION_LIFETIME: Duration = Duration::from_secs(3600);

/// Server inserts this extension on GET routes; true or sender drop closes relays.
#[derive(Clone)]
pub struct ResponsesWebSocketShutdown(pub watch::Receiver<bool>);

struct Identity {
    account: String,
    url: url::Url,
    headers: HeaderMap,
    #[cfg(test)]
    fixture: Option<TestFixture>,
}

#[cfg(test)]
struct TestFixture {
    models: Vec<super::providers::copilot_auth::CopilotModel>,
    client: reqwest::Client,
}

pub async fn handle_responses_websocket(
    State(state): State<ProxyState>,
    upgrade: WebSocketUpgrade,
    request: Request,
) -> Result<Response, ProxyError> {
    if request.headers().contains_key(http::header::ORIGIN) {
        return Ok((StatusCode::FORBIDDEN, axum::Json(json!({"error": {
            "type": "invalid_request_error", "message": "Responses WebSocket accepts origin-less native clients only"
        }}))).into_response());
    }
    let shutdown = request
        .extensions()
        .get::<ResponsesWebSocketShutdown>()
        .cloned()
        .ok_or_else(|| {
            ProxyError::ConfigError("Responses WebSocket shutdown signal unavailable".into())
        })?;
    let provider = state
        .provider_router
        .select_provider()
        .await
        .map_err(|_| ProxyError::NoProvidersConfigured)?;
    let identity = timeout(AUTH_TIMEOUT, identity(&state, &provider))
        .await
        .map_err(|_| {
            ProxyError::AuthError("Copilot WebSocket authentication timed out".into())
        })??;
    let headers = request.headers().clone();
    Ok(upgrade
        .max_message_size(MAX_MESSAGE)
        .max_frame_size(MAX_MESSAGE)
        .max_write_buffer_size(MAX_MESSAGE * 2)
        .on_upgrade(move |socket| relay(socket, state, provider, identity, headers, shutdown.0)))
}

async fn identity(state: &ProxyState, provider: &Provider) -> Result<Identity, ProxyError> {
    let app = state
        .app_handle
        .as_ref()
        .ok_or_else(|| ProxyError::AuthError("Copilot authentication unavailable".into()))?;
    let auth = app.state::<CopilotAuthState>();
    let manager = auth.0.read().await;
    let account = match provider
        .meta
        .as_ref()
        .and_then(|m| m.managed_account_id_for("github_copilot"))
    {
        Some(id) => id,
        None => manager
            .get_status()
            .await
            .default_account_id
            .ok_or_else(|| ProxyError::AuthError("No default Copilot account".into()))?,
    };
    let token = manager
        .get_valid_token_for_account(&account)
        .await
        .map_err(|_| ProxyError::AuthError("Copilot WebSocket authentication failed".into()))?;
    let url = upstream_url(&manager.get_api_endpoint(&account).await)?;
    let mut headers = HeaderMap::new();
    for (name, value) in build_copilot_request_headers(&token)? {
        headers.insert(name, value);
    }
    // Never forward the inbound Authorization, Cookie, Host, Origin, or connection headers.
    headers.insert(
        http::header::ORIGIN,
        HeaderValue::from_str(&url.origin().ascii_serialization())
            .map_err(|_| ProxyError::ConfigError("Invalid Copilot origin".into()))?,
    );
    Ok(Identity {
        account,
        url,
        headers,
        #[cfg(test)]
        fixture: None,
    })
}

fn upstream_url(base: &str) -> Result<url::Url, ProxyError> {
    let mut url = url::Url::parse(base)
        .map_err(|_| ProxyError::ConfigError("Invalid Copilot API endpoint".into()))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProxyError::ConfigError(
            "Copilot WebSocket requires an authenticated HTTPS API origin".into(),
        ));
    }
    url.set_path(&format!("{}/responses", url.path().trim_end_matches('/')));
    Ok(url)
}

type Upstream = WebSocketStream<reqwest::Upgraded>;

/// Uses the exact HTTP transport (including HTTP/SOCKS proxy and TLS policy).
/// reqwest performs CONNECT where appropriate, then supplies the upgraded stream.
async fn connect(client: reqwest::Client, identity: &Identity) -> Result<Upstream, &'static str> {
    let key = tokio_tungstenite::tungstenite::handshake::client::generate_key();
    let response = timeout(
        AUTH_TIMEOUT,
        client
            .get(identity.url.clone())
            .version(http::Version::HTTP_11)
            .headers(identity.headers.clone())
            .header("connection", "Upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", &key)
            .send(),
    )
    .await
    .map_err(|_| "Upstream WebSocket handshake timed out")?
    .map_err(|_| "Upstream WebSocket handshake failed")?;
    if response.url() != &identity.url
        || response.status() != StatusCode::SWITCHING_PROTOCOLS
        || !header_has_token(response.headers(), "connection", "upgrade")
        || !header_has_token(response.headers(), "upgrade", "websocket")
        || response
            .headers()
            .get("sec-websocket-accept")
            .and_then(|h| h.to_str().ok())
            != Some(derive_accept_key(key.as_bytes()).as_str())
        || response.headers().contains_key("sec-websocket-extensions")
        || response.headers().contains_key("sec-websocket-protocol")
    {
        return Err("Upstream did not accept the native Responses WebSocket protocol");
    }
    let upgraded = timeout(AUTH_TIMEOUT, response.upgrade())
        .await
        .map_err(|_| "Upstream WebSocket upgrade timed out")?
        .map_err(|_| "Upstream WebSocket upgrade failed")?;
    let config = WebSocketConfig {
        max_message_size: Some(MAX_MESSAGE),
        max_frame_size: Some(MAX_MESSAGE),
        max_write_buffer_size: MAX_MESSAGE * 2,
        ..Default::default()
    };
    Ok(WebSocketStream::from_raw_socket(upgraded, Role::Client, Some(config)).await)
}

fn header_has_token(headers: &HeaderMap, name: &str, token: &str) -> bool {
    headers
        .get_all(name)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|s| s.split(','))
        .any(|s| s.trim().eq_ignore_ascii_case(token))
}

#[derive(Clone)]
struct CompletedResponse {
    model: String,
    terminal_id: String,
}

#[derive(Default)]
struct Protocol {
    completed: HashMap<String, CompletedResponse>,
    // Includes both created and terminal aliases of retired responses.
    terminal: HashSet<String>,
    completed_count: usize,
    retained_id_bytes: usize,
    active_id: Option<String>,
    active_aliases: HashSet<String>,
    identity_limit_exceeded: bool,
    active_model: Option<String>,
}
impl Protocol {
    fn prepare(&self, value: &mut Value) -> Result<bool, &'static str> {
        match value.get("type").and_then(Value::as_str) {
            Some("response.create") => {
                if self.active_model.is_some() {
                    return Err("Only one response may be active per connection");
                }
                if self.completed_count >= MAX_RESPONSES {
                    return Err("Connection response limit reached; open a new connection");
                }
                let model = match value.get("model") {
                    Some(Value::String(m)) if !m.trim().is_empty() => m.clone(),
                    None => value.get("previous_response_id").and_then(Value::as_str)
                        .and_then(|id| self.completed.get(id)).map(|response| response.model.clone())
                        .ok_or("A model is required unless previous_response_id identifies a completed response on this connection")?,
                    _ => return Err("model must be a non-empty string"),
                };
                value["model"] = Value::String(model);
                // Copilot can rotate its opaque response ID at completion. The
                // terminal identity, not its provisional created alias, is the
                // authoritative continuation handle sent back upstream.
                if let Some(completed) = value
                    .get("previous_response_id")
                    .and_then(Value::as_str)
                    .and_then(|id| self.completed.get(id))
                {
                    value["previous_response_id"] = Value::String(completed.terminal_id.clone());
                }
                Ok(true)
            }
            Some("response.cancel") => {
                if self.active_model.is_none() {
                    return Err("No active response to cancel");
                }
                if let Some(id) = value.get("response_id") {
                    if id.as_str().is_none() || id.as_str() != self.active_id.as_deref() {
                        return Err("Cancellation response_id does not match the active response");
                    }
                }
                Ok(false)
            }
            _ => Err("Only response.create and response.cancel are supported"),
        }
    }
    fn started(&mut self, model: String) {
        self.active_model = Some(model);
        self.active_id = None;
        self.active_aliases.clear();
    }
    /// Correlate the sole in-flight request even when Copilot rotates the opaque
    /// ID between response.created and its terminal event. Never reinterpret a
    /// known retired alias as the current request. Native event bytes stay intact.
    fn observe<'a>(&mut self, event: &'a Value) -> Option<&'a Value> {
        let kind = event.get("type").and_then(Value::as_str)?;
        let response = event.get("response")?;
        let id = response.get("id").and_then(Value::as_str)?;
        if id.is_empty() || id.len() > MAX_RESPONSE_ID_BYTES {
            // Do not silently forward an unaccountable completion and leave the
            // connection stuck active. Surface a protocol error and close it.
            self.identity_limit_exceeded = true;
            return None;
        }
        if self.terminal.contains(id) || self.active_model.is_none() {
            return None;
        }
        if kind == "response.created" {
            // Bound repeated created aliases too, rather than growing an
            // attacker-controlled set or forgetting stale response identities.
            if self.active_aliases.len() >= 4 && !self.active_aliases.contains(id) {
                self.identity_limit_exceeded = true;
                return None;
            }
            self.active_aliases.insert(id.to_owned());
            self.active_id = Some(id.to_owned());
        }
        if matches!(
            kind,
            "response.completed" | "response.failed" | "response.incomplete" | "response.cancelled"
        ) {
            self.active_aliases.insert(id.to_owned());
            self.completed_count += 1;
            let model = self.active_model.take().unwrap();
            for alias in self.active_aliases.drain() {
                // Budget the actual retained identity strings: set key plus
                // successful continuation-map key and terminal handle.
                self.retained_id_bytes += alias.len();
                if kind == "response.completed" {
                    self.retained_id_bytes += alias.len() + id.len();
                }
                self.terminal.insert(alias.clone());
                if kind == "response.completed" {
                    self.completed.insert(
                        alias,
                        CompletedResponse {
                            model: model.clone(),
                            terminal_id: id.to_owned(),
                        },
                    );
                }
            }
            self.active_id = None;
            // Log this terminal first, then close instead of evicting aliases
            // that would allow a stale event to consume a later request.
            if self.retained_id_bytes >= MAX_RETAINED_ID_BYTES {
                self.identity_limit_exceeded = true;
            }
            return Some(response);
        }
        None
    }
}

struct Active {
    ctx: RequestContext,
    _guard: ActiveConnectionGuard,
    first_event_ms: Option<u64>,
}

async fn authorize(
    state: &ProxyState,
    provider: &Provider,
    identity: &Identity,
    body: &mut Value,
) -> Result<(), &'static str> {
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .ok_or("model required")?;
    check_current_model_setting(state, provider, model)?;
    let resolved = resolve_model(state, identity, model).await?;
    // A catalog refresh can await network I/O. Recheck deletion/disable after
    // that await, but never replace the connection's account or endpoint.
    check_current_model_setting(state, provider, &resolved.id)?;
    body["model"] = Value::String(resolved.id);
    Ok(())
}

fn check_current_model_setting(
    state: &ProxyState,
    provider: &Provider,
    model: &str,
) -> Result<(), &'static str> {
    let current = state
        .db
        .get_provider_by_id(&provider.id, "codex")
        .map_err(|_| "Pinned provider settings unavailable")?
        .ok_or("Pinned provider no longer exists")?;
    if !model_enabled_in_settings(&current.settings_config, model) {
        return Err("Model is disabled in provider settings");
    }
    Ok(())
}

async fn resolve_model(
    state: &ProxyState,
    identity: &Identity,
    model: &str,
) -> Result<super::providers::copilot_auth::CopilotModel, &'static str> {
    #[cfg(test)]
    if let Some(fixture) = &identity.fixture {
        return super::providers::copilot_model_map::resolve_endpoint_model(
            model,
            &fixture.models,
            "ws:/responses",
        )
        .ok_or("Model does not advertise ws:/responses for this account");
    }
    let app = state
        .app_handle
        .as_ref()
        .ok_or("Copilot authentication unavailable")?;
    let auth = app.state::<CopilotAuthState>();
    let manager = auth.0.read().await;
    manager
        .resolve_endpoint_model_for_account(&identity.account, model, "ws:/responses")
        .await
        .map_err(|_| "Copilot model authorization failed")?
        .ok_or("Model does not advertise ws:/responses for this account")
}

async fn client_send(socket: &mut WebSocket, message: Message) -> bool {
    matches!(timeout(IO_TIMEOUT, socket.send(message)).await, Ok(Ok(())))
}
async fn upstream_send(socket: &mut Upstream, message: UpMessage) -> bool {
    matches!(timeout(IO_TIMEOUT, socket.send(message)).await, Ok(Ok(())))
}
async fn local_error(socket: &mut WebSocket, message: &str) -> bool {
    client_send(socket, Message::Text(json!({"type":"error","error":{"type":"invalid_request_error","code":"atlas_websocket_error","message":message}}).to_string())).await
}

async fn relay(
    mut socket: WebSocket,
    state: ProxyState,
    provider: Provider,
    identity: Identity,
    headers: HeaderMap,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut protocol = Protocol::default();
    let mut active: Option<Active> = None;
    let mut upstream: Option<Upstream> = None;
    let deadline = Instant::now() + CONNECTION_LIFETIME;
    let mut last_activity = Instant::now();
    let mut last_upstream = Instant::now();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    loop {
        if *shutdown.borrow() {
            break;
        }
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tokio::time::sleep_until(deadline) => break,
            _ = heartbeat.tick() => {
                if last_activity.elapsed() >= IDLE_TIMEOUT { break; }
                if let Some(up) = &mut upstream {
                    if last_upstream.elapsed() > Duration::from_secs(90) || !upstream_send(up, UpMessage::Ping(Vec::new())).await { break; }
                }
                if !client_send(&mut socket, Message::Ping(Vec::new())).await { break; }
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        last_activity = Instant::now();
                        let mut body: Value = match serde_json::from_str(&text) {
                            Ok(body) => body,
                            Err(_) => { if !local_error(&mut socket, "Expected a JSON object event").await { break; } continue; }
                        };
                        let create = match protocol.prepare(&mut body) {
                            Ok(create) => create,
                            Err(message) => { if !local_error(&mut socket, message).await { break; } continue; }
                        };
                        if create {
                            let request_body = body.clone();
                            match timeout(AUTH_TIMEOUT, authorize(&state, &provider, &identity, &mut body)).await {
                                Ok(Ok(())) => {},
                                result => {
                                    let message = result.ok().and_then(Result::err).unwrap_or("Model authorization timed out");
                                    if !local_error(&mut socket, message).await { break; } continue;
                                }
                            }
                            if upstream.is_none() {
                                let client = match super::http_client::get_no_redirect() {
                                    Ok(client) => client,
                                    Err(_) => { local_error(&mut socket, "Outbound proxy configuration unavailable").await; break; }
                                };
                                #[cfg(test)]
                                let client = identity.fixture.as_ref().map(|f| f.client.clone()).unwrap_or(client);
                                match connect(client, &identity).await {
                                    Ok(up) => { upstream = Some(up); last_upstream = Instant::now(); },
                                    Err(message) => { local_error(&mut socket, message).await; break; }
                                }
                            }
                            if let Err(message) = check_current_model_setting(&state, &provider, body["model"].as_str().unwrap()) {
                                if !local_error(&mut socket, message).await { break; }
                                continue;
                            }
                            let ctx = match RequestContext::new_for_provider(&state, &request_body, &headers, provider.clone()).await {
                                Ok(mut ctx) => { ctx.outbound_model = body["model"].as_str().map(str::to_owned); ctx },
                                Err(_) => { local_error(&mut socket, "Request context unavailable").await; break; }
                            };
                            ctx.diagnostics.request_shape("ws:/responses", &body);
                            let guard = ActiveConnectionGuard::acquire(state.status.clone(), state.app_handle.clone(), &body, &headers).await;
                            protocol.started(body["model"].as_str().unwrap().to_owned());
                            active = Some(Active { ctx, _guard: guard, first_event_ms: None });
                        }
                        // Authorization/handshake can await while shutdown fires. Never
                        // start a billable request after observing that shutdown.
                        if *shutdown.borrow() || shutdown.has_changed().is_err() { break; }
                        // Exactly one send attempt. Never retry an ambiguous write.
                        if let Some(up) = &mut upstream {
                            if !upstream_send(up, UpMessage::Text(body.to_string())).await { break; }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => { if !client_send(&mut socket, Message::Pong(data)).await { break; } }
                    Some(Ok(Message::Pong(_))) => {},
                    Some(Ok(Message::Close(close))) => {
                        if let Some(up) = &mut upstream {
                            let close = close.map(|f| tokio_tungstenite::tungstenite::protocol::CloseFrame { code: f.code.into(), reason: f.reason });
                            upstream_send(up, UpMessage::Close(close)).await;
                        }
                        break;
                    }
                    Some(Ok(Message::Binary(_))) => { local_error(&mut socket, "Binary Responses events are unsupported").await; break; }
                    _ => break,
                }
            }
            message = async { match &mut upstream { Some(up) => up.next().await, None => std::future::pending().await } } => {
                last_upstream = Instant::now();
                match message {
                    Some(Ok(UpMessage::Text(text))) => {
                        last_activity = Instant::now();
                        if let Ok(event) = serde_json::from_str::<Value>(&text) {
                            if let Some(request) = &mut active {
                                request.first_event_ms.get_or_insert_with(|| request.ctx.latency_ms());
                            }
                            if let Some(response) = protocol.observe(&event) {
                                if let Some(request) = active.take() {
                                    let ctx = request.ctx;
                                    let kind = event["type"].as_str().unwrap_or_default();
                                    if kind == "response.failed" { ctx.diagnostics.failed_event(&event, None).await; }
                                    // Terminal ID is sufficient for a real zero-token response,
                                    // but connect-only and unknown terminal events never log.
                                    let mut usage = TokenUsage::from_codex_response_auto(response).unwrap_or_default();
                                    usage.message_id = response["id"].as_str().map(str::to_owned);
                                    log_usage(&state, &provider.id, "codex", ctx.outbound_model.as_deref().unwrap_or(&ctx.request_model),
                                        &ctx.request_model, usage, ctx.latency_ms(), request.first_event_ms, true,
                                        if kind == "response.failed" { 502 } else { 200 }, Some(ctx.session_id.clone()), ctx.reasoning_effort, ctx.client).await;
                                }
                            } else if event["type"] == "error" {
                                if let Some(request) = active.take() { request.ctx.diagnostics.failed_event(&event, None).await; }
                                // A generic error does not prove a running response was cancelled.
                                // Preserve it, then close rather than permit a misattributed create.
                                client_send(&mut socket, Message::Text(text)).await;
                                break;
                            }
                            if protocol.identity_limit_exceeded {
                                if protocol.active_model.is_none() { client_send(&mut socket, Message::Text(text.clone())).await; }
                                local_error(&mut socket, "Upstream response identity limit exceeded").await;
                                break;
                            }
                        }
                        // Preserve native events byte-for-byte; no SSE/Chat transformation.
                        if !client_send(&mut socket, Message::Text(text)).await { break; }
                    }
                    Some(Ok(UpMessage::Ping(data))) => { if let Some(up) = &mut upstream { if !upstream_send(up, UpMessage::Pong(data)).await { break; } } }
                    Some(Ok(UpMessage::Pong(_))) => {},
                    Some(Ok(UpMessage::Close(close))) => {
                        let close = close.map(|f| axum::extract::ws::CloseFrame { code: f.code.into(), reason: f.reason });
                        client_send(&mut socket, Message::Close(close)).await;
                        break;
                    }
                    _ => break,
                }
            }
        }
    }
    if let Some(request) = active {
        request.ctx.diagnostics.stream_failure().await;
    }
    if let Some(up) = &mut upstream {
        upstream_send(up, UpMessage::Close(None)).await;
    }
    client_send(&mut socket, Message::Close(None)).await;
}

#[cfg(test)]
#[path = "responses_websocket_tests.rs"]
mod tests;
