//! response handler module
//!
//! Unified handling of streaming and non-streaming API responses

use super::{
    content_encoding::{decompress_body_with_limit, get_content_encoding, DecompressError},
    diagnostics::RequestDiagnostics,
    forwarder::ActiveConnectionGuard,
    handler_config::{StreamUsageEventFilter, UsageParserConfig},
    handler_context::RequestContext,
    server::ProxyState,
    sse::{strip_sse_field, take_sse_block},
    types::ReasoningEffort,
    upstream_response::{ProxyResponse, MAX_RESPONSE_BODY_BYTES},
    usage::parser::TokenUsage,
    ProxyError,
};
use axum::http::{header::HeaderMap, HeaderName};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Mutex;

// ============================================================================
// Response header processing
// ============================================================================

/// Response headers defined in RFC 2616 / RFC 7230 that should not be forwarded by proxies.
const HOP_BY_HOP_RESPONSE_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

/// Remove the response-side hop-by-hop header, as well as the extension header named in `Connection`.
pub(crate) fn strip_hop_by_hop_response_headers(headers: &mut HeaderMap) {
    let connection_listed_headers: Vec<HeaderName> = headers
        .get_all(axum::http::header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .filter_map(|name| HeaderName::from_bytes(name.as_bytes()).ok())
        .collect();

    for name in HOP_BY_HOP_RESPONSE_HEADERS {
        headers.remove(*name);
    }

    for name in connection_listed_headers {
        headers.remove(name);
    }
}

/// Removed entity headers that would become distorted after rebuilding the response body.
pub(crate) fn strip_entity_headers_for_rebuilt_body(headers: &mut HeaderMap) {
    headers.remove(axum::http::header::CONTENT_ENCODING);
    headers.remove(axum::http::header::CONTENT_LENGTH);
    headers.remove(axum::http::header::TRANSFER_ENCODING);
}

/// Read the response body and decompress it if necessary, ensuring that the headers are consistent with the returned body.
///
/// Allow long Codex responses to finish without an additional local body deadline.
pub(crate) async fn read_decoded_body(
    response: ProxyResponse,
    tag: &str,
) -> Result<(HeaderMap, http::StatusCode, Bytes), ProxyError> {
    let mut headers = response.headers().clone();
    let status = response.status();
    let raw_bytes = response.bytes_with_limit(MAX_RESPONSE_BODY_BYTES).await?;

    log::debug!(
        "[{tag}] Received upstream response body: status={}, bytes={}, headers={}",
        status.as_u16(),
        raw_bytes.len(),
        format_headers(&headers)
    );

    let mut body_bytes = raw_bytes.clone();
    let mut decoded = false;

    if let Some(encoding) = get_content_encoding(&headers) {
        log::debug!("[{tag}] Decompress non-streaming response: content-encoding={encoding}");
        match decompress_body_with_limit(&encoding, &raw_bytes, MAX_RESPONSE_BODY_BYTES) {
            Ok(Some(decompressed)) => {
                // The decoder stops when the budget is exhausted, which must be ≤ MAX_RESPONSE_BODY_BYTES
                body_bytes = Bytes::from(decompressed);
                decoded = true;
            }
            // Unsupported encoding: pass through as is and retain the content-encoding header,
            // Let downstream diagnostics/clients know this is still compressed bytes
            Ok(None) => {}
            Err(DecompressError::TooLarge { .. }) => {
                return Err(ProxyError::ResponseBodyTooLarge(MAX_RESPONSE_BODY_BYTES));
            }
            Err(DecompressError::Io(_)) => {
                log::warn!("[{tag}] Upstream body decompression failed; forwarding original bytes");
            }
        }
    }

    if decoded {
        strip_entity_headers_for_rebuilt_body(&mut headers);
    }

    Ok((headers, status, body_bytes))
}

// ============================================================================
// public interface
// ============================================================================

/// Detect if the response is an SSE streaming response
#[inline]
pub fn is_sse_response(response: &ProxyResponse) -> bool {
    response.is_sse()
}

/// Handling streaming responses
pub async fn handle_streaming(
    response: ProxyResponse,
    ctx: &RequestContext,
    state: &ProxyState,
    parser_config: &UsageParserConfig,
    connection_guard: Option<ActiveConnectionGuard>,
) -> Response {
    let status = response.status();
    log::debug!(
        "[{}] Received upstream streaming response: status={}, headers={}",
        ctx.tag,
        status.as_u16(),
        format_headers(response.headers())
    );
    // Check if the streaming response is compressed (SSE usually does not, and SSE parsing will fail if compressed)
    if get_content_encoding(response.headers()).is_some() {
        log::warn!(
            "[{}] Upstream SSE is compressed; event parsing may fail (encoding omitted)",
            ctx.tag
        );
    }

    let mut response_headers = response.headers().clone();
    strip_hop_by_hop_response_headers(&mut response_headers);

    let mut builder = axum::response::Response::builder().status(status);

    // Copy response headers
    for (key, value) in &response_headers {
        builder = builder.header(key, value);
    }

    // Create byte stream
    let stream = response.bytes_stream();

    // Create a usage collector; do not parse each SSE event on the streaming hot path when usage logging is turned off.
    let usage_collector = create_usage_collector(ctx, state, status.as_u16(), parser_config);

    // Create a transparent flow with logs
    let logged_stream = create_logged_passthrough_stream_with_diagnostics(
        stream,
        ctx.tag,
        usage_collector,
        connection_guard,
        Some(ctx.diagnostics.clone()),
    );

    let body = axum::body::Body::from_stream(logged_stream);
    match builder.body(body) {
        Ok(resp) => resp,
        Err(e) => {
            log::error!("[{}] Failed to build streaming response: {e}", ctx.tag);
            ProxyError::Internal(format!("Failed to build streaming response: {e}")).into_response()
        }
    }
}

/// Handling non-streaming responses
pub async fn handle_non_streaming(
    response: ProxyResponse,
    ctx: &RequestContext,
    state: &ProxyState,
    parser_config: &UsageParserConfig,
    // guard is held within the function scope. After the entire package response is read, it will be dropped along with the function return.
    _connection_guard: Option<ActiveConnectionGuard>,
) -> Result<Response, ProxyError> {
    let (mut response_headers, status, body_bytes) = read_decoded_body(response, ctx.tag).await?;
    strip_hop_by_hop_response_headers(&mut response_headers);

    log::debug!(
        "[{}] Upstream response body received: bytes={} (content omitted)",
        ctx.tag,
        body_bytes.len()
    );

    // Usage recording is always on for the dashboard.
    if let Ok(json_value) = serde_json::from_slice::<Value>(&body_bytes) {
        // Analyze usage
        if let Some(usage) = (parser_config.response_parser)(&json_value) {
            // Attribution priority: usage parsed model → response model field → mapped outbound
            // Model(Route Takes True) → Client Requests Model. Empty strings are considered missing.
            let model = usage
                .model
                .clone()
                .filter(|m| !m.is_empty())
                .or_else(|| {
                    json_value
                        .get("model")
                        .and_then(|m| m.as_str())
                        .filter(|m| !m.is_empty())
                        .map(str::to_string)
                })
                .or_else(|| ctx.outbound_model.clone())
                .unwrap_or_else(|| ctx.request_model.clone());

            spawn_log_usage(
                state,
                ctx,
                usage,
                &model,
                &ctx.request_model,
                status.as_u16(),
                false,
            );
        } else {
            let model = json_value
                .get("model")
                .and_then(|m| m.as_str())
                .filter(|m| !m.is_empty())
                .map(str::to_string)
                .or_else(|| ctx.outbound_model.clone())
                .unwrap_or_else(|| ctx.request_model.clone());
            spawn_log_usage(
                state,
                ctx,
                TokenUsage::default(),
                &model,
                &ctx.request_model,
                status.as_u16(),
                false,
            );
            log::debug!(
                "[{}] Failed to parse usage information, skipping record",
                parser_config.app_type_str
            );
        }
    } else {
        log::debug!(
            "[{}] <<< Response (not JSON): {} bytes",
            ctx.tag,
            body_bytes.len()
        );
        spawn_log_usage(
            state,
            ctx,
            TokenUsage::default(),
            ctx.outbound_model.as_deref().unwrap_or(&ctx.request_model),
            &ctx.request_model,
            status.as_u16(),
            false,
        );
    }

    // Build response
    let mut builder = axum::response::Response::builder().status(status);
    for (key, value) in response_headers.iter() {
        builder = builder.header(key, value);
    }

    let body = axum::body::Body::from(body_bytes);
    builder.body(body).map_err(|e| {
        log::error!("[{}] Failed to build response: {e}", ctx.tag);
        ProxyError::Internal(format!("Failed to build response: {e}"))
    })
}

/// Universal response processing entry
///
/// Automatically select streaming or non-streaming based on response type
pub async fn process_response(
    response: ProxyResponse,
    ctx: &RequestContext,
    state: &ProxyState,
    parser_config: &UsageParserConfig,
    connection_guard: Option<ActiveConnectionGuard>,
) -> Result<Response, ProxyError> {
    if is_sse_response(&response) {
        Ok(handle_streaming(response, ctx, state, parser_config, connection_guard).await)
    } else {
        handle_non_streaming(response, ctx, state, parser_config, connection_guard).await
    }
}

// ============================================================================
// SSE usage collector
// ============================================================================

type UsageCallbackWithTiming = Arc<dyn Fn(Vec<Value>, Option<u64>) + Send + Sync + 'static>;

/// SSE usage collector
#[derive(Clone)]
pub struct SseUsageCollector {
    inner: Arc<SseUsageCollectorInner>,
}

struct SseUsageCollectorInner {
    events: Mutex<Vec<Value>>,
    first_event_time: Mutex<Option<std::time::Instant>>,
    first_event_set: AtomicBool,
    start_time: std::time::Instant,
    on_complete: UsageCallbackWithTiming,
    should_collect: Option<StreamUsageEventFilter>,
    finished: AtomicBool,
}

impl SseUsageCollector {
    /// Create a usage collector; `should_collect` is used to skip events unrelated to usage on the hot path.
    pub fn new(
        start_time: std::time::Instant,
        should_collect: Option<StreamUsageEventFilter>,
        callback: impl Fn(Vec<Value>, Option<u64>) + Send + Sync + 'static,
    ) -> Self {
        let on_complete: UsageCallbackWithTiming = Arc::new(callback);
        Self {
            inner: Arc::new(SseUsageCollectorInner {
                events: Mutex::new(Vec::new()),
                first_event_time: Mutex::new(None),
                first_event_set: AtomicBool::new(false),
                start_time,
                on_complete,
                should_collect,
                finished: AtomicBool::new(false),
            }),
        }
    }

    pub fn should_collect(&self, data: &str) -> bool {
        self.inner
            .should_collect
            .map(|filter| filter(data))
            .unwrap_or(true)
    }

    /// Marks the time of the first collected SSE event, following the existing approximate semantics of `first_token_ms`.
    async fn mark_first_collected_event_time(&self) {
        if self.inner.first_event_set.load(Ordering::Acquire) {
            return;
        }
        let mut first_time = self.inner.first_event_time.lock().await;
        if first_time.is_none() {
            *first_time = Some(std::time::Instant::now());
            self.inner.first_event_set.store(true, Ordering::Release);
        }
    }

    /// Push SSE events
    pub async fn push(&self, event: Value) {
        self.mark_first_collected_event_time().await;
        let mut events = self.inner.events.lock().await;
        events.push(event);
    }

    /// Complete collection and trigger callback
    pub async fn finish(&self) {
        if self.inner.finished.swap(true, Ordering::SeqCst) {
            return;
        }

        let events = {
            let mut guard = self.inner.events.lock().await;
            std::mem::take(&mut *guard)
        };

        let first_token_ms = {
            let first_time = self.inner.first_event_time.lock().await;
            first_time.map(|t| (t - self.inner.start_time).as_millis() as u64)
        };

        (self.inner.on_complete)(events, first_token_ms);
    }
}

struct SseUsageFinishGuard {
    collector: Option<SseUsageCollector>,
}

impl SseUsageFinishGuard {
    fn new(collector: SseUsageCollector) -> Self {
        Self {
            collector: Some(collector),
        }
    }

    fn disarm(&mut self) {
        self.collector = None;
    }
}

impl Drop for SseUsageFinishGuard {
    fn drop(&mut self) {
        if let Some(collector) = self.collector.take() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    collector.finish().await;
                });
            } else {
                log::warn!("Tokio runtime is unavailable when SSE usage finish protection is triggered, skipping asynchronous finish");
            }
        }
    }
}

