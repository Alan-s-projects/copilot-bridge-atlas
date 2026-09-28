//! Bounded HTTP diagnostics for requests sent to Copilot.
//!
//! The Atlas ID is also the request ID of a failed usage-history row. Never
//! interpolate request bodies, arbitrary headers, or upstream error text into
//! the summary log. Bounded failure snapshots are recorded separately under
//! the same Atlas ID in the file log and local database.

use super::{
    error_mapper::map_proxy_error_to_status, types::ReasoningEffort,
    upstream_response::ProxyResponse, usage::logger::UsageLogger, ProxyError,
};
use crate::{database::RequestDiagnosticDetail, Database};
use bytes::Bytes;
use futures::{stream::Stream, task::Poll};
use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    task::Context,
    time::Instant,
};

const MAX_ERROR_PREVIEW: usize = 4096;
const MAX_ERROR_JSON: usize = 32 * 1024;
const MAX_DIAGNOSTIC_IDS: usize = 6;
const MAX_SNAPSHOT_BODY: usize = 32 * 1024;
const MAX_SNAPSHOT_HEADERS: usize = 8 * 1024;

#[derive(Default, Clone)]
struct Details {
    endpoint: String,
    upstream_model: String,
    transport: &'static str,
    streaming: bool,
    applied_effort: String,
    outgoing_bytes: usize,
    input_items: usize,
    tools: usize,
    tools_state: &'static str,
    requested_tool_choice: &'static str,
    upstream_tool_choice: &'static str,
    parallel_tool_calls: &'static str,
    status: Option<u16>,
    declared_bytes: Option<u64>,
    encoding: Option<String>,
    upstream_ids: Vec<String>,
    body_summary: Option<String>,
    body_truncated: bool,
    preview: Vec<u8>,
    response_preview: Vec<u8>,
    snapshot: RequestDiagnosticDetail,
}

pub(crate) struct RequestDiagnostics {
    pub id: String,
    db: Arc<Database>,
    provider_id: String,
    requested_model: String,
    session_id: String,
    started: Instant,
    details: Mutex<Details>,
    response_bytes: AtomicU64,
    response_logged: AtomicBool,
    response_truncated: AtomicBool,
    failure_logged: AtomicBool,
}

