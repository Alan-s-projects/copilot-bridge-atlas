//! request handler
//!
//! Handle HTTP requests for various API endpoints
//!
//! Reconstructed structure:
//! - Common logic extracted to `handler_context` and `response_processor` modules
//! - Each handler only retains unique business logic

use super::{
    content_encoding::{
        decompress_body_with_limit, get_content_encoding, is_supported_content_encoding,
        DecompressError,
    },
    error_mapper::{get_error_message, map_proxy_error_to_status},
    forwarder::ActiveConnectionGuard,
    handler_config::{codex_stream_usage_event_filter, CODEX_PARSER_CONFIG},
    handler_context::RequestContext,
    providers::{
        codex_chat_common::extract_reasoning_field_text,
        codex_chat_history::record_responses_sse_stream,
        streaming_codex_chat::create_responses_sse_stream_from_chat_with_context,
        streaming_copilot_responses, transform_codex_chat,
    },
    response_processor::{
        create_logged_passthrough_stream_with_diagnostics, process_response, read_decoded_body,
        strip_entity_headers_for_rebuilt_body, strip_hop_by_hop_response_headers,
        SseUsageCollector,
    },
    server::ProxyState,
    sse::{strip_sse_field, take_sse_block},
    types::*,
    usage::parser::TokenUsage,
    ProxyError,
};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use bytes::Bytes;
use http_body_util::{BodyExt, LengthLimitError, Limited};
use serde_json::{json, Value};

const MAX_CODEX_REQUEST_BODY_BYTES: usize = 200 * 1024 * 1024;

// ============================================================================
// Health checks and status queries (simple endpoint)
// ============================================================================

/// health check
pub async fn health_check() -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(json!({
            "status": "healthy",
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })),
    )
}

/// Get service status
pub async fn get_status(State(state): State<ProxyState>) -> Result<Json<ProxyStatus>, ProxyError> {
    let mut status = state.status.read().await.clone();
    status.active_requests.truncate(5);
    Ok(Json(status))
}

/// Codex model discovery uses Atlas's own catalog, independently of client TOML.
pub async fn handle_models() -> Result<Json<Value>, ProxyError> {
    let app_dir = crate::config::get_app_config_dir();
    let path = crate::copilot_bridge::catalog_path();
    if !path.exists() || !crate::config::path_is_within(&app_dir, &path) {
        return Ok(Json(json!({ "models": [] })));
    }
    let catalog = crate::codex_config::read_codex_model_catalog_text(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|catalog| catalog.get("models").is_some_and(Value::is_array))
        .map(|mut catalog| {
            if let Some(models) = catalog.get_mut("models").and_then(Value::as_array_mut) {
                models.retain(|model| {
                    model
                        .get("slug")
                        .or_else(|| model.get("id"))
                        .or_else(|| model.get("model"))
                        .and_then(Value::as_str)
                        .is_some_and(super::providers::copilot_model_map::is_valid_model_id)
                });
            }
            catalog
        })
        .unwrap_or_else(|| json!({ "models": [] }));
    Ok(Json(catalog))
}

fn endpoint_with_query(uri: &axum::http::Uri, endpoint: &str) -> String {
    match uri.query() {
        Some(query) => format!("{endpoint}?{query}"),
        None => endpoint.to_string(),
    }
}

/// The Codex client (especially the Desktop login state) may enable zstd compression for the request body, causing subsequent
/// `serde_json::from_slice` direct parsing failed. Here, decompress before parsing and peel off the distorted entity header.
/// (content-encoding / content-length / transfer-encoding) - the forwarding layer will be based on the decompressed
/// Clear text JSON regenerates correct headers.
fn decode_codex_request_body(
    headers: &mut axum::http::HeaderMap,
    body_bytes: Bytes,
    max_bytes: usize,
) -> Result<Bytes, ProxyError> {
    let Some(encoding) = get_content_encoding(headers) else {
        return Ok(body_bytes);
    };

    if !is_supported_content_encoding(&encoding) {
        return Err(ProxyError::InvalidRequest(format!(
            "Unsupported request content-encoding: {encoding}"
        )));
    }

    log::debug!("[Codex] Decompress the request body: content-encoding={encoding}");
    let decompressed = match decompress_body_with_limit(&encoding, &body_bytes, max_bytes) {
        Ok(Some(decompressed)) => decompressed,
        // is_supported_content_encoding has ensured that the encoding is supported and will not return None normally;
        // Defensive caveat: It is better to report an error than to transparently pass the compressed bytes as JSON.
        Ok(None) => {
            return Err(ProxyError::InvalidRequest(format!(
                "Unsupported request content-encoding: {encoding}"
            )));
        }
        Err(DecompressError::TooLarge { .. }) => {
            return Err(ProxyError::RequestBodyTooLarge(max_bytes));
        }
        Err(DecompressError::Io(e)) => {
            log::warn!("[Codex] Request body decompression failed (detail omitted)");
            return Err(ProxyError::InvalidRequest(format!(
                "Failed to decompress request body ({encoding}): {e}"
            )));
        }
    };

    headers.remove(axum::http::header::CONTENT_ENCODING);
    headers.remove(axum::http::header::CONTENT_LENGTH);
    headers.remove(axum::http::header::TRANSFER_ENCODING);

    Ok(Bytes::from(decompressed))
}

/// Limit both wire bytes and decoded bytes, including chunked and compressed
/// requests. Axum's DefaultBodyLimit does not cover direct Request body reads.
async fn read_codex_request_body(
    headers: &mut axum::http::HeaderMap,
    body: axum::body::Body,
    max_bytes: usize,
) -> Result<Value, ProxyError> {
    let body_bytes = Limited::new(body, max_bytes)
        .collect()
        .await
        .map_err(|error| {
            if error.is::<LengthLimitError>() {
                ProxyError::RequestBodyTooLarge(max_bytes)
            } else {
                ProxyError::InvalidRequest(format!("Failed to read request body: {error}"))
            }
        })?
        .to_bytes();
    let body_bytes = decode_codex_request_body(headers, body_bytes, max_bytes)?;
    serde_json::from_slice(&body_bytes).map_err(|error| {
        ProxyError::InvalidRequest(format!("Failed to parse request body: {error}"))
    })
}

pub async fn handle_responses(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    let (parts, req_body) = request.into_parts();
    let method = parts.method.clone();
    let uri = parts.uri;
    let mut headers = parts.headers;
    let body =
        read_codex_request_body(&mut headers, req_body, MAX_CODEX_REQUEST_BODY_BYTES).await?;

    let mut ctx = RequestContext::new(&state, &body, &headers).await?;
    let endpoint = endpoint_with_query(&uri, "/responses");

    let is_stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut codex_tool_context = transform_codex_chat::build_codex_tool_context_from_request(&body);

    let forwarder = ctx.create_forwarder(&state);
    let mut result = match forwarder
        .forward_request(
            method,
            &endpoint,
            body,
            headers,
            &ctx.provider,
            &mut ctx.reasoning_effort,
        )
        .await
    {
        Ok(result) => result,
        Err(err) => {
            log_forward_error(&ctx, &err);
            return build_codex_proxy_error_response(&ctx, &endpoint, &err);
        }
    };

    let connection_guard = result.connection_guard.take();
    let codex_upstream_format = result.codex_upstream_format;
    ctx.outbound_model = result.outbound_model.take();
    codex_tool_context.enable_for_outbound_model(ctx.outbound_model.as_deref());
    let response = result.response;
    let response = if codex_upstream_format
        == Some(super::forwarder::CodexUpstreamFormat::CompatibleResponses)
    {
        super::providers::transform_copilot_responses::adapt_response(
            response,
            codex_tool_context.clone(),
        )
        .await
        .map_err(|error| {
            ctx.diagnostics.proxy_failure("adapt_response", &error);
            error
        })?
    } else {
        response
    };

    if codex_response_transform(codex_upstream_format) == CodexResponseTransform::ChatCompletions {
        return handle_codex_chat_to_responses_transform(
            response,
            &ctx,
            &state,
            is_stream,
            connection_guard,
            codex_tool_context,
        )
        .await
        .map_err(|error| {
            ctx.diagnostics.proxy_failure("chat_conversion", &error);
            error
        });
    }

    let response = if ctx.provider.is_github_copilot()
        && matches!(
            codex_upstream_format,
            Some(
                super::forwarder::CodexUpstreamFormat::NativeResponses
                    | super::forwarder::CodexUpstreamFormat::CompatibleResponses
            )
        ) {
        streaming_copilot_responses::normalize_response(response)
    } else {
        response
    };

    // Keep item identity stable across all Copilot Responses stream events.
    // The integer rewrite also applies to requests without namespace tools.

    process_response(
        response,
        &ctx,
        &state,
        &CODEX_PARSER_CONFIG,
        connection_guard,
    )
    .await
    .map_err(|error| {
        ctx.diagnostics.proxy_failure("response_processing", &error);
        error
    })
}

/// Handle Codex's standalone Alpha Search protocol as a semantic passthrough.
///
/// Recent Codex clients send web-search commands to a dedicated endpoint instead
/// of embedding them in a Responses request. Keep this path out of the
/// Responses-to-Chat bridge: those formats cannot represent the Alpha
/// Search protocol.
pub async fn handle_alpha_search(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    handle_codex_standalone_passthrough(state, request, "/alpha/search").await
}

/// Handle Codex's legacy Images API endpoint for built-in ImageGen.
pub async fn handle_images_generations(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    handle_codex_standalone_passthrough(state, request, "/images/generations").await
}

/// Handle Codex's legacy Images API edit endpoint for built-in ImageGen.
///
/// Codex switches from `/images/generations` to `/images/edits` whenever the
/// ImageGen tool references existing images (explicit file paths or the last N
/// generated images). The body is plain JSON with data-URL images, so it takes
/// the same standalone passthrough as generations; only the upstream path differs.
pub async fn handle_images_edits(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    handle_codex_standalone_passthrough(state, request, "/images/edits").await
}

