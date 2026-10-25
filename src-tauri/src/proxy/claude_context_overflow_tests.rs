use super::*;

const OBSERVED_OVERFLOW: &str =
    "Your input exceeds the context window of this model. Please adjust your input and try again.";

struct ErrorUpstream {
    fixture: CopilotFixture,
    task: tokio::task::JoinHandle<()>,
}

impl ErrorUpstream {
    async fn start(status: StatusCode, body: Value, sse: bool, chat: bool) -> Self {
        let upstream = Router::new().fallback(post(move || {
            let body = body.clone();
            async move {
                if sse {
                    (
                        status,
                        [("content-type", "text/event-stream")],
                        format!(
                            "event: {}\ndata: {body}\n\n",
                            body["type"].as_str().unwrap()
                        ),
                    )
                        .into_response()
                } else {
                    (status, [("retry-after", "7")], Json(body)).into_response()
                }
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        Self {
            fixture: CopilotFixture {
                endpoint,
                token: "fixture-token".into(),
                models: vec![CopilotModel {
                    id: "gpt-6-astra".into(),
                    name: "Context overflow fixture".into(),
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
            },
            task,
        }
    }
}

impl Drop for ErrorUpstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn error_body(response: Response) -> String {
    String::from_utf8(
        axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

fn prompt(stream: bool) -> Value {
    json!({"model":"claude-fable-5","max_tokens":32,"stream":stream,"messages":[{"role":"user","content":"Hello"}]})
}

#[tokio::test]
async fn context_overflow_http_errors_are_recognizable_on_both_copilot_transports() {
    for chat in [false, true] {
        for stream in [false, true] {
            for (upstream_error, expected) in [
                (json!({"message":OBSERVED_OVERFLOW}), "prompt is too long"),
                (
                    json!({"code":"context_length_exceeded","message":OBSERVED_OVERFLOW}),
                    "prompt is too long",
                ),
                (
                    json!({"code":"model_max_prompt_tokens_exceeded","message":"prompt token count of 210000 exceeds the limit of 200000"}),
                    "prompt is too long: 210000 tokens > 200000 maximum",
                ),
            ] {
                let upstream = ErrorUpstream::start(
                    StatusCode::BAD_REQUEST,
                    json!({"error":upstream_error}),
                    false,
                    chat,
                )
                .await;
                let response = TEST_COPILOT
                    .scope(
                        upstream.fixture.clone(),
                        messages(State(state(db())), request(prompt(stream))),
                    )
                    .await;
                assert_eq!(response.status(), StatusCode::BAD_REQUEST);
                let body: Value = serde_json::from_str(&error_body(response).await).unwrap();
                assert_eq!(
                    body,
                    json!({"type":"error","error":{"type":"invalid_request_error","message":expected}}),
                    "chat={chat} stream={stream}"
                );
            }
        }
    }
}

#[tokio::test]
async fn context_overflow_sse_failures_reach_streaming_and_buffered_claude_clients() {
    for stream in [false, true] {
        for event in [
            json!({"type":"response.failed","response":{"status":"failed","error":{"code":"context_length_exceeded","message":OBSERVED_OVERFLOW}}}),
            json!({"type":"error","error":{"code":"model_max_prompt_tokens_exceeded","message":OBSERVED_OVERFLOW}}),
            json!({"type":"response.completed","response":{"status":"failed","error":{"code":"context_length_exceeded","message":OBSERVED_OVERFLOW}}}),
        ] {
            let upstream = ErrorUpstream::start(StatusCode::OK, event, true, false).await;
            let response = TEST_COPILOT
                .scope(
                    upstream.fixture.clone(),
                    messages(State(state(db())), request(prompt(stream))),
                )
                .await;
            assert_eq!(
                response.status(),
                if stream {
                    StatusCode::OK
                } else {
                    StatusCode::BAD_REQUEST
                }
            );
            let body = error_body(response).await;
            assert!(body.contains("prompt is too long"), "{body}");
            assert!(body.contains("invalid_request_error"), "{body}");
            assert!(!body.contains("event: message_stop"), "{body}");
            if stream {
                assert!(body.contains("event: error"), "{body}");
            }
        }
    }
}

#[tokio::test]
async fn context_overflow_normalization_does_not_reclassify_other_http_errors() {
    for (status, message) in [
        (StatusCode::BAD_REQUEST, "Invalid schema for function Read"),
        (StatusCode::BAD_REQUEST, "Too many images"),
        (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded"),
        (StatusCode::INTERNAL_SERVER_ERROR, OBSERVED_OVERFLOW),
        (StatusCode::PAYLOAD_TOO_LARGE, OBSERVED_OVERFLOW),
    ] {
        let upstream =
            ErrorUpstream::start(status, json!({"error":{"message":message}}), false, false).await;
        let response = TEST_COPILOT
            .scope(
                upstream.fixture.clone(),
                messages(State(state(db())), request(prompt(true))),
            )
            .await;
        assert_eq!(response.status(), status);
        let body = error_body(response).await;
        assert!(!body.contains("prompt is too long"), "{status}: {body}");
    }
}