// ============================================================================
// Internal helper function
// ============================================================================

/// Create a usage collector
pub(crate) fn create_usage_collector(
    ctx: &RequestContext,
    state: &ProxyState,
    status_code: u16,
    parser_config: &UsageParserConfig,
) -> Option<SseUsageCollector> {
    let state = state.clone();
    let provider_id = ctx.provider.id.clone();
    let request_model = ctx.request_model.clone();
    // Attribution guarantee when the model name is missing for streaming events: the mapped outbound model (route takes over the true value) takes precedence.
    // Secondly, the client requests the alias
    let fallback_model = ctx
        .outbound_model
        .clone()
        .unwrap_or_else(|| ctx.request_model.clone());
    // Keep usage attribution with the request context.
    let app_type_str = ctx.app_type_str;
    let tag = ctx.tag;
    let start_time = ctx.start_time;
    let stream_parser = parser_config.stream_parser;
    let model_extractor = parser_config.model_extractor;
    let session_id = ctx.session_id.clone();
    let reasoning_effort = ctx.reasoning_effort.clone();

    Some(SseUsageCollector::new(
        start_time,
        parser_config.stream_event_filter,
        move |events, first_token_ms| {
            if let Some(usage) = stream_parser(&events) {
                let model = model_extractor(&events, &fallback_model);
                let latency_ms = start_time.elapsed().as_millis() as u64;

                let state = state.clone();
                let provider_id = provider_id.clone();
                let session_id = session_id.clone();
                let request_model = request_model.clone();
                let reasoning_effort = reasoning_effort.clone();

                tokio::spawn(async move {
                    log_usage(
                        &state,
                        &provider_id,
                        app_type_str,
                        &model,
                        &request_model,
                        usage,
                        latency_ms,
                        first_token_ms,
                        true, // is_streaming
                        status_code,
                        Some(session_id),
                        reasoning_effort,
                    )
                    .await;
                });
            } else {
                let model = model_extractor(&events, &fallback_model);
                let latency_ms = start_time.elapsed().as_millis() as u64;
                let state = state.clone();
                let provider_id = provider_id.clone();
                let session_id = session_id.clone();
                let request_model = request_model.clone();
                let reasoning_effort = reasoning_effort.clone();

                tokio::spawn(async move {
                    log_usage(
                        &state,
                        &provider_id,
                        app_type_str,
                        &model,
                        &request_model,
                        TokenUsage::default(),
                        latency_ms,
                        first_token_ms,
                        true, // is_streaming
                        status_code,
                        Some(session_id),
                        reasoning_effort,
                    )
                    .await;
                });
                log::debug!("[{tag}] Streaming response lacks usage statistics and skips consumption records");
            }
        },
    ))
}