async fn handle_codex_standalone_passthrough(
    state: ProxyState,
    request: axum::extract::Request,
    canonical_endpoint: &'static str,
) -> Result<axum::response::Response, ProxyError> {
    let (parts, req_body) = request.into_parts();
    let method = parts.method.clone();
    let uri = parts.uri;
    let mut headers = parts.headers;
    let body =
        read_codex_request_body(&mut headers, req_body, MAX_CODEX_REQUEST_BODY_BYTES).await?;

    let mut ctx = RequestContext::new(&state, &body, &headers).await?;
    let endpoint = endpoint_with_query(&uri, canonical_endpoint);

    let forwarder = ctx.create_forwarder(&state);
    let mut result = match forwarder
        .forward_request(
            method,
            &endpoint,
            body,
            headers,
            &ctx.provider,
            &mut ctx.reasoning_effort,
        )
        .await
    {
        Ok(result) => result,
        Err(err) => {
            log_forward_error(&ctx, &err);
            return build_codex_proxy_error_response(&ctx, &endpoint, &err);
        }
    };

    let connection_guard = result.connection_guard.take();
    ctx.outbound_model = result.outbound_model.take();

    process_response(
        result.response,
        &ctx,
        &state,
        &CODEX_PARSER_CONFIG,
        connection_guard,
    )
    .await
    .map_err(|error| {
        ctx.diagnostics.proxy_failure("response_processing", &error);
        error
    })
}