impl RequestDiagnostics {
    pub fn new(
        db: Arc<Database>,
        provider_id: String,
        requested_model: String,
        session_id: String,
        started: Instant,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: format!("atlas_{}", uuid::Uuid::new_v4().simple()),
            db,
            provider_id,
            requested_model: safe_identifier(&requested_model, 96),
            session_id,
            started,
            details: Mutex::new(Details {
                transport: "unresolved",
                applied_effort: "none".into(),
                upstream_model: "unresolved".into(),
                tools_state: "unresolved",
                requested_tool_choice: "unresolved",
                upstream_tool_choice: "unresolved",
                parallel_tool_calls: "unresolved",
                ..Details::default()
            }),
            response_bytes: AtomicU64::new(0),
            response_logged: AtomicBool::new(false),
            response_truncated: AtomicBool::new(false),
            failure_logged: AtomicBool::new(false),
        })
    }

    fn details(&self) -> std::sync::MutexGuard<'_, Details> {
        self.details
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub fn request_shape(&self, endpoint: &str, body: &Value) {
        let mut details = self.details();
        details.endpoint = safe_path(endpoint);
        details.streaming = body.get("stream").and_then(Value::as_bool) == Some(true);
        details.input_items = body
            .get("input")
            .or_else(|| body.get("messages"))
            .map(|value| value.as_array().map_or(1, Vec::len))
            .unwrap_or(0);
        details.tools = body
            .get("tools")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        details.requested_tool_choice = tool_choice_kind(body.get("tool_choice"));
    }

    pub fn outgoing(
        &self,
        model: Option<&str>,
        transport: &'static str,
        streaming: bool,
        effort: &ReasoningEffort,
        bytes: usize,
        body: &Value,
        headers: &HeaderMap,
        body_bytes: &[u8],
    ) {
        let mut details = self.details();
        details.upstream_model = model
            .map(|value| safe_identifier(value, 96))
            .unwrap_or_else(|| "unknown".into());
        details.transport = transport;
        details.streaming = streaming;
        details.applied_effort = effort
            .applied
            .as_deref()
            .map(|value| safe_identifier(value, 16))
            .unwrap_or_else(|| "none".into());
        details.outgoing_bytes = bytes;
        details.input_items = body
            .get("input")
            .or_else(|| body.get("messages"))
            .map(|value| value.as_array().map_or(1, Vec::len))
            .unwrap_or(0);
        details.tools = body
            .get("tools")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        details.tools_state = tools_state(body.get("tools"));
        details.upstream_tool_choice = tool_choice_kind(body.get("tool_choice"));
        details.parallel_tool_calls = parallel_tool_calls_kind(body.get("parallel_tool_calls"));
        details.snapshot.request_headers = Some(snapshot_headers(headers.iter()));
        details.snapshot.request_body = Some(snapshot_body(body_bytes, body_bytes.len()));
    }

    fn response_headers(&self, status: u16, headers: &HeaderMap) {
        let mut details = self.details();
        details.status = Some(status);
        details.declared_bytes = headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        details.encoding = headers
            .get(http::header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .map(|value| safe_identifier(value, 24));
        details.snapshot.response_headers = Some(snapshot_headers(headers.iter()));
        details.upstream_ids = [
            "x-request-id",
            "request-id",
            "x-correlation-id",
            "x-github-request-id",
            "x-github-correlation-id",
            "x-ms-request-id",
            "x-ms-correlation-request-id",
            "x-copilot-request-id",
            "cf-ray",
            "traceparent",
        ]
        .into_iter()
        .filter_map(|name| {
            let value = headers.get(name)?.to_str().ok()?;
            safe_correlation_id(value).map(|value| format!("{name}:{value}"))
        })
        .take(MAX_DIAGNOSTIC_IDS)
        .collect();
    }

    pub fn observe(self: &Arc<Self>, response: ProxyResponse) -> ProxyResponse {
        let status = response.status();
        let headers = response.headers().clone();
        self.response_headers(status.as_u16(), &headers);
        let stream = ObservedBody {
            inner: response.bytes_stream(),
            diagnostics: self.clone(),
            done: false,
        };
        ProxyResponse::streamed(status, headers, stream)
    }

    fn add_bytes(&self, bytes: &Bytes) {
        self.response_bytes
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        let mut details = self.details();
        let remaining = MAX_SNAPSHOT_BODY.saturating_sub(details.response_preview.len());
        details
            .response_preview
            .extend_from_slice(&bytes[..remaining.min(bytes.len())]);
        if details.status.is_some_and(|status| status >= 400) {
            let remaining = MAX_ERROR_PREVIEW.saturating_sub(details.preview.len());
            details
                .preview
                .extend_from_slice(&bytes[..remaining.min(bytes.len())]);
        }
    }

    fn completed(&self, truncated: bool, outcome: &'static str) {
        if self.response_logged.swap(true, Ordering::AcqRel) {
            return;
        }
        self.response_truncated.store(truncated, Ordering::Relaxed);
        let details = self.details().clone();
        log::info!(
            target: "atlas_http",
            "atlas_id={} status={} elapsed_ms={} endpoint={} requested_model={} upstream_model={} transport={} streaming={} applied_effort={} outgoing_bytes={} input_items={} tools={} tools_state={} requested_tool_choice={} upstream_tool_choice={} parallel_tool_calls={} response_bytes={} declared_bytes={} truncated={} outcome={} upstream_ids={}",
            self.id,
            details.status.map_or_else(|| "none".into(), |value| value.to_string()),
            self.started.elapsed().as_millis(),
            details.endpoint,
            self.requested_model,
            details.upstream_model,
            details.transport,
            details.streaming,
            details.applied_effort,
            details.outgoing_bytes,
            details.input_items,
            details.tools,
            details.tools_state,
            details.requested_tool_choice,
            details.upstream_tool_choice,
            details.parallel_tool_calls,
            self.response_bytes.load(Ordering::Relaxed),
            details.declared_bytes.map_or_else(|| "none".into(), |value| value.to_string()),
            truncated,
            outcome,
            if details.upstream_ids.is_empty() {
                "none".into()
            } else {
                details.upstream_ids.join("|")
            }
        );
    }

    pub fn error_body(&self, bytes: &[u8], decoded: bool) {
        let mut details = self.details();
        details.body_truncated = bytes.len() > MAX_ERROR_JSON;
        details.body_summary = Some(summarize_body(
            bytes,
            details.encoding.is_some() && !decoded,
            bytes.len() > MAX_ERROR_JSON,
        ));
        let mut body = snapshot_body(bytes, bytes.len());
        if let Some(encoding) = &details.encoding {
            body = if decoded {
                format!("[decoded from {encoding}]\n{body}")
            } else {
                format!("[encoded as {encoding}]\n{body}")
            };
        }
        details.snapshot.response_body = Some(body);
    }

    pub fn failed_event(&self, event: &Value, raw_data: Option<&str>) {
        let error = event
            .pointer("/response/error")
            .or_else(|| event.get("error"));
        let summary = error
            .map(summarize_error_value)
            .unwrap_or_else(|| "kind=sse message=response.failed".into());
        let serialized = raw_data
            .map(str::to_owned)
            .unwrap_or_else(|| event.to_string());
        let mut details = self.details();
        details.body_summary = Some(summary);
        details.snapshot.response_body = Some(format!(
            "event: response.failed\ndata: {}",
            snapshot_body(serialized.as_bytes(), serialized.len())
        ));
        drop(details);
        self.record_failure("stream", 502);
    }

    pub fn proxy_failure(&self, stage: &'static str, error: &ProxyError) {
        // The client receives the original error. Only the private diagnostic
        // and history row use these bounded, sanitized categories.
        self.completed(self.details().status.is_some(), stage);
        self.record_failure(stage, map_proxy_error_to_status(error));
    }

    pub fn stream_failure(&self) {
        self.record_failure("stream_read", 502);
    }

    fn record_failure(&self, stage: &'static str, history_status: u16) {
        if self.failure_logged.swap(true, Ordering::AcqRel) {
            return;
        }
        let details = self.details().clone();
        let summary = details.body_summary.unwrap_or_else(|| {
            summarize_body(
                &details.preview,
                details.encoding.is_some(),
                details.preview.len() >= MAX_ERROR_PREVIEW,
            )
        });
        let ids = if details.upstream_ids.is_empty() {
            "none".into()
        } else {
            details.upstream_ids.join("|")
        };
        let summary = if summary == "kind=empty" {
            format!("kind=empty stage={stage}")
        } else {
            summary
        };
        log::warn!(
            target: "atlas_http",
            "atlas_id={} stage={} upstream_status={} history_status={} endpoint={} requested_model={} upstream_model={} transport={} streaming={} outgoing_bytes={} input_items={} tools={} tools_state={} requested_tool_choice={} upstream_tool_choice={} parallel_tool_calls={} response_bytes={} truncated={} body_truncated={} upstream_ids={} body={}",
            self.id,
            stage,
            details.status.map_or_else(|| "none".into(), |value| value.to_string()),
            history_status,
            details.endpoint,
            self.requested_model,
            details.upstream_model,
            details.transport,
            details.streaming,
            details.outgoing_bytes,
            details.input_items,
            details.tools,
            details.tools_state,
            details.requested_tool_choice,
            details.upstream_tool_choice,
            details.parallel_tool_calls,
            self.response_bytes.load(Ordering::Relaxed),
            self.response_truncated.load(Ordering::Relaxed),
            details.body_truncated || details.preview.len() >= MAX_ERROR_PREVIEW,
            ids,
            summary
        );

        let logger = UsageLogger::new(&self.db);
        let history_recorded = logger
            .log_error_with_context(
                self.id.clone(),
                self.provider_id.clone(),
                "codex".into(),
                self.requested_model.clone(),
                history_status,
                format!(
                    "Atlas diagnostic {}: tools={} tools_state={} requested_tool_choice={} upstream_tool_choice={} parallel_tool_calls={} {summary}",
                    self.id,
                    details.tools,
                    details.tools_state,
                    details.requested_tool_choice,
                    details.upstream_tool_choice,
                    details.parallel_tool_calls,
                ),
                self.started.elapsed().as_millis() as u64,
                details.streaming,
                Some(self.session_id.clone()),
                None,
                ReasoningEffort {
                    requested: None,
                    applied: (details.applied_effort != "none").then_some(details.applied_effort),
                },
            )
            .is_ok();
        if !history_recorded {
            log::warn!(target: "atlas_http", "atlas_id={} failed to record diagnostic history row", self.id);
        }
        let mut snapshot = details.snapshot;
        snapshot.upstream_status = details.status;
        snapshot.failure_stage = Some(stage.to_owned());
        if snapshot.response_body.is_none() && !details.response_preview.is_empty() {
            snapshot.response_body = Some(snapshot_body(
                &details.response_preview,
                self.response_bytes.load(Ordering::Relaxed) as usize,
            ));
        }
        for (label, value) in [
            ("request_headers", snapshot.request_headers.as_deref()),
            ("request_body", snapshot.request_body.as_deref()),
            ("response_headers", snapshot.response_headers.as_deref()),
            ("response_body", snapshot.response_body.as_deref()),
        ] {
            let escaped = serde_json::to_string(value.unwrap_or("[not available]"))
                .unwrap_or_else(|_| "\"[serialization failed]\"".into());
            log::warn!(target: "atlas_http", "atlas_id={} capture.{label}={escaped}", self.id);
        }
        if !history_recorded {
            return;
        }
        if let Err(error) = self.db.save_request_diagnostics(&self.id, &snapshot) {
            log::warn!(target: "atlas_http", "atlas_id={} failed to record request/response snapshot: {error}", self.id);
        }
    }
}