/// Asynchronous logging of usage
fn spawn_log_usage(
    state: &ProxyState,
    ctx: &RequestContext,
    usage: TokenUsage,
    model: &str,
    request_model: &str,
    status_code: u16,
    is_streaming: bool,
) {
    let state = state.clone();
    let provider_id = ctx.provider.id.clone();
    let app_type_str = ctx.app_type_str.to_string();
    let model = model.to_string();
    let request_model = request_model.to_string();
    let latency_ms = ctx.latency_ms();
    let session_id = ctx.session_id.clone();
    let reasoning_effort = ctx.reasoning_effort.clone();

    tokio::spawn(async move {
        log_usage(
            &state,
            &provider_id,
            &app_type_str,
            &model,
            &request_model,
            usage,
            latency_ms,
            None,
            is_streaming,
            status_code,
            Some(session_id),
            reasoning_effort,
        )
        .await;
    });
}

/// Record usage consistently for native Responses and the Chat bridge.
#[allow(clippy::too_many_arguments)]
pub(super) async fn log_usage(
    state: &ProxyState,
    provider_id: &str,
    app_type: &str,
    model: &str,
    request_model: &str,
    usage: TokenUsage,
    latency_ms: u64,
    first_token_ms: Option<u64>,
    is_streaming: bool,
    status_code: u16,
    session_id: Option<String>,
    reasoning_effort: ReasoningEffort,
) {
    use super::usage::logger::UsageLogger;

    let logger = UsageLogger::new(&state.db);
    let pricing_model = model;

    let request_id = usage.dedup_request_id(app_type, provider_id);

    log::debug!(
        "[{app_type}] Record request log: id={request_id}, provider={provider_id}, model={model}, streaming={is_streaming}, status={status_code}, latency_ms={latency_ms}, first_token_ms={first_token_ms:?}, session={}, input={}, output={}, cache_read={}, cache_creation={}",
        session_id.as_deref().unwrap_or("none"),
        usage.input_tokens,
        usage.output_tokens,
        usage.cache_read_tokens,
        usage.cache_creation_tokens
    );

    if let Err(e) = logger.log_with_calculation(
        request_id,
        provider_id.to_string(),
        app_type.to_string(),
        model.to_string(),
        request_model.to_string(),
        pricing_model.to_string(),
        usage,
        rust_decimal::Decimal::ONE,
        latency_ms,
        first_token_ms,
        status_code,
        session_id,
        None, // provider_type
        is_streaming,
        reasoning_effort,
    ) {
        log::warn!("[USG-001] Failed to record usage: {e}");
    }
}