pub async fn handle_responses_compact(
    State(state): State<ProxyState>,
    request: axum::extract::Request,
) -> Result<axum::response::Response, ProxyError> {
    let (parts, req_body) = request.into_parts();
    let method = parts.method.clone();
    let uri = parts.uri;
    let mut headers = parts.headers;
    let body =
        read_codex_request_body(&mut headers, req_body, MAX_CODEX_REQUEST_BODY_BYTES).await?;

    let mut ctx = RequestContext::new(&state, &body, &headers).await?;
    let endpoint = endpoint_with_query(&uri, "/responses/compact");

    let is_stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut codex_tool_context = transform_codex_chat::build_codex_tool_context_from_request(&body);

    let forwarder = ctx.create_forwarder(&state);
    let mut result = match forwarder
        .forward_request(
            method,
            &endpoint,
            body,
            headers,
            &ctx.provider,
            &mut ctx.reasoning_effort,
        )
        .await
    {
        Ok(result) => result,
        Err(err) => {
            log_forward_error(&ctx, &err);
            return build_codex_proxy_error_response(&ctx, &endpoint, &err);
        }
    };

    let connection_guard = result.connection_guard.take();
    let codex_upstream_format = result.codex_upstream_format;
    ctx.outbound_model = result.outbound_model.take();
    codex_tool_context.enable_for_outbound_model(ctx.outbound_model.as_deref());
    let response = result.response;

    if codex_response_transform(codex_upstream_format) == CodexResponseTransform::ChatCompletions {
        return handle_codex_chat_to_responses_transform(
            response,
            &ctx,
            &state,
            is_stream,
            connection_guard,
            codex_tool_context,
        )
        .await
        .map_err(|error| {
            ctx.diagnostics.proxy_failure("chat_conversion", &error);
            error
        });
    }

    process_response(
        response,
        &ctx,
        &state,
        &CODEX_PARSER_CONFIG,
        connection_guard,
    )
    .await
    .map_err(|error| {
        ctx.diagnostics.proxy_failure("response_processing", &error);
        error
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexResponseTransform {
    Passthrough,
    ChatCompletions,
}

fn codex_response_transform(
    upstream_format: Option<super::forwarder::CodexUpstreamFormat>,
) -> CodexResponseTransform {
    match upstream_format {
        Some(super::forwarder::CodexUpstreamFormat::ChatCompletions) => {
            CodexResponseTransform::ChatCompletions
        }
        Some(
            super::forwarder::CodexUpstreamFormat::NativeResponses
            | super::forwarder::CodexUpstreamFormat::CompatibleResponses,
        )
        | None => CodexResponseTransform::Passthrough,
    }
}

async fn handle_codex_chat_to_responses_transform(
    response: super::upstream_response::ProxyResponse,
    ctx: &RequestContext,
    state: &ProxyState,
    is_stream: bool,
    connection_guard: Option<ActiveConnectionGuard>,
    tool_context: transform_codex_chat::CodexToolContext,
) -> Result<axum::response::Response, ProxyError> {
    let status = response.status();

    if !status.is_success() {
        // The shape of the upstream Chat error body is inconsistent with Responses (such as MiniMax's base_resp, custom detail field);
        // Direct transparent transmission will make the Codex client unable to recognize the error code. This is uniformly converted to Responses style.
        // `{"error": {message, type, code, param}}`, retain the original HTTP status code.
        return handle_codex_chat_error_response(response, ctx, status).await;
    }

    if is_stream || response.is_sse() {
        let stream = response.bytes_stream();
        let sse_stream = create_responses_sse_stream_from_chat_with_context(stream, tool_context);
        let sse_stream = record_responses_sse_stream(sse_stream, state.codex_chat_history.clone());

        let usage_collector = {
            let state = state.clone();
            let provider_id = ctx.provider.id.clone();
            let request_model = ctx.request_model.clone();
            // Attribution backlog for takeover/model override scenarios: outbound truth values ​​take precedence over client request aliases
            let fallback_model = ctx
                .outbound_model
                .clone()
                .unwrap_or_else(|| ctx.request_model.clone());
            let app_type_str = ctx.app_type_str;
            let start_time = ctx.start_time;
            let session_id = ctx.session_id.clone();
            let reasoning_effort = ctx.reasoning_effort.clone();

            Some(SseUsageCollector::new(
                start_time,
                Some(codex_stream_usage_event_filter),
                move |events, first_token_ms| {
                    let usage =
                        TokenUsage::from_codex_stream_events_auto(&events).unwrap_or_default();
                    // When upstream adheres to OpenAI semantics and omits usage, the Chat→Responses converter will synthesize a
                    // All 0's response.completed, from_codex_response to input/output field
                    // If exists (even if =0), Some is returned. Missing the nonzero gate will cause all 0 usage to be written as well:
                    // message_id=None → dedup_request_id degenerates into a random UUID, unable to deduplicate, each transaction
                    // Skip an empty usage row instead of inflating request counts.
                    if !usage.has_billable_tokens() {
                        log::debug!("[Codex] Streaming response usage is all 0 or missing, skip consumption record");
                        return;
                    }
                    let model = usage
                        .model
                        .clone()
                        .filter(|m| !m.is_empty())
                        .unwrap_or_else(|| fallback_model.clone());
                    let latency_ms = start_time.elapsed().as_millis() as u64;

                    let state = state.clone();
                    let provider_id = provider_id.clone();
                    let request_model = request_model.clone();
                    let outbound_model = fallback_model.clone();
                    let session_id = session_id.clone();
                    let reasoning_effort = reasoning_effort.clone();

                    tokio::spawn(async move {
                        log_usage(
                            &state,
                            &provider_id,
                            app_type_str,
                            &model,
                            &request_model,
                            &outbound_model,
                            usage,
                            latency_ms,
                            first_token_ms,
                            true,
                            status.as_u16(),
                            Some(session_id),
                            reasoning_effort,
                        )
                        .await;
                    });
                },
            ))
        };

        let logged_stream = create_logged_passthrough_stream_with_diagnostics(
            sse_stream,
            ctx.tag,
            usage_collector,
            connection_guard,
            Some(ctx.diagnostics.clone()),
        );

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "Content-Type",
            axum::http::HeaderValue::from_static("text/event-stream"),
        );
        headers.insert(
            "Cache-Control",
            axum::http::HeaderValue::from_static("no-cache"),
        );

        let body = axum::body::Body::from_stream(logged_stream);
        return Ok((headers, body).into_response());
    }

    let _connection_guard = connection_guard;
    let (mut response_headers, status, body_bytes) = read_decoded_body(response, ctx.tag).await?;
    let body_str = String::from_utf8_lossy(&body_bytes);
    let chat_response: Value = match serde_json::from_slice(&body_bytes) {
        Ok(value) => value,
        // Some gateways return SSE without the matching Content-Type header:
        // Aggregate by SSE when upstream returns SSE body with untagged Content-Type on stream:false .
        Err(_) if body_looks_like_sse(&body_str) => {
            log::warn!("[Codex] Upstream returns untagged SSE body for non-streaming requests, and uses Chat SSE aggregation to find out");
            // When aggregation also fails: the server log only records the length, and provides on-site diagnosis for client errors (C7)
            chat_sse_to_response_value(&body_str).map_err(|e| {
                log::error!(
                    "[Codex] SSE aggregation failed: atlas_id={}, body_bytes={}",
                    ctx.diagnostics.id,
                    body_bytes.len()
                );
                aggregate_fallback_error(e, &response_headers, &body_str)
            })?
        }
        Err(e) => {
            log::error!(
                "[Codex] Chat response parsing failed: atlas_id={}, body_bytes={}",
                ctx.diagnostics.id,
                body_bytes.len()
            );
            return Err(upstream_body_parse_error(
                "Failed to parse upstream chat response",
                &e,
                &response_headers,
                &body_str,
            ));
        }
    };
    let responses_response = transform_codex_chat::chat_completion_to_response_with_context(
        chat_response,
        &tool_context,
    )
    .map_err(|e| {
        log::error!(
            "[Codex] Chat response conversion failed: atlas_id={}",
            ctx.diagnostics.id
        );
        e
    })?;
    state
        .codex_chat_history
        .record_response(&responses_response)
        .await;

    // When upstream non-streaming Chat omits usage, chat_usage_to_responses_usage will be synthesized into all 0 usage
    // (transform_codex_chat.rs:1581), from_codex_response exists for the input/output field (even if =0)
    // That is, returns Some. Use the has_billable_tokens gate to skip all 0s and avoid empty lines from inflating the number of requests - and streaming branches
    // Keep the same empty-usage rule as the streaming path.
    if let Some(usage) = TokenUsage::from_codex_response_auto(&responses_response)
        .filter(TokenUsage::has_billable_tokens)
    {
        let model = responses_response
            .get("model")
            .and_then(|m| m.as_str())
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .or_else(|| ctx.outbound_model.clone())
            .unwrap_or_else(|| ctx.request_model.clone());
        let request_model = ctx.request_model.clone();
        let outbound_model = ctx
            .outbound_model
            .clone()
            .unwrap_or_else(|| ctx.request_model.clone());
        let app_type_str = ctx.app_type_str;
        tokio::spawn({
            let state = state.clone();
            let provider_id = ctx.provider.id.clone();
            let session_id = ctx.session_id.clone();
            let latency_ms = ctx.latency_ms();
            let reasoning_effort = ctx.reasoning_effort.clone();
            async move {
                log_usage(
                    &state,
                    &provider_id,
                    app_type_str,
                    &model,
                    &request_model,
                    &outbound_model,
                    usage,
                    latency_ms,
                    None,
                    false,
                    status.as_u16(),
                    Some(session_id),
                    reasoning_effort,
                )
                .await;
            }
        });
    }

    strip_entity_headers_for_rebuilt_body(&mut response_headers);
    strip_hop_by_hop_response_headers(&mut response_headers);
    // Builder::header has append semantics; if it is not removed first, it will be sent to the upstream Content-Type.
    response_headers.remove(axum::http::header::CONTENT_TYPE);

    let mut builder = axum::response::Response::builder().status(status);
    for (key, value) in response_headers.iter() {
        builder = builder.header(key, value);
    }
    builder = builder.header(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );

    let response_body = serde_json::to_vec(&responses_response).map_err(|e| {
        log::error!("[Codex] Serialization Responses failed: {e}");
        ProxyError::TransformError(format!("Failed to serialize responses response: {e}"))
    })?;

    builder
        .body(axum::body::Body::from(response_body))
        .map_err(|e| {
            log::error!("[Codex] Failed to build Responses: {e}");
            ProxyError::Internal(format!("Failed to build response: {e}"))
        })
}

/// Convert error responses from upstream Chat Completions into Responses API error shapes.
///
/// Matched with the normal response branch: normal responses have been rewritten into Responses form, and error responses are still retained
/// Chat error body (such as MiniMax's `{"base_resp": {"status_code": 2013}}`), Codex
/// Client-side error handling will not be able to align the fields. Here the upstream body is read and regularized into
/// `{"error": {message, type, code, param}}` and retain the original HTTP status code.
async fn handle_codex_chat_error_response(
    response: super::upstream_response::ProxyResponse,
    ctx: &RequestContext,
    status: axum::http::StatusCode,
) -> Result<axum::response::Response, ProxyError> {
    let (mut response_headers, _status, body_bytes) = read_decoded_body(response, ctx.tag).await?;
    ctx.diagnostics.error_body(&body_bytes, true);
    ctx.diagnostics.proxy_failure(
        "chat_error_response",
        &ProxyError::UpstreamError {
            status: status.as_u16(),
            body: None,
        },
    );

    // If non-JSON upstream error bodies (Cloudflare HTML, plain text "Unauthorized", etc.) are thrown as None,
    // The client cannot see the original diagnostic information; wrap it into Value::String and take the string branch of the conversion function.
    let parsed_value: Value = match serde_json::from_slice::<Value>(&body_bytes) {
        Ok(value) => value,
        Err(_) => {
            const MAX_RAW_ERROR_BYTES: usize = 1024;
            let lossy = String::from_utf8_lossy(&body_bytes);
            let truncated = if lossy.len() > MAX_RAW_ERROR_BYTES {
                let mut end = MAX_RAW_ERROR_BYTES;
                while end > 0 && !lossy.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}…(truncated)", &lossy[..end])
            } else {
                lossy.into_owned()
            };
            log::warn!(
                "[Codex] Chat error response is not legal JSON, transparent transmission as text: body_bytes={} (content omitted)",
                body_bytes.len()
            );
            Value::String(truncated)
        }
    };

    let responses_error = transform_codex_chat::chat_error_to_response_error(Some(&parsed_value));

    strip_entity_headers_for_rebuilt_body(&mut response_headers);
    strip_hop_by_hop_response_headers(&mut response_headers);
    // Builder::header has append semantics; if it is not removed first, it will be sent to the upstream Content-Type.
    response_headers.remove(axum::http::header::CONTENT_TYPE);

    let mut builder = axum::response::Response::builder().status(status);
    for (key, value) in response_headers.iter() {
        builder = builder.header(key, value);
    }
    builder = builder.header(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );

    let body = serde_json::to_vec(&responses_error).map_err(|e| {
        log::error!("[Codex] Failed to serialize Responses error body: {e}");
        ProxyError::TransformError(format!("Failed to serialize responses error: {e}"))
    })?;

    builder.body(axum::body::Body::from(body)).map_err(|e| {
        log::error!("[Codex] Failed to build Responses error response: {e}");
        ProxyError::Internal(format!("Failed to build response: {e}"))
    })
}

/// Construct failures at the forwarding layer (non-upstream responses) into enriched Codex error responses.
///
/// Different from `handle_codex_chat_error_response` (handling upstream real error response, copying upstream header),
/// There is no upstream response to refer to here, only a `application/json` error body is generated. Status code goes
/// `map_proxy_error_to_status`, this function is aligned with `ProxyError::into_response`.
///
/// Note: `endpoint` via `endpoint_with_query` may carry query (such as `?beta=true`) and be
/// Write the error body as is. The current Codex endpoint does not put credentials in the query, so it is safe; if it is reused in the future
/// Strip query credentials before displaying an endpoint.
fn build_codex_proxy_error_response(
    ctx: &RequestContext,
    endpoint: &str,
    error: &ProxyError,
) -> Result<axum::response::Response, ProxyError> {
    let status = axum::http::StatusCode::from_u16(map_proxy_error_to_status(error))
        .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    let body = codex_proxy_error_json(&ctx.provider.name, &ctx.request_model, endpoint, error);
    let body = serde_json::to_vec(&body).map_err(|e| {
        log::error!("[Codex] Serialization proxy error body failed: {e}");
        ProxyError::Internal(format!("Failed to serialize proxy error: {e}"))
    })?;

    axum::response::Response::builder()
        .status(status)
        .header(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        )
        .body(axum::body::Body::from(body))
        .map_err(|e| {
            log::error!("[Codex] Failed to build proxy error response: {e}");
            ProxyError::Internal(format!("Failed to build proxy error response: {e}"))
        })
}

fn codex_proxy_error_json(
    provider_name: &str,
    request_model: &str,
    endpoint: &str,
    error: &ProxyError,
) -> Value {
    let (mut body, upstream_status) = match error {
        ProxyError::UpstreamError { status, body } => {
            let parsed_body = body
                .as_deref()
                .map(|body| serde_json::from_str::<Value>(body).unwrap_or_else(|_| json!(body)));
            (
                transform_codex_chat::chat_error_to_response_error(parsed_body.as_ref()),
                Some(*status),
            )
        }
        _ => (
            json!({
                "error": {
                    "message": get_error_message(error),
                    "type": "proxy_error",
                    "code": codex_proxy_error_code(error),
                    "param": Value::Null,
                }
            }),
            None,
        ),
    };

    let Some(error_obj) = body
        .get_mut("error")
        .and_then(|value| value.as_object_mut())
    else {
        return body;
    };

    let message = if upstream_status == Some(413) {
        // HTTP 413 comes from the upstream gateway, whose body limit can be lower
        // than the local 200 MB request limit. The upstream response is often
        // A whole section of nginx HTML is of no value to users. Here it is replaced with a clear pointer to the upstream + actionable guidance.
        // Avoid the recurring misunderstanding of "thinking that Copilot Bridge Atlas encapsulates nginx / is the fault of the local proxy".
        format!(
            concat!(
                "Upstream provider rejected the request with HTTP 413 (Payload Too Large). ",
                "The request body exceeds the upstream gateway's size limit; this is the ",
                "provider's server-side limit, not a Copilot Bridge Atlas limit. ",
                "Provider: {provider}; model: {model}; endpoint: {endpoint}. ",
                "To recover, shrink the request: run /compact, remove large pasted logs or ",
                "inline images, or ask the provider to raise its request body limit ",
                "(e.g. nginx client_max_body_size)."
            ),
            provider = provider_name,
            model = request_model,
            endpoint = endpoint,
        )
    } else {
        let cause = error_obj
            .get("message")
            .and_then(|value| value.as_str())
            .map(ToString::to_string)
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| get_error_message(error));
        let status_fragment = upstream_status
            .map(|status| format!("; upstream_status: HTTP {status}"))
            .unwrap_or_default();
        if upstream_status == Some(408) {
            format!(
                "Upstream provider returned HTTP 408 (Request Timeout). \
                 Atlas received this response from upstream; it is not Atlas's local timeout. \
                 Provider: {provider_name}; model: {request_model}; endpoint: {endpoint}; cause: {cause}. \
                 Retry once. If this repeats for a long conversation, reduce its context or \
                 start a new chat with a short handoff. A shorter follow-up alone still includes \
                 the existing conversation. This response does not establish a token-limit violation."
            )
        } else {
            format!(
                "Copilot Bridge Atlas local proxy failed while handling Codex endpoint {endpoint}. Provider: {provider_name}; model: {request_model}{status_fragment}; cause: {cause}"
            )
        }
    };

    error_obj.insert(
        "message".to_string(),
        Value::String(compact_error_message(&message, 1800)),
    );

    if error_obj
        .get("type")
        .and_then(|value| value.as_str())
        .map(|value| value.trim().is_empty())
        .unwrap_or(true)
    {
        error_obj.insert("type".to_string(), Value::String("proxy_error".to_string()));
    }

    if error_obj.get("code").map(Value::is_null).unwrap_or(true) {
        error_obj.insert(
            "code".to_string(),
            Value::String(codex_proxy_error_code(error).to_string()),
        );
    }

    if !error_obj.contains_key("param") {
        error_obj.insert("param".to_string(), Value::Null);
    }

    error_obj.insert(
        "provider".to_string(),
        Value::String(provider_name.to_string()),
    );
    error_obj.insert(
        "model".to_string(),
        Value::String(request_model.to_string()),
    );
    // Use only for Codex local routing; do not reuse to query endpoints that may carry credentials.
    error_obj.insert("endpoint".to_string(), Value::String(endpoint.to_string()));
    if let Some(status) = upstream_status {
        error_obj.insert(
            "upstream_status".to_string(),
            Value::Number(serde_json::Number::from(status)),
        );
    }

    body
}