fn snapshot_body(bytes: &[u8], total_bytes: usize) -> String {
    if bytes.len() > MAX_SNAPSHOT_BODY {
        let half = MAX_SNAPSHOT_BODY / 2;
        return format!(
            "{}\n[omitted {} bytes of {total_bytes}]\n{}",
            render_snapshot_bytes(&bytes[..half]),
            total_bytes.saturating_sub(MAX_SNAPSHOT_BODY),
            render_snapshot_bytes(&bytes[bytes.len() - half..])
        );
    }
    let mut output = render_snapshot_bytes(bytes);
    if total_bytes > bytes.len() {
        output.push_str(&format!(
            "\n[truncated after {} bytes of {total_bytes}]",
            bytes.len()
        ));
    }
    output
}

fn render_snapshot_bytes(sample: &[u8]) -> String {
    match std::str::from_utf8(sample) {
        Ok(text)
            if !text
                .chars()
                .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')) =>
        {
            text.to_owned()
        }
        _ => {
            use std::fmt::Write;
            let mut hex = String::with_capacity(sample.len() * 2 + 22);
            hex.push_str("[non-text body, hex]\n");
            for byte in sample {
                write!(&mut hex, "{byte:02x}").expect("writing to a String cannot fail");
            }
            hex
        }
    }
}

fn snapshot_headers<'a>(
    headers: impl Iterator<Item = (&'a HeaderName, &'a HeaderValue)>,
) -> String {
    let mut output = String::new();
    for (name, value) in headers {
        let name = name.as_str();
        let value = value.to_str().unwrap_or("[non-text header]");
        let lower = name.to_ascii_lowercase();
        let secret_name = [
            "authorization",
            "cookie",
            "token",
            "secret",
            "api-key",
            "api_key",
        ]
        .iter()
        .any(|part| lower.contains(part));
        let value = if secret_name || looks_sensitive(value) {
            "[redacted]"
        } else {
            value
        };
        let line = format!("{name}: {value}\n");
        if output.len() + line.len() > MAX_SNAPSHOT_HEADERS {
            output.push_str("[headers truncated]");
            break;
        }
        output.push_str(&line);
    }
    if output.is_empty() {
        "(no headers)".into()
    } else {
        output.trim_end().to_owned()
    }
}

