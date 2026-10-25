use super::*;

const OMITTED: &str =
    "[Atlas: earlier tool image omitted by the Messages API image-history limit.]";

struct ImageUpstream {
    endpoint: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
    chat: bool,
}

impl ImageUpstream {
    async fn start(chat: bool, upstream_error: Option<&'static str>) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let capture = requests.clone();
        let mock = Router::new().fallback(post(move |Json(body): Json<Value>| {
            let capture = capture.clone();
            async move {
                capture.lock().unwrap().push(body);
                if let Some(message) = upstream_error {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error":{"message":message}})),
                    )
                        .into_response();
                }
                let wire = if chat {
                    format!(
                        "data: {}\n\ndata: [DONE]\n\n",
                        json!({"id":"chatcmpl_image_history","model":"gpt-6-astra","choices":[{"index":0,"delta":{"role":"assistant","content":"Images received"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2}})
                    )
                } else {
                    format!(
                        "event: response.completed\ndata: {}\n\n",
                        json!({"type":"response.completed","response":{"id":"resp_image_history","model":"gpt-6-astra","status":"completed","output":[{"id":"msg_image_history","type":"message","role":"assistant","content":[{"type":"output_text","text":"Images received"}]}],"usage":{"input_tokens":10,"output_tokens":2}}})
                    )
                };
                ([("content-type", "text/event-stream")], wire).into_response()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        Self {
            endpoint,
            requests,
            task,
            chat,
        }
    }

    fn fixture(&self) -> CopilotFixture {
        CopilotFixture {
            endpoint: self.endpoint.clone(),
            token: "fixture-token".into(),
            models: vec![CopilotModel {
                id: "gpt-6-astra".into(),
                name: "Image-history fixture".into(),
                vendor: "OpenAI".into(),
                model_picker_enabled: true,
                supported_endpoints: vec![if self.chat {
                    "/chat/completions"
                } else {
                    "/responses"
                }
                .into()],
                ..Default::default()
            }],
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }

    fn captured(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for ImageUpstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn image(label: &str) -> Value {
    json!({"type":"image","source":{"type":"url","url":format!("https://example.com/{label}.png")}})
}

fn image_history(total: usize, stream: bool) -> Value {
    assert!(total >= 4);
    let mut old_content = vec![json!({"type":"text","text":"Preserve old tool text"})];
    old_content.extend((0..total - 4).map(|n| image(&format!("old-{n}"))));
    json!({
        "model":"claude-sonnet-5", "max_tokens":32, "stream":stream,
        "system":"Compare the screenshots",
        "tools":[{"name":"screen","description":"Capture a screenshot","input_schema":{"type":"object","properties":{}}}],
        "messages":[
            {"role":"user","content":[{"type":"text","text":"Keep this reference"},image("user-reference")]},
            {"role":"assistant","content":[{"type":"tool_use","id":"old","name":"screen","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"old","content":old_content}]},
            {"role":"assistant","content":[{"type":"tool_use","id":"latest-a","name":"screen","input":{}},{"type":"tool_use","id":"latest-b","name":"screen","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"latest-a","content":[image("latest-a")]}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"latest-b","content":[image("latest-b")]},image("user-current")]}
        ]
    })
}

fn user_images(total: usize, stream: bool) -> Value {
    let content: Vec<_> = (0..total).map(|n| image(&format!("user-{n}"))).collect();
    json!({"model":"claude-sonnet-5","max_tokens":32,"stream":stream,"messages":[{"role":"user","content":content}]})
}

// Inspect real outbound parts for both transports, not image URLs serialized as text.
fn image_urls(value: &Value) -> Vec<String> {
    let mut urls = Vec::new();
    match value {
        Value::Object(object) => {
            match value["type"].as_str() {
                Some("input_image") => {
                    urls.push(value["image_url"].as_str().unwrap().to_owned());
                }
                Some("image_url") => {
                    urls.push(value["image_url"]["url"].as_str().unwrap().to_owned());
                }
                _ => {}
            }
            for child in object.values() {
                urls.extend(image_urls(child));
            }
        }
        Value::Array(array) => {
            for child in array {
                urls.extend(image_urls(child));
            }
        }
        _ => {}
    }
    urls
}

fn omission_count(value: &Value) -> usize {
    match value {
        Value::String(text) => text.matches(OMITTED).count(),
        Value::Array(array) => array.iter().map(omission_count).sum(),
        Value::Object(object) => object.values().map(omission_count).sum(),
        _ => 0,
    }
}

async fn response_json(response: Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}

async fn assert_success(response: Response, stream: bool) {
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("Images received"), "{text}");
    if stream {
        assert!(text.contains("event: message_start"), "{text}");
        assert!(text.contains("event: message_stop"), "{text}");
    } else {
        let body: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["type"], "message");
    }
}

#[tokio::test]
async fn image_budget_boundaries_reach_chat_and_responses_for_json_and_streaming() {
    for chat in [false, true] {
        for stream in [false, true] {
            let upstream = ImageUpstream::start(chat, None).await;
            let state = state(db());
            TEST_COPILOT
                .scope(upstream.fixture(), async {
                    for total in [49, 50, 51] {
                        let response =
                            messages(State(state.clone()), request(image_history(total, stream)))
                                .await;
                        assert_success(response, stream).await;
                        let rows = upstream.captured();
                        let outbound = rows.last().unwrap();
                        let urls = image_urls(outbound);
                        assert_eq!(
                            urls.len(),
                            total.min(50),
                            "chat={chat}, stream={stream}, images={total}"
                        );
                        assert_eq!(omission_count(outbound), usize::from(total > 50));
                        for protected in ["user-reference", "user-current", "latest-a", "latest-b"]
                        {
                            assert!(urls.contains(&format!("https://example.com/{protected}.png")));
                        }
                        assert_eq!(
                            urls.contains(&"https://example.com/old-0.png".to_owned()),
                            total <= 50
                        );
                        assert!(urls.contains(&"https://example.com/old-1.png".to_owned()));
                        assert!(outbound.to_string().contains("Preserve old tool text"));
                        assert_eq!(outbound["model"], "gpt-6-astra");
                    }
                })
                .await;
            assert_eq!(upstream.captured().len(), 3);
        }
    }
}

#[tokio::test]
async fn protected_user_overflow_is_rejected_before_any_upstream_request() {
    let upstream = ImageUpstream::start(false, None).await;
    TEST_COPILOT
        .scope(upstream.fixture(), async {
            for stream in [false, true] {
                let response = messages(State(state(db())), request(user_images(51, stream))).await;
                assert_eq!(response.status(), StatusCode::BAD_REQUEST);
                let body = response_json(response).await;
                assert_eq!(body["type"], "error");
                assert_eq!(body["error"]["type"], "invalid_request_error");
                assert!(body["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("Messages API image limit exceeded"));
            }
        })
        .await;
    assert!(upstream.captured().is_empty());
}

#[tokio::test]
async fn disabled_image_history_forwards_all_51_protected_images() {
    for chat in [false, true] {
        let upstream = ImageUpstream::start(chat, None).await;
        let state = state(db());
        state.config.write().await.claude.image_history.enabled = false;
        TEST_COPILOT
            .scope(upstream.fixture(), async {
                assert_success(
                    messages(State(state), request(user_images(51, false))).await,
                    false,
                )
                .await;
            })
            .await;
        let rows = upstream.captured();
        assert_eq!(rows.len(), 1);
        assert_eq!(image_urls(&rows[0]).len(), 51);
        assert_eq!(omission_count(&rows[0]), 0);
    }
}

#[tokio::test]
async fn direct_responses_does_not_apply_the_messages_image_budget() {
    let upstream = ImageUpstream::start(false, None).await;
    let content: Vec<_> = (0..51)
        .map(|n| json!({"type":"input_image","image_url":format!("https://example.com/direct-{n}.png")}))
        .collect();
    let native =
        json!({"model":"gpt-6-astra","stream":true,"input":[{"role":"user","content":content}]});
    TEST_COPILOT
        .scope(upstream.fixture(), async {
            let mut request = request(native.clone());
            *request.uri_mut() = "/v1/responses".parse().unwrap();
            let response = handlers::handle_responses(State(state(db())), request)
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
        })
        .await;
    let rows = upstream.captured();
    assert_eq!(rows.len(), 1);
    assert_eq!(image_urls(&rows[0]).len(), 51);
    assert_eq!(rows[0]["input"], native["input"]);
    assert_eq!(omission_count(&rows[0]), 0);
}

#[tokio::test]
async fn count_tokens_uses_the_same_pruned_history_without_an_upstream_call() {
    let upstream = ImageUpstream::start(false, None).await;
    let original = image_history(51, false);
    let mut explicitly_pruned = original.clone();
    explicitly_pruned["messages"][2]["content"][0]["content"][1] =
        json!({"type":"text","text":OMITTED});
    let state = state(db());
    TEST_COPILOT
        .scope(upstream.fixture(), async {
            let response = count_tokens(State(state.clone()), request(original)).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["x-atlas-token-count"], "estimate");
            let actual = response_json(response).await;
            let expected = count_tokens(State(state.clone()), request(explicitly_pruned)).await;
            assert_eq!(expected.status(), StatusCode::OK);
            assert_eq!(
                actual["input_tokens"],
                response_json(expected).await["input_tokens"]
            );
            state.config.write().await.claude.image_history.enabled = false;
            let unpruned = count_tokens(State(state), request(image_history(51, false))).await;
            assert_eq!(unpruned.status(), StatusCode::OK);
            assert!(
                response_json(unpruned).await["input_tokens"]
                    .as_u64()
                    .unwrap()
                    > actual["input_tokens"].as_u64().unwrap()
            );
        })
        .await;
    assert!(upstream.captured().is_empty());
}

#[tokio::test]
async fn count_tokens_rejects_protected_overflow_and_does_not_count_base64_text() {
    let upstream = ImageUpstream::start(false, None).await;
    TEST_COPILOT
        .scope(upstream.fixture(), async {
            let state = state(db());
            let overflow = count_tokens(State(state.clone()), request(user_images(51, false))).await;
            assert_eq!(overflow.status(), StatusCode::BAD_REQUEST);
            assert!(response_json(overflow).await["error"]["message"].as_str().unwrap().contains("Messages API image limit exceeded"));
            let mut counts = Vec::new();
            for data in ["YQ==".to_owned(), "YWFh".repeat(32 * 1024)] {
                let body = json!({"model":"claude-sonnet-5","system":"Inspect image","messages":[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":data}}]}]});
                let response = count_tokens(State(state.clone()), request(body)).await;
                assert_eq!(response.status(), StatusCode::OK);
                counts.push(response_json(response).await["input_tokens"].as_u64().unwrap());
            }
            assert_eq!(counts[0], counts[1], "Base64 bytes must not be estimated as prompt text");
        })
        .await;
    assert!(upstream.captured().is_empty());
}

#[tokio::test]
async fn old_settings_default_image_history_on_and_model_overrides_persist() {
    let legacy: ClaudeConfig =
        serde_json::from_value(json!({"address":"127.0.0.1","port":15723,"sonnet":"gpt-6-astra"}))
            .unwrap();
    assert!(legacy.image_history.enabled);
    assert_eq!(legacy.image_history.max_images, 50);
    assert!(legacy.image_history.model_limits.is_empty());
    let db = db();
    let mut settings = db.get_global_proxy_config().await.unwrap();
    settings.claude = legacy;
    settings.claude.image_history.max_images = 40;
    settings
        .claude
        .image_history
        .model_limits
        .insert("gpt-6-astra".into(), 30);
    db.update_global_proxy_config(settings.clone())
        .await
        .unwrap();
    let loaded = db.get_global_proxy_config().await.unwrap();
    assert_eq!(loaded.claude.image_history, settings.claude.image_history);
    let wire = serde_json::to_value(&loaded.claude).unwrap();
    assert_eq!(wire["imageHistory"]["enabled"], true);
    assert_eq!(wire["imageHistory"]["maxImages"], 40);
    assert_eq!(wire["imageHistory"]["modelLimits"]["gpt-6-astra"], 30);
}

#[tokio::test]
async fn upstream_image_limit_errors_keep_original_message_without_retry() {
    const ORIGINAL: &str = "Maximum number of images is 20 per request";
    let upstream = ImageUpstream::start(false, Some(ORIGINAL)).await;
    TEST_COPILOT
        .scope(upstream.fixture(), async {
            let response = messages(State(state(db())), request(image_history(49, false))).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body = response_json(response).await;
            assert_eq!(body["type"], "error");
            let message = body["error"]["message"].as_str().unwrap();
            assert!(
                message.contains(ORIGINAL),
                "Original upstream diagnostic was lost: {message}"
            );
            assert!(
                message.len() > ORIGINAL.len(),
                "Expected actionable Messages API guidance: {message}"
            );
        })
        .await;
    let rows = upstream.captured();
    assert_eq!(
        rows.len(),
        1,
        "Image-limit failures must not retry with images removed"
    );
    assert_eq!(image_urls(&rows[0]).len(), 49);
}