fn codex_proxy_error_code(error: &ProxyError) -> &'static str {
    match error {
        ProxyError::ForwardFailed(_) => "copilot_bridge_atlas_forward_failed",
        ProxyError::Timeout(_) => "copilot_bridge_atlas_timeout",
        ProxyError::NoProvidersConfigured => "copilot_bridge_atlas_no_providers_configured",
        ProxyError::ConfigError(_) => "copilot_bridge_atlas_config_error",
        ProxyError::TransformError(_) => "copilot_bridge_atlas_transform_error",
        ProxyError::InvalidRequest(_) => "copilot_bridge_atlas_invalid_request",
        ProxyError::RequestBodyTooLarge(_) => "copilot_bridge_atlas_request_body_too_large",
        ProxyError::AuthError(_) => "copilot_bridge_atlas_auth_error",
        ProxyError::UpstreamError { .. } => "copilot_bridge_atlas_upstream_error",
        ProxyError::DatabaseError(_) => "copilot_bridge_atlas_database_error",
        ProxyError::Internal(_) => "copilot_bridge_atlas_internal_error",
        ProxyError::AlreadyRunning
        | ProxyError::NotRunning
        | ProxyError::BindFailed(_)
        | ProxyError::StopTimeout
        | ProxyError::StopFailed(_)
        | ProxyError::ResponseBodyTooLarge(_) => "copilot_bridge_atlas_proxy_error",
    }
}

fn compact_error_message(message: &str, max_chars: usize) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }

    let truncated = normalized
        .chars()
        .take(max_chars)
        .collect::<String>()
        .trim_end()
        .to_string();
    format!("{truncated}…(truncated)")
}

/// Determine whether the response body "looks like" SSE text (#2234 Sniffing).
///
/// Only called after JSON parsing has failed: it is impossible for a valid JSON to start with these prefixes, and there is zero chance of false positives.
/// Recognize all SSE field types, including the comment prefix used for
/// `: PROCESSING` comment line.
fn body_looks_like_sse(body: &str) -> bool {
    let trimmed = body.trim_start_matches('\u{feff}').trim_start();
    ["data:", "event:", "id:", "retry:", ":"]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

/// Construct upstream parsing errors with on-site diagnostics: only structured classification and metadata attached,
/// Prevent the response body from entering the persistence log indirectly through the error chain.
fn upstream_body_parse_error(
    prefix: &str,
    err: &serde_json::Error,
    headers: &axum::http::HeaderMap,
    body: &str,
) -> ProxyError {
    ProxyError::TransformError(format!(
        "{prefix}: {err} {}",
        body_diagnostics_suffix(headers, body)
    ))
}

/// When SSE aggregation fails, the same on-site diagnosis is attached to the internal error of the aggregator.
/// Mission #2234 The client of the sniffing arm also obtained root cause clues.
/// Rather than just "No chat completion choices in upstream SSE" naked messages without header/body.
fn aggregate_fallback_error(
    err: ProxyError,
    headers: &axum::http::HeaderMap,
    body: &str,
) -> ProxyError {
    let base = match &err {
        ProxyError::TransformError(m) => m.clone(),
        other => other.to_string(),
    };
    ProxyError::TransformError(format!("{base} {}", body_diagnostics_suffix(headers, body)))
}

/// Classify the main text into limited categories and retain key clues such as HTML/SSE/garbled code without recording the main text.
fn classify_body_for_diagnostics(body: &str) -> &'static str {
    let trimmed = body.trim_start_matches('\u{feff}').trim_start();
    if trimmed.is_empty() {
        return "empty";
    }
    if body_looks_like_sse(trimmed) {
        return "sse";
    }

    // Classification only examines the first 4 KiB to avoid oversized text returned by line scan anomalies again for diagnosis.
    let sample = trimmed.chars().take(4096).collect::<String>();
    let prefix = sample
        .chars()
        .take(256)
        .collect::<String>()
        .to_ascii_lowercase();
    if ["<!doctype html", "<html", "<head", "<body"]
        .iter()
        .any(|marker| prefix.starts_with(marker))
    {
        return "html";
    }
    if sample.contains('\u{fffd}')
        || sample
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return "binary-or-encoded";
    }
    if prefix.starts_with('{') || prefix.starts_with('[') {
        return "json-like";
    }
    "text"
}

/// On-site diagnostic suffix: content-type, content-encoding, body length and security classification, excluding body text.
fn body_diagnostics_suffix(headers: &axum::http::HeaderMap, body: &str) -> String {
    let header_str = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("<none>")
    };
    format!(
        "(content-type: {}; content-encoding: {}; body-bytes: {}; body-kind: {}; content omitted)",
        header_str("content-type"),
        header_str("content-encoding"),
        body.len(),
        classify_body_for_diagnostics(body),
    )
}

/// Extracts reportable error messages from the SSE chunk's error field. placeholder shape (empty object, empty message,
/// false, empty string, etc., commonly found in the error field attached to each chunk of OpenAI compatible gateways) returned
/// None - This should not be used to determine that the entire flow has failed (otherwise, the successful flow will be accidentally killed as the 422, C12/C2234 target group).
fn error_event_message(error: &Value) -> Option<String> {
    if let Some(msg) = error.get("message").and_then(|m| m.as_str()) {
        return (!msg.is_empty()).then(|| msg.to_string());
    }
    if let Some(s) = error.as_str() {
        return (!s.is_empty()).then(|| s.to_string());
    }
    None
}

/// Parse the event name and data payload of a single SSE block (multiline data are concatenated with \n per specification).
/// Allow leading whitespace at the beginning of the line before matching field names - aligned with the trim tolerance of body_looks_like_sse,
/// Otherwise the indented `  data:` line was accepted by sniffing but is silently lost here (C4). Return None if there are no data rows.
fn sse_block_parts(block: &str) -> Option<(String, String)> {
    let mut event_name = String::new();
    let mut data_lines: Vec<&str> = Vec::new();
    for line in block.lines() {
        let line = line.trim_start();
        if let Some(evt) = strip_sse_field(line, "event") {
            event_name = evt.trim().to_string();
        } else if let Some(d) = strip_sse_field(line, "data") {
            data_lines.push(d);
        }
    }
    (!data_lines.is_empty()).then(|| (event_name, data_lines.join("\n")))
}