struct ObservedBody {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    diagnostics: Arc<RequestDiagnostics>,
    done: bool,
}

impl Stream for ObservedBody {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                self.diagnostics.add_bytes(&bytes);
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(error))) => {
                self.done = true;
                self.diagnostics.completed(true, "read_error");
                self.diagnostics.stream_failure();
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                self.done = true;
                self.diagnostics.completed(false, "complete");
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for ObservedBody {
    fn drop(&mut self) {
        if !self.done {
            self.diagnostics.completed(true, "incomplete");
        }
    }
}

fn safe_path(endpoint: &str) -> String {
    let path = endpoint.split('?').next().unwrap_or("");
    if path.is_empty()
        || path.len() > 160
        || !path.starts_with('/')
        || path.split('/').any(|segment| matches!(segment, "." | ".."))
        || !path
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.'))
    {
        return "[invalid-path]".into();
    }
    path.to_owned()
}

fn tools_state(value: Option<&Value>) -> &'static str {
    match value {
        None => "absent",
        Some(Value::Null) => "null",
        Some(Value::Array(tools)) if tools.is_empty() => "empty",
        Some(Value::Array(_)) => "nonempty",
        Some(_) => "other",
    }
}

fn tool_choice_kind(value: Option<&Value>) -> &'static str {
    match value {
        None => "absent",
        Some(Value::String(choice)) => match choice.as_str() {
            "auto" => "auto",
            "none" => "none",
            "required" => "required",
            _ => "other",
        },
        Some(Value::Object(_)) => "object",
        Some(Value::Null) => "null",
        Some(_) => "other",
    }
}

fn parallel_tool_calls_kind(value: Option<&Value>) -> &'static str {
    match value {
        None => "absent",
        Some(Value::Bool(true)) => "true",
        Some(Value::Bool(false)) => "false",
        Some(_) => "other",
    }
}

fn safe_identifier(value: &str, limit: usize) -> String {
    if value.is_empty()
        || value.len() > limit
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '/' | ':'))
        || looks_sensitive(value)
    {
        return "[redacted]".into();
    }
    value.to_owned()
}

fn safe_correlation_id(value: &str) -> Option<String> {
    (value.len() <= 128
        && !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '/'))
        && !looks_sensitive(value))
    .then(|| value.to_owned())
}

fn looks_sensitive(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "bearer",
        "sk-",
        "gho_",
        "ghp_",
        "ghu_",
        "ghs_",
        "github_pat_",
        "api_key",
        "password",
        "cookie",
        "secret",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn safe_token(value: &str, limit: usize) -> Option<String> {
    let value = value.trim();
    if (1..=limit).contains(&value.len())
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && !looks_sensitive(value)
    {
        Some(value.to_owned())
    } else {
        None
    }
}

fn safe_param_path(value: &str) -> Option<String> {
    let valid = (1..=128).contains(&value.len())
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '[' | ']'))
        && !looks_sensitive(value)
        && value.split('.').all(|part| {
            let (name, index) = part.split_once('[').unwrap_or((part, ""));
            !name.is_empty()
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                && (index.is_empty()
                    || index.strip_suffix(']').is_some_and(|index| {
                        !index.is_empty() && index.chars().all(|ch| ch.is_ascii_digit())
                    }))
        });
    valid.then(|| value.to_owned())
}

fn diagnostic_number_name(path: &str) -> bool {
    let name = path.rsplit('.').next().unwrap_or(path);
    let name = name.split('[').next().unwrap_or(name);
    matches!(
        name,
        "count"
            | "limit"
            | "prompt_tokens"
            | "input_tokens"
            | "output_tokens"
            | "max_tokens"
            | "max_prompt_tokens"
            | "max_output_tokens"
    )
}

fn collect_numbers(value: &Value, path: &str, output: &mut Vec<String>) {
    match value {
        Value::Number(number) if diagnostic_number_name(path) => {
            output.push(format!("{path}={number}"));
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate().take(8) {
                collect_numbers(item, &format!("{path}[{index}]"), output);
            }
        }
        Value::Object(fields) => {
            for (key, child) in fields.iter().take(12) {
                if let Some(key) = safe_token(key, 48) {
                    let next = if path.is_empty() {
                        key
                    } else {
                        format!("{path}.{key}")
                    };
                    collect_numbers(child, &next, output);
                }
            }
        }
        _ => {}
    }
    output.truncate(12);
}

fn grok_prompt_limit(message: &str) -> Option<(u64, u64)> {
    let words: Vec<_> = message.split_whitespace().collect();
    let prompt = words
        .windows(4)
        .position(|phrase| phrase == ["prompt", "token", "count", "of"])?;
    let count = words.get(prompt + 4)?.parse().ok()?;
    if words.get(prompt + 5..prompt + 8) != Some(&["exceeds", "the", "limit"]) {
        return None;
    }
    if words.get(prompt + 8) != Some(&"of") {
        return None;
    }
    let limit = words.get(prompt + 9)?.trim_end_matches('.').parse().ok()?;
    Some((count, limit))
}