/// Forward a stream with usage logging and no local idle deadline.
#[cfg(test)]
pub fn create_logged_passthrough_stream(
    stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    tag: &'static str,
    usage_collector: Option<SseUsageCollector>,
    connection_guard: Option<ActiveConnectionGuard>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    create_logged_passthrough_stream_with_diagnostics(
        stream,
        tag,
        usage_collector,
        connection_guard,
        None,
    )
}

pub(crate) fn create_logged_passthrough_stream_with_diagnostics(
    stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    tag: &'static str,
    usage_collector: Option<SseUsageCollector>,
    connection_guard: Option<ActiveConnectionGuard>,
    diagnostics: Option<Arc<RequestDiagnostics>>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let _conn_guard = connection_guard;
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut collector = usage_collector;
        let mut finish_guard = collector.clone().map(SseUsageFinishGuard::new);
        let inspect_sse_events =
            collector.is_some() || diagnostics.is_some() || log::log_enabled!(log::Level::Debug);
        let mut is_first_chunk = true;

        tokio::pin!(stream);

        loop {
            match stream.next().await {
                Some(Ok(bytes)) => {
                    if is_first_chunk {
                        log::debug!(
                            "[{tag}] Received the first packet of upstream streaming: bytes={}",
                            bytes.len()
                        );
                    }
                    is_first_chunk = false;
                    if inspect_sse_events {
                        crate::proxy::sse::append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

                        // Attempt to parse and log complete SSE events
                        while let Some(event_text) = take_sse_block(&mut buffer) {
                            if !event_text.trim().is_empty() {
                                // An SSE event joins all data fields with LF before JSON parsing.
                                // Parsing each line separately loses multiline terminal usage.
                                let mut data_fields = event_text
                                    .lines()
                                    .filter_map(|line| strip_sse_field(line, "data"));
                                let Some(first) = data_fields.next() else {
                                    continue;
                                };
                                // Single-line events stay borrowed on the usual hot path.
                                let mut data = std::borrow::Cow::Borrowed(first);
                                for field in data_fields {
                                    let joined = data.to_mut();
                                    joined.push('\n');
                                    joined.push_str(field);
                                }
                                if data.trim() != "[DONE]" {
                                    if let Some(diagnostics) = &diagnostics {
                                        let named_failure = event_text.lines().any(|line| {
                                            strip_sse_field(line, "event")
                                                .is_some_and(|event| event.trim() == "response.failed")
                                        });
                                        if data.len() <= 8192 {
                                            match serde_json::from_str::<Value>(&data) {
                                                Ok(event)
                                                    if named_failure
                                                        || event.get("type").and_then(Value::as_str)
                                                            == Some("response.failed") =>
                                                {
                                                    diagnostics.failed_event(&event, Some(data.as_ref()));
                                                }
                                                _ if named_failure => diagnostics.failed_event(
                                                    &serde_json::json!({"type":"response.failed"}),
                                                    Some(data.as_ref()),
                                                ),
                                                _ => {}
                                            }
                                        } else if named_failure {
                                            diagnostics.failed_event(
                                                &serde_json::json!({"type":"response.failed"}),
                                                Some(data.as_ref()),
                                            );
                                        }
                                    }
                                    let collected = match &collector {
                                        Some(c) if c.should_collect(&data) => {
                                            match serde_json::from_str::<Value>(&data) {
                                                Ok(json_value) => {
                                                    c.push(json_value).await;
                                                    true
                                                }
                                                Err(_) => false,
                                            }
                                        }
                                        _ => false,
                                    };
                                    log::trace!(
                                        "[{tag}] <<< SSE data: bytes={}, usage_collected={collected} (content omitted)",
                                        data.len()
                                    );
                                } else {
                                    log::debug!("[{tag}] <<< SSE: [DONE]");
                                }
                            }
                        }
                    }

                    yield Ok(bytes);
                }
                Some(Err(e)) => {
                    if let Some(diagnostics) = &diagnostics {
                        diagnostics.stream_failure();
                    } else {
                        log::error!("[{tag}] stream read failed (detail omitted)");
                    }
                    yield Err(std::io::Error::other(e.to_string()));
                    break;
                }
                None => {
                    // Stream ends normally
                    break;
                }
            }
        }

        if let Some(c) = collector.take() {
            c.finish().await;
        }
        if let Some(guard) = &mut finish_guard {
            guard.disarm();
        }
    }
}