/// Aggregate Chat Completions streaming SSE into a single chat.completion JSON (#2234).
///
/// Exclusively for non-streaming branches: upstream returns SSE body for stream:false but Content-Type is unmarked
/// text/event-stream, header check (is_sse) is invalid. After aggregation, it is fed to the existing non-streaming converter
/// (`chat_completion_to_response_with_context`),
/// What the client gets is still legal JSON, and the non-stream semantics remain unchanged.
/// Incremental merge semantics are aligned with providers/streaming.rs: tool_calls are positioned by delta.index,
/// id/name is overwritten when it appears, arguments string splicing; reasoning forms (reasoning_content/
/// reasoning / reasoning_details) merged into the same accumulator via the codex_chat_common common extractor;
/// finish_reason The first non-null value is locked (kimi-k2.6 will send it after tool_use
/// The tail block of finish_reason, see streaming.rs).
fn chat_sse_to_response_value(body: &str) -> Result<Value, ProxyError> {
    // Strip BOM: The sniffer accepts the beginning of the BOM, but strip_sse_field matches exactly by the beginning of the line,
    // Not stripping will cause the first data row to be lost silently
    let mut buffer = body.trim_start_matches('\u{feff}').to_string();

    let mut id = Value::Null;
    let mut created = Value::Null;
    let mut model = Value::Null;
    let mut content = String::new();
    let mut reasoning_content = String::new();
    // tool_calls aggregated by index in BTreeMap: upstream controllable index (u64) will not densify
    // Array - the old `while len() <= index { push }` writing method will OOM the whole thing when encountering index=4e9
    // Process (C1). BTreeMap not only eliminates unbounded allocation, but also naturally maintains index output in order.
    let mut tool_calls: std::collections::BTreeMap<usize, Value> =
        std::collections::BTreeMap::new();
    let mut finish_reason = Value::Null;
    let mut usage = Value::Null;
    let mut saw_choice = false;
    let mut saw_done = false;

    // strict=false for residual tail blocks: the truncated JSON half is ignored instead of reporting an error, same as
    // Residual processing of responses_sse_to_response_value is symmetric (C2), otherwise a pinched
    // The tail block will accidentally kill the fully aggregated response as 422.
    let mut process_event =
        |event_name: &str, data_str: &str, strict: bool| -> Result<(), ProxyError> {
            let trimmed = data_str.trim();
            if trimmed == "[DONE]" {
                saw_done = true;
                return Ok(());
            }
            if trimmed.is_empty() {
                return Ok(());
            }
            let chunk: Value = match serde_json::from_str(data_str) {
                Ok(v) => v,
                Err(_) if !strict => return Ok(()),
                Err(e) => {
                    return Err(ProxyError::TransformError(format!(
                        "Failed to parse upstream SSE chunk: {e}"
                    )))
                }
            };

            // `event: error` event: The error is marked by the event name, and the data body may not have an error key (directly
            // error object). Even if the complete choice has been aggregated before, it must be judged as a failure, otherwise the gateway's
            // Quota/throttling errors disguised as successes (C18).
            if event_name.eq_ignore_ascii_case("error") {
                let message = chunk
                    .get("error")
                    .and_then(error_event_message)
                    .or_else(|| error_event_message(&chunk))
                    .unwrap_or_else(|| "upstream error event in SSE stream".to_string());
                return Err(ProxyError::TransformError(message));
            }
            // The gateway delivers the error as a normal data chunk ({"error":{...}}): only if the error contains
            // The reportable message failed. Empty object/empty message/null/false and other placeholder shapes (partial
            // Each chunk of the OpenAI compatible gateway is provided) and cannot accidentally kill the successful flow (C12) accordingly.
            if let Some(message) = chunk
                .get("error")
                .filter(|e| !e.is_null())
                .and_then(error_event_message)
            {
                return Err(ProxyError::TransformError(message));
            }

            // The first "meaningful" value locks the envelope. Azure's content-filter pre-block band
            // ""/0 placeholder (streaming.rs has the same empty string guard), the placeholder value cannot be used to freeze the field
            for (slot, key) in [
                (&mut id, "id"),
                (&mut created, "created"),
                (&mut model, "model"),
            ] {
                if slot.is_null() {
                    if let Some(v) = chunk.get(key).filter(|v| envelope_value_meaningful(v)) {
                        *slot = v.clone();
                    }
                }
            }
            // OpenAI semantics: usage only when the final chunk is non-null
            if let Some(u) = chunk.get("usage").filter(|u| !u.is_null()) {
                usage = u.clone();
            }

            // The agent context only has a single choice (n=1), and only aggregates choices with index==0
            let Some(choice) = chunk
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .find(|ch| ch.get("index").and_then(|i| i.as_u64()).unwrap_or(0) == 0)
                })
            else {
                return Ok(());
            };

            // Evidence of "seen response" must be choice payload: metadata/usage-only chunk +
            // If the flow of [DONE] (no choice in the whole process) counts, it will bypass the two guards below.
            // Packaging empty content fake success
            saw_choice = true;

            // finish_reason first non-null is locked (aligned with first-wins of streaming.rs:
            // Multiple finish_reason The upstream tail block "stop" cannot overwrite the first arriving "tool_calls")
            if finish_reason.is_null() {
                if let Some(fr) = choice.get("finish_reason").filter(|v| !v.is_null()) {
                    finish_reason = fr.clone();
                }
            }
            // Payload selection: Normal increment is delta; but fake streaming transfer will complete chat.completion
            // Packaged as a single event (message instead of delta), some also come with empty delta:{}. delta is an empty object
            // And when there is a message, the message snapshot is used instead (overwriting the previous accumulated increment to prevent double counting in mixed forms),
            // Otherwise, the content is silently discarded, and the completion guard is penetrated by its finish_reason → empty content is falsely successful (C3).
            let delta_nonempty = choice
                .get("delta")
                .and_then(|d| d.as_object())
                .is_some_and(|o| !o.is_empty());
            let (payload, is_full_message) = if delta_nonempty {
                (choice.get("delta").unwrap(), false)
            } else if let Some(message) = choice.get("message") {
                (message, true)
            } else if let Some(delta) = choice.get("delta") {
                // Empty delta and no message: normal pure finish_reason closing block
                (delta, false)
            } else {
                return Ok(());
            };
            if is_full_message {
                content.clear();
                reasoning_content.clear();
                tool_calls.clear();
            }
            match payload.get("content") {
                Some(Value::String(text)) => content.push_str(text),
                Some(Value::Array(parts)) => {
                    for part in parts {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            content.push_str(text);
                        } else if let Some(refusal) = part.get("refusal").and_then(|r| r.as_str()) {
                            content.push_str(refusal);
                        }
                    }
                }
                _ => {}
            }
            // refusal: OpenAI official refusal form (delta.refusal / message.refusal string).
            // Both downstream converters treat refusal as visible content. Missing the read will make the refusal response an empty message and a false success (C15).
            if let Some(refusal) = payload.get("refusal").and_then(|r| r.as_str()) {
                content.push_str(refusal);
            }
            // Exhaustive extraction of reasoning field directly reuses codex_chat_common (reasoning_content >
            // reasoning string/object > reasoning_details) to avoid missing files in the third handwritten implementation:
            // Also preserve reasoning exposed through the alternate details field.
            if let Some(text) = extract_reasoning_field_text(payload) {
                reasoning_content.push_str(&text);
            }
            if let Some(deltas) = payload.get("tool_calls").and_then(|t| t.as_array()) {
                for (pos, tc) in deltas.iter().enumerate() {
                    merge_tool_call_delta(&mut tool_calls, tc, pos);
                }
            } else if let Some(fc) = payload.get("function_call").filter(|v| !v.is_null()) {
                // legacy function_call (deprecated in 2023 but still has transitive callbacks) → when single tool_call.
                // Both downstream converters support function_call, and missing reads will cause finish_reason
                // "function_call"→stop_reason "tool_use" has zero tool blocks and blocks the agent loop (C17).
                let synthetic = json!({
                    "index": 0,
                    "id": fc.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                    "type": "function",
                    "function": fc,
                });
                merge_tool_call_delta(&mut tool_calls, &synthetic, 0);
            }
            Ok(())
        };

    while let Some(block) = take_sse_block(&mut buffer) {
        if let Some((event, data)) = sse_block_parts(&block) {
            process_event(&event, &data, true)?;
        }
    }
    // There may be no empty line delimiter after the last event (half-stream/non-canonical upstream): residual buffer when the last block
    // Handling, strict=false tolerates truncated tail blocks (C2).
    if let Some((event, data)) = sse_block_parts(&buffer) {
        process_event(&event, &data, false)?;
    }

    if !saw_choice {
        return Err(ProxyError::TransformError(
            "No chat completion choices in upstream SSE".to_string(),
        ));
    }
    // Completion guard: close-delimited response truncation is not detectable at the byte level, missing
    // finish_reason and [DONE] are processed as truncation when completing the evidence to avoid half-cut content
    // A response wrapped to look "successful" is returned silently (a failure pattern that is harder to diagnose than 422).
    if finish_reason.is_null() && !saw_done {
        return Err(ProxyError::TransformError(
            "Upstream SSE stream appears truncated (no finish_reason or [DONE] marker)".to_string(),
        ));
    }

    // Discard empty tool-call slots. Missing IDs/names use the original index
    // so tool outputs can still reference a stable call identity.
    let tool_calls: Vec<Value> = tool_calls
        .into_iter()
        .filter(|(_, tc)| {
            tc["id"].as_str().is_some_and(|s| !s.is_empty())
                || tc["function"]["name"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty())
                || tc["function"]["arguments"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty())
        })
        .map(|(index, mut tc)| {
            if tc["id"].as_str().is_none_or(str::is_empty) {
                tc["id"] = json!(format!("tool_call_{index}"));
            }
            if tc["function"]["name"].as_str().is_none_or(str::is_empty) {
                tc["function"]["name"] = json!("unknown_tool");
            }
            tc
        })
        .collect();

    let mut message = serde_json::Map::new();
    message.insert("role".to_string(), json!("assistant"));
    message.insert("content".to_string(), json!(content));
    if !reasoning_content.is_empty() {
        message.insert("reasoning_content".to_string(), json!(reasoning_content));
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }

    // Synthesize UUID when the upstream does not return a valid id: leaving null/"" will cause the downstream dedup_request_id to degenerate to
    // Constant "session:" global collision, INSERT OR REPLACE silently overwrites previous usage lines, undercounting costs (C9).
    let id = if envelope_value_meaningful(&id) {
        id
    } else {
        json!(uuid::Uuid::new_v4().to_string())
    };

    let mut response = json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": Value::Object(message),
            "finish_reason": finish_reason,
        }],
    });
    if !usage.is_null() {
        response["usage"] = usage;
    }
    Ok(response)
}

/// Whether the envelope field is "meaningful": filtering null, empty string and value 0 (including floating point 0.0 - Azure
/// The placeholder value of the content-filter preceding block) prevents the placeholder value from preemptively freezing id/model/created.
fn envelope_value_meaningful(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
        _ => true,
    }
}

/// Merge single tool_calls increment into BTreeMap aggregated by index: OpenAI streaming puts id/name
/// The first increment and arguments are distributed in slices, and the target is located according to delta.index; when the index is missing, it returns to the array where it is located.
/// (The complete tool_calls in message form usually do not have index, and pressing 0 will overwrite each other).
fn merge_tool_call_delta(
    tool_calls: &mut std::collections::BTreeMap<usize, Value>,
    delta: &Value,
    fallback_index: usize,
) {
    let index = delta
        .get("index")
        .and_then(|i| i.as_u64())
        .map(|i| i as usize)
        .unwrap_or(fallback_index);
    let target = tool_calls.entry(index).or_insert_with(|| {
        json!({
            "id": "",
            "type": "function",
            "function": {"name": "", "arguments": ""}
        })
    });
    if let Some(v) = delta
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        target["id"] = json!(v);
    }
    if let Some(func) = delta.get("function") {
        if let Some(name) = func
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            target["function"]["name"] = json!(name);
        }
        // arguments: string concatenated directly; object/array concatenated after serialization - non-stream message
        // Snapshots often return arguments as objects (OpenAI compatibility deviation), and only recognizing string will lose parameters.
        // Causes tool to execute with empty input (C16)
        match func.get("arguments") {
            Some(Value::String(args)) => {
                if let Some(existing) = target["function"]["arguments"].as_str() {
                    target["function"]["arguments"] = json!(format!("{existing}{args}"));
                }
            }
            Some(v @ (Value::Object(_) | Value::Array(_))) => {
                let serialized = serde_json::to_string(v).unwrap_or_default();
                if let Some(existing) = target["function"]["arguments"].as_str() {
                    target["function"]["arguments"] = json!(format!("{existing}{serialized}"));
                }
            }
            _ => {}
        }
    }
}

// ============================================================================
// Usage logging for the Codex Chat bridge.
// ============================================================================

fn log_forward_error(ctx: &RequestContext, error: &ProxyError) {
    ctx.diagnostics.proxy_failure("forward", error);
}

