//! Bounded, content-free HTTP diagnostics for requests sent to Copilot.
//!
//! The Atlas ID is also the request ID of a failed usage-history row. Never
//! interpolate request bodies, arbitrary headers, or upstream error text into
//! a log record without passing through the small allowlists below.

use super::{
    error_mapper::map_proxy_error_to_status, types::ReasoningEffort,
    upstream_response::ProxyResponse, usage::logger::UsageLogger, ProxyError,
};
use crate::Database;
use bytes::Bytes;
use futures::{stream::Stream, task::Poll};
use http::HeaderMap;
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
const MAX_MESSAGE_CHARS: usize = 200;
const MAX_DIAGNOSTIC_IDS: usize = 6;

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
    status: Option<u16>,
    declared_bytes: Option<u64>,
    encoding: Option<String>,
    upstream_ids: Vec<String>,
    body_summary: Option<String>,
    body_truncated: bool,
    preview: Vec<u8>,
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
    }

    pub fn outgoing(
        &self,
        model: Option<&str>,
        transport: &'static str,
        streaming: bool,
        effort: &ReasoningEffort,
        bytes: usize,
        body: &Value,
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
        let status = self.details().status;
        if status.is_some_and(|status| status >= 400) {
            let mut details = self.details();
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
            "atlas_id={} status={} elapsed_ms={} endpoint={} requested_model={} upstream_model={} transport={} streaming={} applied_effort={} outgoing_bytes={} input_items={} tools={} response_bytes={} declared_bytes={} truncated={} outcome={} upstream_ids={}",
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
    }

    pub fn failed_event(&self, event: &Value) {
        let error = event
            .pointer("/response/error")
            .or_else(|| event.get("error"));
        let summary = error
            .map(summarize_error_value)
            .unwrap_or_else(|| "kind=sse message=response.failed".into());
        self.details().body_summary = Some(summary);
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
            "atlas_id={} stage={} upstream_status={} history_status={} endpoint={} requested_model={} upstream_model={} transport={} streaming={} outgoing_bytes={} input_items={} tools={} response_bytes={} truncated={} body_truncated={} upstream_ids={} body={}",
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
            self.response_bytes.load(Ordering::Relaxed),
            self.response_truncated.load(Ordering::Relaxed),
            details.body_truncated || details.preview.len() >= MAX_ERROR_PREVIEW,
            ids,
            summary
        );

        let logger = UsageLogger::new(&self.db);
        if logger
            .log_error_with_context(
                self.id.clone(),
                self.provider_id.clone(),
                "codex".into(),
                self.requested_model.clone(),
                history_status,
                format!("Atlas diagnostic {}: {summary}", self.id),
                self.started.elapsed().as_millis() as u64,
                details.streaming,
                Some(self.session_id.clone()),
                None,
                ReasoningEffort {
                    requested: None,
                    applied: (details.applied_effort != "none").then_some(details.applied_effort),
                },
            )
            .is_err()
        {
            log::warn!(target: "atlas_http", "atlas_id={} failed to record diagnostic history row", self.id);
        }
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

fn diagnostic_field(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => bounded_error_text(value.as_bytes()),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        _ => "none".into(),
    }
}

fn summarize_error_value(value: &Value) -> String {
    let error = value.get("error").unwrap_or(value);
    let message = error
        .get("message")
        .or_else(|| error.get("detail"))
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .map(|value| bounded_error_text(value.as_bytes()))
        .unwrap_or_else(|| "[message unavailable]".into());
    let code = diagnostic_field(error.get("code"));
    let kind = diagnostic_field(error.get("type"));
    let param = diagnostic_field(error.get("param"));
    let request_id = ["request_id", "requestId", "correlation_id"]
        .iter()
        .find_map(|key| error.get(key).and_then(Value::as_str))
        .and_then(safe_correlation_id)
        .unwrap_or_else(|| "none".into());
    format!("kind=json message={message:?} code={code} type={kind} param={param} body_request_id={request_id}")
}

fn bounded_error_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_ERROR_JSON)])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_MESSAGE_CHARS)
        .collect()
}

fn summarize_body(body: &[u8], compressed: bool, truncated: bool) -> String {
    if body.is_empty() {
        return "kind=empty".into();
    }
    let text = String::from_utf8_lossy(&body[..body.len().min(MAX_ERROR_JSON)]);
    if !compressed && !truncated && body.len() <= MAX_ERROR_JSON {
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            return summarize_error_value(&value);
        }
    }
    let kind = if compressed {
        "compressed"
    } else if truncated {
        "truncated"
    } else {
        "text"
    };
    format!("kind={kind} message={:?}", bounded_error_text(body))
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
        );
        diagnostics
    }

    #[test]
    fn sanitizes_error_messages_and_never_logs_arbitrary_body_text() {
        assert_eq!(
            summarize_body(br#"{"error":{"message":"prompt token count of 390003 exceeds the limit of 372000","code":"model_max_prompt_tokens_exceeded","type":"invalid_request_error","param":"input"}}"#, false, false),
            "kind=json message=\"prompt token count of 390003 exceeds the limit of 372000\" code=model_max_prompt_tokens_exceeded type=invalid_request_error param=input body_request_id=none"
        );
        assert_eq!(
            summarize_body(b"Bad Request\n", false, false),
            "kind=text message=\"Bad Request\""
        );
        assert_eq!(
            summarize_body(b"my private prompt and bearer secret", false, false),
            "kind=text message=\"my private prompt and bearer secret\""
        );
        assert_eq!(summarize_body(b"", false, false), "kind=empty");
        assert_eq!(
            summarize_body(b"compressed bytes", true, false),
            "kind=compressed message=\"compressed bytes\""
        );
        assert_eq!(
            summarize_body(
                br#"{"error":{"message":"Authorization bearer private-token was rejected","code":"model_max_prompt_tokens_exceeded","param":"input.token_count"}}"#,
                false,
                false
            ),
            "kind=json message=\"Authorization bearer private-token was rejected\" code=model_max_prompt_tokens_exceeded type=none param=input.token_count body_request_id=none"
        );
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
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, log::Level::Info);
        assert_eq!(entries[1].0, log::Level::Warn);
        assert!(entries[1].1.contains("390003"));
        assert!(entries[1].1.contains("model_max_prompt_tokens_exceeded"));
        for (_, line) in &entries {
            assert_eq!(line.lines().count(), 1);
            assert!(line.contains(&diagnostics.id));
            assert!(line.contains("upstream-123"));
            assert!(!line.contains("Bearer private"));
            assert!(!line.contains("access_token"));
        }
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
        assert!(entries[1].1.contains("private compressed"));

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
        assert!(entries[1].1.contains("pppppp"));
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
        diagnostics.failed_event(&serde_json::json!({
            "type": "response.failed",
            "response": {
                "error": {
                    "message": "prompt token count of 390003 exceeds the limit of 372000",
                    "code": "model_max_prompt_tokens_exceeded",
                    "param": "input"
                }
            }
        }));
        let history = db.get_request_logs(&Default::default(), 0, 10).unwrap();
        assert_eq!(history.data[0].request_id, diagnostics.id);
        let entries = logger.0.lock().unwrap().clone();
        assert!(entries
            .iter()
            .any(|(_, line)| line.contains("code=model_max_prompt_tokens_exceeded")));
        assert!(entries.iter().any(|(_, line)| line.contains("390003")));
    }
}