fn collect_structure(value: &Value, path: &str, output: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields.iter().take(16) {
                let key = safe_token(key, 48).unwrap_or_else(|| "invalid_key".into());
                let next = if path.is_empty() {
                    key
                } else {
                    format!("{path}.{key}")
                };
                output.push(format!("{next}:{}", json_type_name(child)));
                if output.len() < 24 && (child.is_object() || child.is_array()) {
                    collect_structure(child, &next, output);
                }
            }
        }
        Value::Array(items) => {
            output.push(format!(
                "{path}[]:{}",
                items.first().map(json_type_name).unwrap_or("empty")
            ));
            if let Some(item) = items.first() {
                collect_structure(item, &format!("{path}[]"), output);
            }
        }
        _ => {}
    }
    output.truncate(24);
}

fn summarize_error_value(value: &Value) -> String {
    let error = value.get("error").unwrap_or(value);
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .filter(|value| !looks_sensitive(value))
        .and_then(|value| safe_token(value, 96))
        .unwrap_or_else(|| "none".into());
    let kind = error
        .get("type")
        .and_then(Value::as_str)
        .and_then(|value| safe_token(value, 96))
        .unwrap_or_else(|| "none".into());
    let param = error
        .get("param")
        .and_then(Value::as_str)
        .and_then(safe_param_path)
        .unwrap_or_else(|| "none".into());
    let mut numbers = Vec::new();
    collect_numbers(error, "", &mut numbers);
    if numbers.is_empty() {
        if let Some((count, limit)) = error
            .get("message")
            .and_then(Value::as_str)
            .and_then(grok_prompt_limit)
        {
            numbers.push(format!("prompt_tokens={count}"));
            numbers.push(format!("limit={limit}"));
        }
    }
    let mut structure = Vec::new();
    collect_structure(error, "", &mut structure);
    format!(
        "kind=json code={code} type={kind} param={param} numbers={} fields={}",
        if numbers.is_empty() {
            "none".into()
        } else {
            numbers.join(",")
        },
        if structure.is_empty() {
            "none".into()
        } else {
            structure.join(",")
        }
    )
}