/// Record request usage
#[allow(clippy::too_many_arguments)]
async fn log_usage(
    state: &ProxyState,
    provider_id: &str,
    app_type: &str,
    model: &str,
    request_model: &str,
    _outbound_model: &str,
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

#[cfg(test)]
mod tests {
    use super::{
        body_looks_like_sse, chat_sse_to_response_value, classify_body_for_diagnostics,
        codex_proxy_error_json, read_codex_request_body, upstream_body_parse_error,
    };
    use crate::proxy::ProxyError;

    #[tokio::test]
    async fn reasoning_metadata_is_always_logged_for_native_and_chat_streaming_and_json() {
        use crate::{
            database::Database,
            provider::Provider,
            proxy::{
                handler_config::CODEX_PARSER_CONFIG,
                handler_context::RequestContext,
                provider_router::ProviderRouter,
                providers::codex_chat_history::CodexChatHistoryStore,
                response_processor::process_response,
                server::ProxyState,
                types::{CopilotOptimizerConfig, ReasoningEffort},
                upstream_response::ProxyResponse,
            },
            services::usage_stats::LogFilters,
        };
        use serde_json::json;
        use std::{collections::HashMap, sync::Arc, time::Instant};
        use tokio::sync::RwLock;

        for chat in [false, true] {
            for stream in [false, true] {
                let db = Arc::new(Database::memory().unwrap());
                let config = db.get_proxy_config().await.unwrap();
                let state = ProxyState {
                    db: db.clone(),
                    config: Arc::new(RwLock::new(config)),
                    status: Arc::new(RwLock::new(Default::default())),
                    start_time: Arc::new(RwLock::new(None)),
                    current_providers: Arc::new(RwLock::new(HashMap::new())),
                    provider_router: Arc::new(ProviderRouter::new(db.clone())),
                    codex_chat_history: Arc::new(CodexChatHistoryStore::default()),
                    app_handle: None,
                };
                let ctx = RequestContext {
                    start_time: Instant::now(),
                    provider: Provider::with_id("fixture".into(), "Fixture".into(), json!({})),
                    request_model: "gpt-6-astra".into(),
                    outbound_model: Some("gpt-6-astra".into()),
                    reasoning_effort: ReasoningEffort {
                        requested: Some("ultra".into()),
                        applied: Some("high".into()),
                    },
                    tag: "Codex",
                    app_type_str: "codex",
                    session_id: "reasoning-fixture".into(),
                    session_client_provided: true,
                    copilot_optimizer_config: CopilotOptimizerConfig::default(),
                    diagnostics: crate::proxy::diagnostics::RequestDiagnostics::new(
                        db.clone(),
                        "fixture".into(),
                        "gpt-6-astra".into(),
                        "test-session".into(),
                        Instant::now(),
                    ),
                };
                let body = if chat {
                    let mut choice = json!({"finish_reason":"stop"});
                    choice[if stream { "delta" } else { "message" }] =
                        json!({"role":"assistant","content":"Done"});
                    json!({
                        "id":"chatcmpl_reasoning_fixture","model":"gpt-6-astra",
                        "choices":[choice],"usage":{"prompt_tokens":10,"completion_tokens":3}
                    })
                } else {
                    json!({
                        "id":"resp_reasoning_fixture","model":"gpt-6-astra",
                        "output":[],"usage":{"input_tokens":10,"output_tokens":3}
                    })
                };
                let mut headers = axum::http::HeaderMap::new();
                let body = if stream {
                    headers.insert("content-type", "text/event-stream".parse().unwrap());
                    let event = if chat {
                        body
                    } else {
                        json!({"type":"response.completed","response":body})
                    };
                    format!("data: {event}\n\ndata: [DONE]\n\n")
                } else {
                    body.to_string()
                };
                let upstream = ProxyResponse::buffered(
                    axum::http::StatusCode::OK,
                    headers,
                    bytes::Bytes::from(body),
                );
                let response = if chat {
                    super::handle_codex_chat_to_responses_transform(
                        upstream,
                        &ctx,
                        &state,
                        stream,
                        None,
                        Default::default(),
                    )
                    .await
                    .unwrap()
                } else {
                    process_response(upstream, &ctx, &state, &CODEX_PARSER_CONFIG, None)
                        .await
                        .unwrap()
                };
                axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap();
                let logs = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    loop {
                        let logs = db.get_request_logs(&LogFilters::default(), 0, 20).unwrap();
                        if !logs.data.is_empty() {
                            break logs;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                })
                .await
                .expect("usage logging should complete");
                assert_eq!(logs.total, 1, "chat={chat}, stream={stream}");
                assert_eq!(
                    logs.data[0].requested_reasoning_effort.as_deref(),
                    Some("ultra")
                );
                assert_eq!(
                    logs.data[0].applied_reasoning_effort.as_deref(),
                    Some("high")
                );
                assert_eq!(logs.data[0].is_streaming, stream);
            }
        }
    }

    fn encode_request(encoding: &str, body: &[u8]) -> Vec<u8> {
        use std::io::Write;
        match encoding {
            "identity" => body.to_vec(),
            "gzip" => {
                let mut encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(body).unwrap();
                encoder.finish().unwrap()
            }
            "deflate" => {
                let mut encoder =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(body).unwrap();
                encoder.finish().unwrap()
            }
            "zstd" => zstd::stream::encode_all(body, 0).unwrap(),
            "gzip, zstd" => {
                zstd::stream::encode_all(encode_request("gzip", body).as_slice(), 0).unwrap()
            }
            _ => panic!("unknown fixture encoding"),
        }
    }

    #[tokio::test]
    async fn request_body_limits_wire_bytes_before_collecting_the_rest() {
        use axum::{body::Body, http::HeaderMap, response::IntoResponse};
        use bytes::Bytes;
        use futures::StreamExt;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        let polls = Arc::new(AtomicUsize::new(0));
        let count = polls.clone();
        let stream = futures::stream::iter([
            Ok::<_, std::io::Error>(Bytes::from_static(b"1234")),
            Ok(Bytes::from_static(b"5678")),
            Ok(Bytes::from_static(b"must not be read")),
        ])
        .inspect(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
        });
        let error = read_codex_request_body(&mut HeaderMap::new(), Body::from_stream(stream), 6)
            .await
            .unwrap_err();
        assert!(matches!(error, ProxyError::RequestBodyTooLarge(6)));
        assert_eq!(
            error.into_response().status(),
            axum::http::StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(polls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn request_body_limits_decoded_bytes_for_each_encoding() {
        use axum::{body::Body, http::HeaderMap};
        let body = serde_json::to_vec(&serde_json::json!({
            "model": "gpt-6-astra",
            "input": "x".repeat(4096)
        }))
        .unwrap();
        for encoding in ["gzip", "deflate", "zstd", "gzip, zstd"] {
            let compressed = encode_request(encoding, &body);
            assert!(compressed.len() < 1024);
            let mut headers = HeaderMap::new();
            headers.insert("content-encoding", encoding.parse().unwrap());
            let result = read_codex_request_body(&mut headers, Body::from(compressed), 1024).await;
            assert!(
                matches!(result, Err(ProxyError::RequestBodyTooLarge(1024))),
                "{encoding}: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn request_body_preserves_tools_images_and_headers_within_the_limit() {
        use axum::{body::Body, http::HeaderMap};
        let expected = serde_json::json!({
            "model": "gpt-6-astra",
            "input": [
                {"type": "message", "role": "user", "content": [
                    {"type": "input_text", "text": "Keep this content exactly. ".repeat(64)},
                    {"type": "input_image", "image_url": "data:image/png;base64,aGVsbG8="}
                ]},
                {"type": "function_call_output", "call_id": "call_fixture", "output": "Hello"}
            ],
            "tools": [{"type": "function", "name": "check", "parameters": {"type": "object"}}]
        });
        let body = serde_json::to_vec(&expected).unwrap();
        for encoding in ["identity", "gzip", "deflate", "zstd", "gzip, zstd"] {
            let compressed = encode_request(encoding, &body);
            let mut headers = HeaderMap::new();
            headers.insert("content-encoding", encoding.parse().unwrap());
            headers.insert("content-length", compressed.len().into());
            headers.insert("x-client-request-id", "request_fixture".parse().unwrap());
            let parsed = read_codex_request_body(&mut headers, Body::from(compressed), body.len())
                .await
                .unwrap();
            assert_eq!(parsed, expected, "{encoding}");
            assert_eq!(headers["x-client-request-id"], "request_fixture");
            if encoding != "identity" {
                assert!(!headers.contains_key("content-encoding"));
                assert!(!headers.contains_key("content-length"));
            }
        }
    }

    #[test]
    fn body_looks_like_sse_detects_unlabeled_sse_prefixes() {
        assert!(body_looks_like_sse("data: {\"id\":\"1\"}\n\n"));
        assert!(body_looks_like_sse("event: message\ndata: {}\n\n"));
        // The other two field lines of the SSE specification may also start with
        assert!(body_looks_like_sse("id: 1\ndata: {}\n\n"));
        assert!(body_looks_like_sse("retry: 3000\ndata: {}\n\n"));
        // OpenRouter will send a comment line before the stream
        assert!(body_looks_like_sse(
            ": OPENROUTER PROCESSING\n\ndata: {}\n\n"
        ));
        // BOM + leading blank
        assert!(body_looks_like_sse("\u{feff}\n  data: {}\n\n"));
        // HTML intercept pages and normal text should not be misinterpreted as SSE
        assert!(!body_looks_like_sse("<html><body>blocked</body></html>"));
        assert!(!body_looks_like_sse("Bad Gateway"));
        assert!(!body_looks_like_sse(""));
    }

    #[test]
    fn upstream_body_parse_error_carries_field_diagnostics() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("content-type", "text/html".parse().unwrap());
        headers.insert("content-encoding", "gzip".parse().unwrap());
        let parse_err = serde_json::from_str::<serde_json::Value>("<html>").unwrap_err();

        let err = upstream_body_parse_error(
            "Failed to parse upstream response",
            &parse_err,
            &headers,
            "<html>\nblocked</html>",
        );

        match err {
            ProxyError::TransformError(msg) => {
                assert!(msg.contains("content-type: text/html"), "{msg}");
                assert!(msg.contains("content-encoding: gzip"), "{msg}");
                assert!(msg.contains("body-bytes: 21"), "{msg}");
                assert!(msg.contains("body-kind: html"), "{msg}");
                assert!(!msg.contains("blocked"), "{msg}");
            }
            other => panic!("expected TransformError, got {other:?}"),
        }
    }

    #[test]
    fn upstream_body_parse_error_marks_missing_headers() {
        let headers = axum::http::HeaderMap::new();
        let parse_err = serde_json::from_str::<serde_json::Value>("data:").unwrap_err();

        let err = upstream_body_parse_error("x", &parse_err, &headers, "data: oops");

        match err {
            ProxyError::TransformError(msg) => {
                assert!(msg.contains("content-type: <none>"), "{msg}");
                assert!(msg.contains("content-encoding: <none>"), "{msg}");
                assert!(msg.contains("body-kind: sse"), "{msg}");
            }
            other => panic!("expected TransformError, got {other:?}"),
        }
    }

    #[test]
    fn body_diagnostics_classifies_without_exposing_content() {
        assert_eq!(classify_body_for_diagnostics(""), "empty");
        assert_eq!(classify_body_for_diagnostics("  <HTML>blocked"), "html");
        assert_eq!(classify_body_for_diagnostics("data: {}\n\n"), "sse");
        assert_eq!(classify_body_for_diagnostics("{\"ok\":true}"), "json-like");
        assert_eq!(
            classify_body_for_diagnostics("decoded\u{fffd}payload"),
            "binary-or-encoded"
        );
        assert_eq!(classify_body_for_diagnostics("Bad Gateway"), "text");
    }

    #[test]
    fn chat_sse_to_response_value_collects_reasoning_alias() {
        // OpenRouter/Kimi uses reasoning (string), and some gateways use object form.
        let sse = "data: {\"id\":\"c1\",\"model\":\"kimi-k2.6\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning\":\"think\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning\":{\"content\":\"ing\"},\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(
            response["choices"][0]["message"]["reasoning_content"],
            "thinking"
        );
        assert_eq!(response["choices"][0]["message"]["content"], "ok");
    }

    #[test]
    fn chat_sse_to_response_value_collects_reasoning_details() {
        // MiMo/OpenRouter and other providers that only send reasoning_details (array form),
        // Through the public extractor, the content of thinking cannot be lost.
        let sse = "data: {\"id\":\"c1\",\"model\":\"mimo\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_details\":[{\"type\":\"reasoning.text\",\"text\":\"think\"}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_details\":[{\"type\":\"reasoning.text\",\"text\":\"ing\"}],\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(
            response["choices"][0]["message"]["reasoning_content"],
            "thinking"
        );
        assert_eq!(response["choices"][0]["message"]["content"], "ok");
    }

    #[test]
    fn chat_sse_to_response_value_skips_azure_placeholder_envelope() {
        // Azure content-filter prefix block has ""/0 placeholder, envelope field cannot be frozen
        let sse = "data: {\"id\":\"\",\"model\":\"\",\"created\":0,\"object\":\"\",\"choices\":[],\"prompt_filter_results\":[]}\n\n\
data: {\"id\":\"chatcmpl-real\",\"model\":\"gpt-5.4\",\"created\":42,\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(response["id"], "chatcmpl-real");
        assert_eq!(response["model"], "gpt-5.4");
        assert_eq!(response["created"], 42);
    }

    #[test]
    fn chat_sse_to_response_value_tolerates_null_error_field() {
        // Each chunk of the one-api system gateway has "error": null, which cannot be misjudged as an upstream error.
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"error\":null,\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(response["choices"][0]["message"]["content"], "hi");
    }

    #[test]
    fn chat_sse_to_response_value_first_finish_reason_wins() {
        // kimi-k2.6 will send the tail block with finish_reason after tool_use.
        // Tail block "stop" cannot overwrite first arriving "tool_calls" (aligned streaming.rs first-wins)
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"f\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n\
data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(response["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn chat_sse_to_response_value_unwraps_message_shaped_fake_stream() {
        // Fake streaming wraps the complete chat.completion into a single SSE event (message instead of delta)
        let sse = "data: {\"id\":\"c1\",\"object\":\"chat.completion\",\"model\":\"m\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"full answer\"},\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(response["choices"][0]["message"]["content"], "full answer");
        assert_eq!(response["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn chat_sse_to_response_value_message_snapshot_overrides_deltas() {
        // Mixed form: when sending the increment first and then the complete message snapshot, the snapshot covers the increment (to prevent double counting)
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"par\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"full\"},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(response["choices"][0]["message"]["content"], "full");
    }

    #[test]
    fn chat_sse_to_response_value_backfills_sparse_tool_call_ids() {
        // Empty shells with empty index are discarded; shells with missing id are backfilled according to the original index tool_call_{idx}
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"name\":\"f2\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        let tool_calls = response["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(
            tool_calls.len(),
            1,
            "The empty shell at index 0 should be discarded"
        );
        assert_eq!(tool_calls[0]["id"], "tool_call_1");
        assert_eq!(tool_calls[0]["function"]["name"], "f2");
    }

    #[test]
    fn chat_sse_to_response_value_strips_bom_before_parsing() {
        // The sniffer accepts the BOM, and the block parsing must also strip it, otherwise the first data row is silently lost
        let sse = "\u{feff}data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(response["choices"][0]["message"]["content"], "hi");
    }

    #[test]
    fn chat_sse_to_response_value_aggregates_text_finish_reason_and_usage() {
        let sse = "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":123,\"model\":\"gpt-5.4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,\"total_tokens\":12}}\n\n\
data: [DONE]\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(response["id"], "chatcmpl-1");
        assert_eq!(response["object"], "chat.completion");
        assert_eq!(response["model"], "gpt-5.4");
        assert_eq!(response["choices"][0]["message"]["role"], "assistant");
        assert_eq!(response["choices"][0]["message"]["content"], "Hello");
        assert_eq!(response["choices"][0]["finish_reason"], "stop");
        assert_eq!(response["usage"]["prompt_tokens"], 10);
    }

    #[test]
    fn chat_sse_to_response_value_merges_tool_call_argument_fragments() {
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"city\\\":\"}}]},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"SF\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n\
data: [DONE]\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        let tool_call = &response["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(tool_call["id"], "call_1");
        assert_eq!(tool_call["function"]["name"], "get_weather");
        assert_eq!(tool_call["function"]["arguments"], "{\"city\":\"SF\"}");
        assert_eq!(response["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn chat_sse_to_response_value_collects_reasoning_content() {
        let sse = "data: {\"id\":\"c1\",\"model\":\"gpt-reasoner\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"think\"},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"ing\",\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(
            response["choices"][0]["message"]["reasoning_content"],
            "thinking"
        );
        assert_eq!(response["choices"][0]["message"]["content"], "ok");
    }

    #[test]
    fn chat_sse_to_response_value_handles_missing_trailing_blank_line() {
        // Non-canonical upstream/half-cut: no blank line delimiter after last event
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(response["choices"][0]["message"]["content"], "hi");
    }

    #[test]
    fn chat_sse_to_response_value_handles_crlf_delimiters() {
        // Real HTTP SSE uses \r\n\r\n delimited events as per specification
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\r\n\
\r\n\
data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\r\n\
\r\n\
data: [DONE]\r\n\
\r\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(response["choices"][0]["message"]["content"], "hi");
        assert_eq!(response["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn chat_sse_to_response_value_propagates_upstream_error_event() {
        let sse = "data: {\"error\":{\"message\":\"rate limited by gateway\",\"code\":429}}\n\n";

        let err = chat_sse_to_response_value(sse).unwrap_err();
        match err {
            ProxyError::TransformError(msg) => assert!(msg.contains("rate limited by gateway")),
            other => panic!("expected TransformError, got {other:?}"),
        }
    }

    #[test]
    fn chat_sse_to_response_value_rejects_truncated_stream() {
        // Only content increment, no finish_reason and no [DONE]: close-delimited cannot be truncated
        // When detecting at the byte level, an error must be reported by truncation instead of silently returning half the content.
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"par\"},\"finish_reason\":null}]}\n\n";

        let err = chat_sse_to_response_value(sse).unwrap_err();
        match err {
            ProxyError::TransformError(msg) => assert!(msg.contains("truncated")),
            other => panic!("expected TransformError, got {other:?}"),
        }
    }

    #[test]
    fn chat_sse_to_response_value_accepts_done_marker_without_finish_reason() {
        // Non-standard upstream may not send finish_reason but ends normally [DONE]: considered completed
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n\
data: [DONE]\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();

        assert_eq!(response["choices"][0]["message"]["content"], "hi");
        assert_eq!(
            response["choices"][0]["finish_reason"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn chat_sse_to_response_value_rejects_stream_without_chunks() {
        let err = chat_sse_to_response_value(": keepalive\n\ndata: [DONE]\n\n").unwrap_err();
        match err {
            ProxyError::TransformError(msg) => {
                assert!(msg.contains("No chat completion choices"))
            }
            other => panic!("expected TransformError, got {other:?}"),
        }
    }

    #[test]
    fn chat_sse_to_response_value_rejects_choiceless_stream_despite_done() {
        // metadata/usage-only chunk + [DONE], no choice payload in the whole process:
        // You cannot pretend success by packaging [DONE] into empty content (saw_choice must use choice as evidence)
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":0,\"total_tokens\":1}}\n\n\
data: [DONE]\n\n";

        let err = chat_sse_to_response_value(sse).unwrap_err();
        match err {
            ProxyError::TransformError(msg) => {
                assert!(msg.contains("No chat completion choices"), "{msg}")
            }
            other => panic!("expected TransformError, got {other:?}"),
        }
    }

    #[test]
    fn chat_sse_to_response_value_huge_tool_call_index_does_not_oom() {
        // C1: Huge index controllable by upstream must not densify the array (the old implementation will OOM the entire process);
        // BTreeMap only occupies one slot, and the original index is used to backfill the synthetic id
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":4000000000,\"function\":{\"name\":\"f\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();
        let tool_calls = response["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "tool_call_4000000000");
        assert_eq!(tool_calls[0]["function"]["name"], "f");
    }

    #[test]
    fn chat_sse_to_response_value_empty_delta_falls_back_to_message_snapshot() {
        // C3: The same choice has both empty delta:{} and complete message snapshot - cannot be used due to delta key
        // If it exists, it will short-circuit to empty delta and discard the message content (finish_reason will also penetrate the guard)
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"message\":{\"role\":\"assistant\",\"content\":\"full answer\"},\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();
        assert_eq!(response["choices"][0]["message"]["content"], "full answer");
        assert_eq!(response["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn chat_sse_to_response_value_empty_delta_scaffold_does_not_wipe_real_content() {
        // C3 reverse trap: when each chunk has true content delta + empty message shell, it cannot be empty
        // message triggers clear to erase the accumulated content (delta is given priority if delta is not empty, and snapshot overwriting is not performed)
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"message\":{},\"finish_reason\":null}]}\n\n\
data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" there\"},\"message\":{},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();
        assert_eq!(response["choices"][0]["message"]["content"], "hi there");
    }

    #[test]
    fn chat_sse_to_response_value_object_form_tool_arguments_preserved() {
        // C16: The arguments in the message snapshot are serialized and retained when the object is returned, and cannot be thrown into empty input.
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"tool_calls\":[{\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":{\"city\":\"SF\"}}}]},\"finish_reason\":\"tool_calls\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();
        let args = response["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(args).unwrap();
        assert_eq!(parsed["city"], "SF");
    }

    #[test]
    fn chat_sse_to_response_value_collects_refusal() {
        // C15: The delta.refusal string is merged into the visible content to avoid rejecting the response and turning the empty message into a false success.
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"refusal\":\"I can't help with that.\"},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();
        assert_eq!(
            response["choices"][0]["message"]["content"],
            "I can't help with that."
        );
    }

    #[test]
    fn chat_sse_to_response_value_maps_legacy_function_call() {
        // C17: legacy function_call → single tool_call, avoid finish_reason
        // function_call is mapped to tool_use but zero tool block blocks the agent
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":null,\"function_call\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"city\\\":\\\"SF\\\"}\"}},\"finish_reason\":\"function_call\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();
        let tc = &response["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(tc["function"]["name"], "get_weather");
        assert_eq!(tc["function"]["arguments"], "{\"city\":\"SF\"}");
    }

    #[test]
    fn chat_sse_to_response_value_event_error_fails_even_after_complete_choice() {
        // C18: event:error (data without error key) fails even if it follows a complete choice.
        // Can't pretend to be successful
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"},\"finish_reason\":\"stop\"}]}\n\n\
event: error\n\
data: {\"message\":\"insufficient_user_quota\",\"code\":429}\n\n";

        let err = chat_sse_to_response_value(sse).unwrap_err();
        match err {
            ProxyError::TransformError(msg) => {
                assert!(msg.contains("insufficient_user_quota"), "{msg}")
            }
            other => panic!("expected TransformError, got {other:?}"),
        }
    }

    #[test]
    fn chat_sse_to_response_value_tolerates_empty_error_placeholder() {
        // C12: error is an empty object/empty message and other placeholder shapes must not accidentally kill the success stream
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"error\":{},\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();
        assert_eq!(response["choices"][0]["message"]["content"], "hi");
    }

    #[test]
    fn chat_sse_to_response_value_tolerates_truncated_residual_after_complete() {
        // C2: After the complete finish_reason block, the tail block is cut off (half JSON), and the complete aggregation cannot be accidentally killed.
        let sse = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\n\
data: {\"usage\":{\"prompt_to";

        let response = chat_sse_to_response_value(sse).unwrap();
        assert_eq!(response["choices"][0]["message"]["content"], "hi");
    }

    #[test]
    fn chat_sse_to_response_value_float_zero_does_not_freeze_envelope() {
        // C14: The floating point 0.0 placeholder created must not freeze the envelope, and the true value should be able to overwrite it.
        let sse = "data: {\"id\":\"\",\"model\":\"\",\"created\":0.0,\"choices\":[]}\n\n\
data: {\"id\":\"chatcmpl-real\",\"model\":\"m\",\"created\":42,\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();
        assert_eq!(response["created"], 42);
        assert_eq!(response["id"], "chatcmpl-real");
    }

    #[test]
    fn chat_sse_to_response_value_synthesizes_id_when_absent() {
        // C9: Synthesize a non-empty unique id when the upstream has no id to avoid the downstream dedup from degenerating into constant collision coverage.
        let sse = "data: {\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\n";

        let r1 = chat_sse_to_response_value(sse).unwrap();
        let r2 = chat_sse_to_response_value(sse).unwrap();
        let id1 = r1["id"].as_str().unwrap();
        let id2 = r2["id"].as_str().unwrap();
        assert!(!id1.is_empty());
        assert_ne!(
            id1, id2,
            "Two aggregations without ids should produce different ids to avoid dedup collisions"
        );
    }

    #[test]
    fn chat_sse_to_response_value_accepts_indented_data_lines() {
        // C4: data lines with indentation at the beginning of the line (accepted by the sniffer) should also be aggregated and not lost silently
        let sse = "  data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\n";

        let response = chat_sse_to_response_value(sse).unwrap();
        assert_eq!(response["choices"][0]["message"]["content"], "hi");
    }

    #[test]
    fn codex_proxy_forward_error_includes_context_and_cause() {
        let error = ProxyError::ForwardFailed("Connection failed: dns lookup failed".to_string());
        let body = codex_proxy_error_json("Copilot", "gpt-chat", "/responses", &error);

        let message = body["error"]["message"].as_str().unwrap();
        assert!(message.contains("Copilot Bridge Atlas local proxy failed"));
        assert!(message.contains("Copilot"));
        assert!(message.contains("gpt-chat"));
        assert!(message.contains("/responses"));
        assert!(message.contains("dns lookup failed"));
        assert_eq!(body["error"]["code"], "copilot_bridge_atlas_forward_failed");
        assert_eq!(body["error"]["provider"], "Copilot");
        assert_eq!(body["error"]["model"], "gpt-chat");
    }

    #[test]
    fn codex_proxy_upstream_error_normalizes_nonstandard_body() {
        let error = ProxyError::UpstreamError {
            status: 502,
            body: Some(r#"{"detail":"upstream gateway failed"}"#.to_string()),
        };
        let body = codex_proxy_error_json("Copilot", "gpt-6-astra", "/responses", &error);

        let message = body["error"]["message"].as_str().unwrap();
        assert!(message.contains("upstream_status: HTTP 502"));
        assert!(message.contains("upstream gateway failed"));
        assert_eq!(body["error"]["code"], "copilot_bridge_atlas_upstream_error");
        assert_eq!(body["error"]["upstream_status"], 502);
    }

    #[test]
    fn codex_proxy_408_preserves_upstream_cause_without_claiming_a_local_timeout() {
        let error = ProxyError::UpstreamError {
            status: 408,
            body: Some(
                r#"{"error":{"message":"Timed out reading request body","code":"request_timeout","type":"timeout_error"}}"#
                    .to_string(),
            ),
        };
        let body = codex_proxy_error_json("GitHub Copilot", "gpt-6-astra", "/responses", &error);
        let message = body["error"]["message"].as_str().unwrap();
        assert!(message.contains("Upstream provider returned HTTP 408"));
        assert!(message.contains("Timed out reading request body"));
        assert!(message.contains("short handoff"));
        assert!(!message.contains("local proxy failed"));
        assert_eq!(body["error"]["code"], "request_timeout");
        assert_eq!(body["error"]["type"], "timeout_error");
        assert_eq!(body["error"]["upstream_status"], 408);
        assert_eq!(body["error"]["provider"], "GitHub Copilot");
        assert_eq!(body["error"]["model"], "gpt-6-astra");
        assert_eq!(body["error"]["endpoint"], "/responses");

        let local = codex_proxy_error_json(
            "GitHub Copilot",
            "gpt-6-astra",
            "/responses",
            &ProxyError::Timeout("Timed out waiting for response headers".into()),
        );
        assert!(local["error"]["upstream_status"].is_null());
        assert!(!local["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Upstream provider returned HTTP 408"));
    }

    #[test]
    fn codex_proxy_413_points_to_upstream_not_local_proxy() {
        // Simulate the 413 HTML page returned by the upstream channel provider nginx due to client_max_body_size
        // (See issue #666: Long context/large picture/large log hits the upstream volume limit)
        let error = ProxyError::UpstreamError {
            status: 413,
            body: Some(
                "<html>\r\n<head><title>413 Request Entity Too Large</title></head>\r\n\
                 <body>\r\n<center><h1>413 Request Entity Too Large</h1></center>\r\n\
                 <hr><center>nginx/1.29.6</center>\r\n</body>\r\n</html>"
                    .to_string(),
            ),
        };
        let body = codex_proxy_error_json("HCAI", "gpt-5.5", "/responses", &error);

        let message = body["error"]["message"].as_str().unwrap();
        // No longer misleads as "local proxy failed"
        assert!(!message.contains("Copilot Bridge Atlas local proxy failed"));
        // Clear pointer to upstream + volume exceeding limit + actionable guidance
        assert!(message.contains("413"));
        assert!(message.to_lowercase().contains("upstream"));
        assert!(message.contains("/compact"));
        // Key: Do not echo the entire nginx HTML to the user
        assert!(!message.contains("<html>"));
        assert!(!message.contains("nginx/1.29.6"));
        // Structured fields are still retained to facilitate programmatic consumption/UI rendering
        assert_eq!(body["error"]["upstream_status"], 413);
        assert_eq!(body["error"]["provider"], "HCAI");
        assert_eq!(body["error"]["model"], "gpt-5.5");
        assert_eq!(body["error"]["endpoint"], "/responses");
    }
}