fn is_safe_diagnostic_header(name: &str) -> bool {
    matches!(
        name,
        "content-type"
            | "content-encoding"
            | "content-length"
            | "retry-after"
            | "cf-ray"
            | "x-request-id"
            | "request-id"
            | "x-correlation-id"
    ) || name.starts_with("x-ratelimit-")
        || name.starts_with("ratelimit-")
}

fn bounded_header_value(value: &axum::http::HeaderValue) -> Option<String> {
    let value = value.to_str().ok()?;
    let mut bounded = value.chars().take(160).collect::<String>();
    if value.chars().count() > 160 {
        bounded.push('…');
    }
    Some(bounded)
}

fn format_headers(headers: &HeaderMap) -> String {
    let mut entries = headers
        .keys()
        .map(|key| {
            let name = key.as_str();
            if !is_safe_diagnostic_header(name) {
                return name.to_string();
            }

            let values = headers
                .get_all(key)
                .iter()
                .filter_map(bounded_header_value)
                .collect::<Vec<_>>();
            if values.is_empty() {
                name.to_string()
            } else {
                format!("{name}={}", values.join("|"))
            }
        })
        .collect::<Vec<_>>();
    entries.sort();
    format!("[{}]", entries.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::error::AppError;
    use crate::proxy::provider_router::ProviderRouter;
    use crate::proxy::providers::codex_chat_history::CodexChatHistoryStore;
    use crate::proxy::types::{ProxyConfig, ProxyStatus};
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::str::FromStr;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    #[tokio::test]
    async fn passthrough_usage_handles_multiline_and_incomplete_responses() {
        use crate::proxy::handler_config::codex_stream_usage_event_filter;
        use futures::TryStreamExt;
        use serde_json::json;

        for kind in [
            "response.completed",
            "response.incomplete",
            "response.failed",
        ] {
            let event = json!({
                "type": kind,
                "response": {
                    "id": "resp_usage_fixture",
                    "model": "gpt-6-astra",
                    "usage": {
                        "input_tokens": 125,
                        "output_tokens": 9,
                        "input_tokens_details": { "cached_tokens": 70 }
                    }
                }
            });
            for multiline in [false, true] {
                let json = if multiline {
                    serde_json::to_string_pretty(&event).unwrap()
                } else {
                    serde_json::to_string(&event).unwrap()
                };
                let data = json
                    .lines()
                    .map(|line| format!("data: {line}\r\n"))
                    .collect::<String>();
                let input =
                    format!(": heartbeat\r\n\r\nevent: {kind}\r\nid: transport-id\r\n{data}\r\n");
                for chunk_size in [1, 7, input.len()] {
                    let captured = Arc::new(std::sync::Mutex::new(None));
                    let capture = captured.clone();
                    let collector = SseUsageCollector::new(
                        std::time::Instant::now(),
                        Some(codex_stream_usage_event_filter),
                        move |events, _| *capture.lock().unwrap() = Some(events),
                    );
                    let chunks = input
                        .as_bytes()
                        .chunks(chunk_size)
                        .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
                        .collect::<Vec<_>>();
                    let output = create_logged_passthrough_stream(
                        futures::stream::iter(chunks),
                        "test",
                        Some(collector),
                        None,
                    )
                    .try_collect::<Vec<_>>()
                    .await
                    .unwrap();
                    assert_eq!(output.concat(), input.as_bytes());
                    let events = captured.lock().unwrap().take().unwrap();
                    assert_eq!(
                        events.as_slice(),
                        std::slice::from_ref(&event),
                        "{kind}, multiline={multiline}"
                    );
                    let usage = TokenUsage::from_codex_stream_events_auto(&events)
                        .expect("terminal response usage must be retained");
                    assert_eq!(usage.input_tokens, 125);
                    assert_eq!(usage.output_tokens, 9);
                    assert_eq!(usage.cache_read_tokens, 70);
                    assert_eq!(usage.message_id.as_deref(), Some("resp_usage_fixture"));
                    assert_eq!(usage.model.as_deref(), Some("gpt-6-astra"));
                }
            }
        }
    }

    #[test]
    fn format_headers_keeps_only_allowlisted_diagnostic_values() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer super-secret".parse().unwrap());
        headers.insert("set-cookie", "session=cookie-secret".parse().unwrap());
        headers.insert("retry-after", "30".parse().unwrap());
        headers.insert("x-ratelimit-remaining", "2".parse().unwrap());
        headers.insert("cf-ray", "abc123-SJC".parse().unwrap());

        let formatted = format_headers(&headers);
        assert!(formatted.contains("authorization"), "{formatted}");
        assert!(formatted.contains("set-cookie"), "{formatted}");
        assert!(formatted.contains("retry-after=30"), "{formatted}");
        assert!(formatted.contains("x-ratelimit-remaining=2"), "{formatted}");
        assert!(formatted.contains("cf-ray=abc123-SJC"), "{formatted}");
        assert!(!formatted.contains("super-secret"), "{formatted}");
        assert!(!formatted.contains("cookie-secret"), "{formatted}");
    }

    #[tokio::test]
    async fn read_decoded_body_rejects_compressed_bomb_without_full_expansion() {
        // The gzipped 128 MiB+1 all-zero payload is only ~130 KiB: the raw read cap doesn't stop it,
        // Only bounded decoding on the decompression side can be rejected. If decoding degenerates into "complete expansion first and then comparison",
        // Payloads with expanded length > MAX_RESPONSE_BODY_BYTES will be returned successfully (test fails).
        let payload = vec![0u8; MAX_RESPONSE_BODY_BYTES + 1];
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &payload).unwrap();
        let compressed = encoder.finish().unwrap();
        assert!(compressed.len() < MAX_RESPONSE_BODY_BYTES);

        let mut headers = HeaderMap::new();
        headers.insert("content-encoding", "gzip".parse().unwrap());
        let response =
            ProxyResponse::buffered(http::StatusCode::OK, headers, Bytes::from(compressed));

        let result = read_decoded_body(response, "test").await;
        assert!(
            matches!(result, Err(ProxyError::ResponseBodyTooLarge(_))),
            "Compression bombs should be rejected instead of full expansion: {:?}",
            result.map(|(_, _, body)| body.len())
        );
    }

    #[test]
    fn test_strip_sse_field_accepts_optional_space() {
        assert_eq!(
            super::strip_sse_field("data: {\"ok\":true}", "data"),
            Some("{\"ok\":true}")
        );
        assert_eq!(
            super::strip_sse_field("data:{\"ok\":true}", "data"),
            Some("{\"ok\":true}")
        );
        assert_eq!(
            super::strip_sse_field("event: message_start", "event"),
            Some("message_start")
        );
        assert_eq!(
            super::strip_sse_field("event:message_start", "event"),
            Some("message_start")
        );
        assert_eq!(super::strip_sse_field("id:1", "data"), None);
    }

    #[test]
    fn test_strip_hop_by_hop_response_headers_removes_standard_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::CONNECTION,
            axum::http::HeaderValue::from_static("keep-alive"),
        );
        headers.insert(
            axum::http::header::HeaderName::from_static("keep-alive"),
            axum::http::HeaderValue::from_static("timeout=5"),
        );
        headers.insert(
            axum::http::header::TRANSFER_ENCODING,
            axum::http::HeaderValue::from_static("chunked"),
        );
        headers.insert(
            axum::http::header::HeaderName::from_static("proxy-connection"),
            axum::http::HeaderValue::from_static("keep-alive"),
        );
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            axum::http::header::CONTENT_LENGTH,
            axum::http::HeaderValue::from_static("12"),
        );

        strip_hop_by_hop_response_headers(&mut headers);

        assert!(!headers.contains_key(axum::http::header::CONNECTION));
        assert!(!headers.contains_key("keep-alive"));
        assert!(!headers.contains_key(axum::http::header::TRANSFER_ENCODING));
        assert!(!headers.contains_key("proxy-connection"));
        assert_eq!(
            headers.get(axum::http::header::CONTENT_TYPE),
            Some(&axum::http::HeaderValue::from_static("application/json"))
        );
        assert_eq!(
            headers.get(axum::http::header::CONTENT_LENGTH),
            Some(&axum::http::HeaderValue::from_static("12"))
        );
    }

    #[test]
    fn test_strip_hop_by_hop_response_headers_removes_connection_listed_extensions() {
        let mut headers = HeaderMap::new();
        headers.append(
            axum::http::header::CONNECTION,
            axum::http::HeaderValue::from_static("x-trace-hop, x-debug-hop"),
        );
        headers.append(
            axum::http::header::CONNECTION,
            axum::http::HeaderValue::from_static("upgrade"),
        );
        headers.insert(
            axum::http::header::HeaderName::from_static("x-trace-hop"),
            axum::http::HeaderValue::from_static("trace"),
        );
        headers.insert(
            axum::http::header::HeaderName::from_static("x-debug-hop"),
            axum::http::HeaderValue::from_static("debug"),
        );
        headers.insert(
            axum::http::header::UPGRADE,
            axum::http::HeaderValue::from_static("websocket"),
        );
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/event-stream"),
        );

        strip_hop_by_hop_response_headers(&mut headers);

        assert!(!headers.contains_key(axum::http::header::CONNECTION));
        assert!(!headers.contains_key("x-trace-hop"));
        assert!(!headers.contains_key("x-debug-hop"));
        assert!(!headers.contains_key(axum::http::header::UPGRADE));
        assert_eq!(
            headers.get(axum::http::header::CONTENT_TYPE),
            Some(&axum::http::HeaderValue::from_static("text/event-stream"))
        );
    }

    fn build_state(db: Arc<Database>) -> ProxyState {
        ProxyState {
            db: db.clone(),
            config: Arc::new(RwLock::new(ProxyConfig::default())),
            status: Arc::new(RwLock::new(ProxyStatus::default())),
            start_time: Arc::new(RwLock::new(None)),
            current_providers: Arc::new(RwLock::new(HashMap::new())),
            provider_router: Arc::new(ProviderRouter::new(db.clone())),
            codex_chat_history: Arc::new(CodexChatHistoryStore::default()),
            app_handle: None,
        }
    }

    fn seed_pricing(db: &Database) -> Result<(), AppError> {
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT OR REPLACE INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["gpt-response-fixture", "Resp Model", "1.0", "0"],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["gpt-request-fixture", "Req Model", "2.0", "0"],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn insert_provider(
        db: &Database,
        id: &str,
        app_type: &str,
        meta: serde_json::Value,
    ) -> Result<(), AppError> {
        let meta_json =
            serde_json::to_string(&meta).map_err(|e| AppError::Database(e.to_string()))?;
        let conn = crate::database::lock_conn!(db.conn);
        conn.execute(
            "INSERT INTO providers (id, app_type, name, settings_config, meta)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, app_type, "Test Provider", "{}", meta_json],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    #[tokio::test]
    async fn test_log_usage_uses_response_model_and_preserves_legacy_metadata(
    ) -> Result<(), AppError> {
        let db = Arc::new(Database::memory()?);
        let app_type = "codex";
        seed_pricing(&db)?;
        {
            let conn = crate::database::lock_conn!(db.conn);
            conn.execute(
                "INSERT OR REPLACE INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
                 VALUES ('gpt-outbound-fixture', 'Outbound Model', '4.0', '0')",
                [],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
            conn.execute(
                "UPDATE proxy_config
                 SET default_cost_multiplier = '9', pricing_model_source = 'request'
                 WHERE app_type = 'codex'",
                [],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        }
        insert_provider(
            &db,
            "provider-current",
            app_type,
            serde_json::json!({
                "providerType": "github_copilot",
                "costMultiplier": "9",
                "pricingModelSource": "request"
            }),
        )?;
        let state = build_state(db.clone());
        log_usage(
            &state,
            "provider-current",
            app_type,
            "gpt-response-fixture",
            "gpt-request-fixture",
            TokenUsage {
                input_tokens: 1_000_000,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                model: None,
                message_id: None,
            },
            10,
            None,
            false,
            200,
            None,
            ReasoningEffort::default(),
        )
        .await;

        let conn = crate::database::lock_conn!(db.conn);
        let (model, request_model, pricing_model, total_cost, cost_multiplier): (
            String,
            String,
            String,
            String,
            String,
        ) = conn
            .query_row(
                "SELECT model, request_model, pricing_model, total_cost_usd, cost_multiplier
                 FROM proxy_request_logs WHERE provider_id = 'provider-current'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        assert_eq!(model, "gpt-response-fixture");
        assert_eq!(request_model, "gpt-request-fixture");
        assert_eq!(pricing_model, "gpt-response-fixture");
        assert_eq!(Decimal::from_str(&cost_multiplier).unwrap(), Decimal::ONE);
        assert_eq!(Decimal::from_str(&total_cost).unwrap(), Decimal::ONE);
        let raw_meta: String = conn
            .query_row(
                "SELECT meta FROM providers WHERE id = 'provider-current'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        let meta: serde_json::Value = serde_json::from_str(&raw_meta).unwrap();
        assert_eq!(meta["costMultiplier"], "9");
        assert_eq!(meta["pricingModelSource"], "request");
        let stored: (String, String) = conn
            .query_row(
                "SELECT default_cost_multiplier, pricing_model_source
                 FROM proxy_config WHERE app_type = 'codex'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        assert_eq!(stored, ("9".into(), "request".into()));
        Ok(())
    }
}