fn summarize_body(body: &[u8], compressed: bool, truncated: bool) -> String {
    if body.is_empty() {
        return "kind=empty".into();
    }
    if compressed {
        return format!("kind=compressed bytes={}", body.len());
    }
    let sample = &body[..body.len().min(MAX_ERROR_JSON)];
    if !truncated && body.len() <= MAX_ERROR_JSON {
        if let Ok(value) = serde_json::from_slice::<Value>(sample) {
            return summarize_error_value(&value);
        }
    }
    let kind = if sample.iter().any(|byte| {
        !byte.is_ascii() || byte.is_ascii_control() && !matches!(byte, b'\n' | b'\r' | b'\t')
    }) {
        "binary"
    } else if sample.starts_with(b"<") {
        "html"
    } else if truncated {
        "truncated"
    } else {
        "text"
    };
    format!("kind={kind} bytes={}", body.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use http::{HeaderValue, StatusCode};
    use std::sync::OnceLock;

    struct CapturedLogger(Mutex<Vec<(log::Level, String)>>);

    impl log::Log for CapturedLogger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.target() == "atlas_http"
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                self.0
                    .lock()
                    .unwrap()
                    .push((record.level(), record.args().to_string()));
            }
        }
        fn flush(&self) {}
    }

    fn captured_logger() -> &'static CapturedLogger {
        static LOGGER: OnceLock<CapturedLogger> = OnceLock::new();
        let logger = LOGGER.get_or_init(|| CapturedLogger(Mutex::new(Vec::new())));
        static INSTALLED: OnceLock<()> = OnceLock::new();
        INSTALLED.get_or_init(|| {
            log::set_logger(logger).expect("test logger must be installed only once");
        });
        logger.0.lock().unwrap().clear();
        log::set_max_level(log::LevelFilter::Info);
        logger
    }

    fn make_diagnostics(db: Arc<Database>) -> Arc<RequestDiagnostics> {
        let diagnostics = RequestDiagnostics::new(
            db,
            "copilot".into(),
            "gpt-6-astra".into(),
            "test-session".into(),
            Instant::now(),
        );
        diagnostics.request_shape(
            "/responses?access_token=do-not-log",
            &serde_json::json!({"input":[{"role":"user","content":"secret prompt"}],"tools":[{"name":"tool"}],"stream":false}),
        );
        diagnostics.outgoing(
            Some("gpt-6-astra"),
            "responses",
            false,
            &ReasoningEffort {
                requested: Some("high".into()),
                applied: Some("high".into()),
            },
            712,
            &serde_json::json!({"input":[{}],"tools":[{}]}),
            &HeaderMap::new(),
            b"",
        );
        diagnostics
    }

    #[test]
    fn structured_failure_summaries_keep_codes_and_counts_without_freeform_text() {
        let summary = summarize_body(br#"{"error":{"message":"prompt contains secret prompt","code":"model_max_prompt_tokens_exceeded","type":"invalid_request_error","param":"input","prompt_tokens":390003,"limit":372000,"unexpected":{"reason":"secret tool output","count":2},"otp":123456}}"#, false, false);
        assert!(summary.contains("code=model_max_prompt_tokens_exceeded"));
        assert!(summary.contains("type=invalid_request_error"));
        assert!(summary.contains("param=input"));
        assert!(summary.contains("prompt_tokens=390003"));
        assert!(summary.contains("limit=372000"));
        assert!(summary.contains("unexpected.count=2"));
        assert!(summary.contains("otp:number"));
        assert!(!summary.contains("otp=123456"));
        assert!(summary.contains("unexpected:object"));
        assert!(!summary.contains("secret prompt"));
        assert!(!summary.contains("secret tool output"));

        let observed = summarize_body(br#"{"error":{"message":"prompt token count of 390003 exceeds the limit of 372000","code":"model_max_prompt_tokens_exceeded","type":"invalid_request_error","param":null}}"#, false, false);
        assert!(observed.contains("code=model_max_prompt_tokens_exceeded"));
        assert!(observed.contains("prompt_tokens=390003"));
        assert!(observed.contains("limit=372000"));
        assert!(!observed.contains("prompt token count"));

        assert_eq!(
            summarize_body(b"Bad Request\nsecret prompt", false, false),
            "kind=text bytes=25"
        );
        assert_eq!(
            summarize_body(b"Bad Request\n", false, false),
            "kind=text bytes=12"
        );
        assert_eq!(summarize_body(b"", false, false), "kind=empty");
        assert_eq!(
            summarize_body(b"compressed secret", true, false),
            "kind=compressed bytes=17"
        );
        assert_eq!(
            summarize_body(b"<html>secret prompt</html>", false, false),
            "kind=html bytes=26"
        );
        assert_eq!(
            summarize_body(b"{not json secret", false, false),
            "kind=text bytes=16"
        );
        assert!(summarize_body(
            br#"{"error":{"code":"Bearer secret","param":"input.secret"}}"#,
            false,
            false
        )
        .contains("code=none"));
    }

    #[test]
    fn only_allowlisted_paths_and_correlation_ids_enter_the_log() {
        assert_eq!(safe_path("/responses?api_key=secret"), "/responses");
        assert_eq!(safe_path("/responses/../token"), "[invalid-path]");
        assert!(safe_correlation_id("request-123").is_some());
        assert!(safe_correlation_id("Bearer secret").is_none());
        assert!(safe_correlation_id("gho_secret").is_none());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn tool_selection_diagnostics_show_the_sent_shape_without_values() {
        let logger = captured_logger();
        let db = Arc::new(Database::memory().unwrap());
        let diagnostics = RequestDiagnostics::new(
            db.clone(),
            "copilot".into(),
            "grok-4.7".into(),
            "test-session".into(),
            Instant::now(),
        );
        diagnostics.request_shape(
            "/responses",
            &serde_json::json!({"input":[{"content":"private prompt"}],"tools":[],"tool_choice":"auto","parallel_tool_calls":true}),
        );
        diagnostics.outgoing(
            Some("grok-4.7"),
            "compatible_responses",
            true,
            &ReasoningEffort::default(),
            100,
            &serde_json::json!({"input":[{"content":"private prompt"}],"tools":[],"parallel_tool_calls":null}),
            &HeaderMap::new(),
            b"",
        );
        let response = diagnostics.observe(ProxyResponse::buffered(
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            Bytes::from_static(b"Bad Request\n"),
        ));
        let body = response.bytes_with_limit(1024).await.unwrap();
        diagnostics.error_body(&body, true);
        diagnostics.proxy_failure(
            "upstream",
            &ProxyError::UpstreamError {
                status: 400,
                body: Some("Bad Request\n".into()),
            },
        );
        let entries = logger.0.lock().unwrap().clone();
        assert!(entries[1].1.contains("tools=0 tools_state=empty requested_tool_choice=auto upstream_tool_choice=absent parallel_tool_calls=other"));
        assert!(!entries[1].1.contains("private prompt"));
        assert!(!entries[1].1.contains("Bad Request"));
        assert!(entries.iter().any(|(_, line)| {
            line.contains("capture.response_body=") && line.contains("Bad Request")
        }));
        let history = db.get_request_logs(&Default::default(), 0, 10).unwrap();
        let saved = history.data[0].error_message.as_deref().unwrap();
        assert!(saved.contains("requested_tool_choice=auto upstream_tool_choice=absent"));
        assert!(!saved.contains("private prompt"));
        assert!(!saved.contains("Bad Request"));

        assert_eq!(
            tool_choice_kind(Some(&Value::String("Bearer private".into()))),
            "other"
        );
        assert_eq!(
            tool_choice_kind(Some(&serde_json::json!({"name":"private"}))),
            "object"
        );
        assert_eq!(tools_state(Some(&serde_json::json!([{}]))), "nonempty");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn info_and_warn_share_a_failure_history_id_without_changing_body() {
        let logger = captured_logger();
        let db = Arc::new(Database::memory().unwrap());
        let diagnostics = make_diagnostics(db.clone());
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", HeaderValue::from_static("upstream-123"));
        headers.insert("authorization", HeaderValue::from_static("Bearer private"));
        let body = Bytes::from_static(
            br#"{"error":{"message":"prompt token count of 390003 exceeds the limit of 372000","code":"model_max_prompt_tokens_exceeded","type":"invalid_request_error","param":"input"}}"#,
        );
        let returned = diagnostics
            .observe(ProxyResponse::buffered(
                StatusCode::BAD_REQUEST,
                headers,
                body.clone(),
            ))
            .bytes_stream()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(returned.concat(), body);
        diagnostics.error_body(&body, true);
        diagnostics.proxy_failure(
            "upstream",
            &ProxyError::UpstreamError {
                status: 400,
                body: Some(String::from_utf8(body.to_vec()).unwrap()),
            },
        );

        let entries = logger.0.lock().unwrap().clone();
        assert_eq!(entries.len(), 6);
        assert_eq!(entries[0].0, log::Level::Info);
        assert_eq!(entries[1].0, log::Level::Warn);
        assert!(entries[1]
            .1
            .contains("code=model_max_prompt_tokens_exceeded"));
        assert!(!entries[1].1.contains("secret prompt"));
        assert!(entries[1].1.contains("model_max_prompt_tokens_exceeded"));
        for (_, line) in &entries {
            assert_eq!(line.lines().count(), 1);
            assert!(line.contains(&diagnostics.id));
            assert!(!line.contains("Bearer private"));
            assert!(!line.contains("access_token"));
        }
        assert!(entries[0].1.contains("upstream-123"));
        assert!(entries[1].1.contains("upstream-123"));
        assert!(entries.iter().any(|(_, line)| {
            line.contains("capture.response_headers=") && line.contains("authorization: [redacted]")
        }));
        assert!(entries[0].1.contains("status=400"));
        assert!(entries[0].1.contains("outgoing_bytes=712"));
        assert!(entries[0].1.contains("input_items=1 tools=1"));
        assert!(entries[1]
            .1
            .contains("code=model_max_prompt_tokens_exceeded"));
        let history = db.get_request_logs(&Default::default(), 0, 10).unwrap();
        assert_eq!(history.total, 1);
        assert_eq!(history.data[0].request_id, diagnostics.id);
        assert_eq!(history.data[0].status_code, 400);
        assert!(!history.data[0]
            .error_message
            .as_deref()
            .unwrap()
            .contains("private-token"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn plain_text_upstream_400_keeps_the_summary_safe_and_captures_the_body() {
        let logger = captured_logger();
        let db = Arc::new(Database::memory().unwrap());
        let diagnostics = make_diagnostics(db.clone());
        let body = Bytes::from_static(b"Bad Request\n");
        let returned = diagnostics
            .observe(ProxyResponse::buffered(
                StatusCode::BAD_REQUEST,
                HeaderMap::new(),
                body.clone(),
            ))
            .bytes_with_limit(1024)
            .await
            .unwrap();
        assert_eq!(returned, body);
        diagnostics.error_body(&body, true);
        diagnostics.proxy_failure(
            "upstream",
            &ProxyError::UpstreamError {
                status: 400,
                body: Some("Bad Request\n".into()),
            },
        );

        let entries = logger.0.lock().unwrap().clone();
        let warning = &entries[1].1;
        assert!(warning.contains("upstream_status=400"));
        assert!(warning.contains("body=kind=text bytes=12"));
        assert!(!warning.contains("Bad Request"));
        assert!(!warning.contains("raw_body="));
        assert!(entries.iter().any(|(_, line)| {
            line.contains("capture.response_body=") && line.contains("Bad Request")
        }));
        let history = db.get_request_logs(&Default::default(), 0, 10).unwrap();
        assert_eq!(history.data[0].request_id, diagnostics.id);
        assert_eq!(history.data[0].status_code, 400);
        let saved = history.data[0].error_message.as_deref().unwrap();
        assert!(saved.contains("kind=text bytes=12"));
        assert!(!saved.contains("Bad Request"));
        assert!(!saved.contains("raw_body="));
        let detail = db
            .get_request_diagnostics(&diagnostics.id)
            .unwrap()
            .unwrap();
        assert_eq!(detail.upstream_status, Some(400));
        assert_eq!(detail.failure_stage.as_deref(), Some("upstream"));
        assert_eq!(detail.response_body.as_deref(), Some("Bad Request\n"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn compressed_and_large_error_bodies_keep_sizes_and_redact_content() {
        let logger = captured_logger();
        let db = Arc::new(Database::memory().unwrap());
        let diagnostics = make_diagnostics(db);
        let mut headers = HeaderMap::new();
        headers.insert("content-encoding", HeaderValue::from_static("gzip"));
        let encoded = Bytes::from_static(b"\x1f\x8bprivate compressed content");
        let returned = diagnostics
            .observe(ProxyResponse::buffered(
                StatusCode::BAD_REQUEST,
                headers,
                encoded.clone(),
            ))
            .bytes_with_limit(1024)
            .await
            .unwrap();
        assert_eq!(returned, encoded);
        diagnostics.error_body(&returned, false);
        diagnostics.proxy_failure(
            "upstream",
            &ProxyError::UpstreamError {
                status: 400,
                body: None,
            },
        );
        let entries = logger.0.lock().unwrap().clone();
        assert!(entries[0]
            .1
            .contains(&format!("response_bytes={}", encoded.len())));
        assert!(entries[1].1.contains("kind=compressed"));
        assert!(!entries[1].1.contains("private compressed"));

        logger.0.lock().unwrap().clear();
        let diagnostics = make_diagnostics(Arc::new(Database::memory().unwrap()));
        let large = vec![b'p'; MAX_ERROR_JSON + 20];
        let _ = diagnostics
            .observe(ProxyResponse::buffered(
                StatusCode::BAD_REQUEST,
                HeaderMap::new(),
                Bytes::from(large.clone()),
            ))
            .bytes_with_limit(large.len() + 1)
            .await
            .unwrap();
        diagnostics.error_body(&large, true);
        diagnostics.proxy_failure(
            "upstream",
            &ProxyError::UpstreamError {
                status: 400,
                body: None,
            },
        );
        let entries = logger.0.lock().unwrap().clone();
        assert!(entries[1].1.contains("body_truncated=true"));
        assert!(entries[1].1.contains("kind=truncated"));
        assert!(!entries[1].1.contains("pppppp"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn late_stream_read_failure_uses_the_same_history_id() {
        let logger = captured_logger();
        let db = Arc::new(Database::memory().unwrap());
        let diagnostics = make_diagnostics(db.clone());
        let response = ProxyResponse::streamed(
            StatusCode::OK,
            HeaderMap::new(),
            futures::stream::iter(vec![
                Ok(Bytes::from_static(b"event: response.created\n\n")),
                Err(std::io::Error::other("secret tool output")),
            ]),
        );
        let mut stream = diagnostics.observe(response).bytes_stream();
        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            Bytes::from_static(b"event: response.created\n\n")
        );
        assert!(stream.next().await.unwrap().is_err());
        assert_eq!(
            db.get_request_logs(&Default::default(), 0, 10)
                .unwrap()
                .data[0]
                .request_id,
            diagnostics.id
        );
        let entries = logger.0.lock().unwrap().clone();
        assert!(entries
            .iter()
            .any(|(level, line)| *level == log::Level::Warn && line.contains("stage=stream_read")));
        assert!(!entries
            .iter()
            .any(|(_, line)| line.contains("secret tool output")));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn response_conversion_and_sse_failure_events_keep_the_atlas_id() {
        let logger = captured_logger();
        let db = Arc::new(Database::memory().unwrap());
        let diagnostics = make_diagnostics(db.clone());
        let body = Bytes::from_static(b"{invalid response with secret prompt");
        let returned = diagnostics
            .observe(ProxyResponse::buffered(
                StatusCode::OK,
                HeaderMap::new(),
                body.clone(),
            ))
            .bytes_with_limit(1024)
            .await
            .unwrap();
        assert_eq!(returned, body);
        diagnostics.proxy_failure(
            "chat_conversion",
            &ProxyError::TransformError("secret prompt".into()),
        );
        let history = db.get_request_logs(&Default::default(), 0, 10).unwrap();
        assert_eq!(history.data[0].request_id, diagnostics.id);
        assert_eq!(history.data[0].status_code, 422);
        let entries = logger.0.lock().unwrap().clone();
        assert!(entries[1].1.contains("stage=chat_conversion"));
        assert!(!entries[1].1.contains("private-token"));

        logger.0.lock().unwrap().clear();
        let db = Arc::new(Database::memory().unwrap());
        let diagnostics = make_diagnostics(db.clone());
        let mut request_headers = HeaderMap::new();
        request_headers.insert("authorization", HeaderValue::from_static("Bearer private"));
        request_headers.insert("content-type", HeaderValue::from_static("application/json"));
        diagnostics.outgoing(
            Some("gpt-6-astra"),
            "responses",
            true,
            &ReasoningEffort::default(),
            25,
            &serde_json::json!({"input":[]}),
            &request_headers,
            br#"{"input":"example request"}"#,
        );
        let mut response_headers = HeaderMap::new();
        response_headers.insert(
            "content-type",
            HeaderValue::from_static("text/event-stream"),
        );
        let _ = diagnostics
            .observe(ProxyResponse::buffered(
                StatusCode::OK,
                response_headers,
                Bytes::from_static(b"event: response.created\n\n"),
            ))
            .bytes_with_limit(1024)
            .await
            .unwrap();
        let failure_event = serde_json::json!({
            "type": "response.failed",
            "response": {"error": {"message": "secret tool output", "code": "model_max_prompt_tokens_exceeded", "param": "input", "prompt_tokens": 390003}}
        });
        let raw_event = failure_event.to_string();
        diagnostics.failed_event(&failure_event, Some(&raw_event));
        let history = db.get_request_logs(&Default::default(), 0, 10).unwrap();
        assert_eq!(history.data[0].request_id, diagnostics.id);
        let entries = logger.0.lock().unwrap().clone();
        assert!(entries
            .iter()
            .any(|(_, line)| line.contains("code=model_max_prompt_tokens_exceeded")));
        assert!(entries
            .iter()
            .any(|(_, line)| line.contains("prompt_tokens=390003")));
        assert!(!entries[1].1.contains("secret tool output"));
        assert!(entries.iter().any(|(_, line)| {
            line.contains("capture.response_body=") && line.contains("secret tool output")
        }));
        assert!(entries.iter().any(|(_, line)| {
            line.contains("capture.request_body=") && line.contains("example request")
        }));
        assert!(!entries
            .iter()
            .any(|(_, line)| line.contains("Bearer private")));
        let detail = db
            .get_request_diagnostics(&diagnostics.id)
            .unwrap()
            .unwrap();
        assert_eq!(detail.upstream_status, Some(200));
        assert_eq!(detail.failure_stage.as_deref(), Some("stream"));
        assert!(detail.response_body.unwrap().contains("secret tool output"));
        assert!(detail
            .request_headers
            .unwrap()
            .contains("authorization: [redacted]"));
    }
}
