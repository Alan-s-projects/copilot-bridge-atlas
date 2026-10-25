use super::*;
use std::io::Write;
use std::process::{Command, Stdio};

// Optional end-to-end tests: real Claude Code, real Atlas routes, deterministic local Copilot responses.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires ATLAS_TEST_CLAUDE_BIN; starts an isolated Claude Code process"]
async fn context_overflow_real_claude_compacts_and_retries() {
    run_claude_recovery(false, false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires ATLAS_TEST_CLAUDE_BIN; starts an isolated Claude Code process"]
async fn context_overflow_real_claude_stream_compacts_and_retries() {
    run_claude_recovery(false, true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires ATLAS_TEST_CLAUDE_BIN; starts an isolated Claude Code process"]
async fn context_overflow_real_claude_old_wording_does_not_compact() {
    run_claude_recovery(true, false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires ATLAS_TEST_CLAUDE_BIN and ATLAS_LIVE_AUTH_DIR; makes live Copilot requests"]
async fn context_overflow_live_claude_compacts_and_retries() {
    run_claude_recovery(false, false, true).await;
}

async fn run_claude_recovery(old_wording: bool, streamed_error: bool, live: bool) {
    let binary = std::env::var("ATLAS_TEST_CLAUDE_BIN").expect("set ATLAS_TEST_CLAUDE_BIN");
    let root =
        std::env::var("ATLAS_TEST_CLAUDE_ARTIFACTS").expect("set ATLAS_TEST_CLAUDE_ARTIFACTS");
    let root = std::path::PathBuf::from(root);
    std::fs::create_dir_all(&root).unwrap();
    let run = root.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&run).unwrap();
    let auth_home = tempfile::tempdir().unwrap();
    let live_config = if live {
        use crate::proxy::providers::copilot_auth::CopilotAuthManager;
        let source = std::env::var("ATLAS_LIVE_AUTH_DIR").expect("set ATLAS_LIVE_AUTH_DIR");
        let auth = auth_home.path().join("copilot_auth.json");
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
        let manager = CopilotAuthManager::new(auth_home.path().to_owned());
        let token = manager
            .get_valid_token()
            .await
            .expect("Copilot authentication failed");
        let models = manager
            .fetch_models()
            .await
            .expect("Copilot catalog unavailable");
        Some((manager.get_default_api_endpoint().await, token, models))
    } else {
        None
    };
    let live_endpoint = live_config
        .as_ref()
        .map(|(endpoint, _, _)| endpoint.clone());
    let live_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .unwrap();
    let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
    let capture = calls.clone();
    let upstream = Router::new().fallback(post(move |uri: axum::http::Uri, mut headers: HeaderMap, Json(body): Json<Value>| {
        let capture = capture.clone();
        let live_endpoint = live_endpoint.clone();
        let live_client = live_client.clone();
        async move {
            let input = body.get("input").map(Value::to_string).unwrap_or_default();
            let is_summary = input.contains("create a detailed summary of the conversation");
            let has_original_history = input.contains("Reference material:");
            let index = {
                let mut calls = capture.lock().unwrap();
                calls.push(body.clone());
                calls.len()
            };
            if index >= 2 && has_original_history && !is_summary
                && !input.contains("This session is being continued from a previous conversation") {
                let failure = json!({"code":"context_length_exceeded","message":"Your input exceeds the context window of this model. Please adjust your input and try again."});
                if streamed_error {
                    return ([("content-type", "text/event-stream")], format!("event: response.failed\ndata: {}\n\n", json!({"type":"response.failed","response":{"status":"failed","error":failure}}))).into_response();
                }
                return (StatusCode::BAD_REQUEST, Json(json!({"error":failure}))).into_response();
            }
            if let Some(endpoint) = live_endpoint {
                headers.remove("host");
                headers.remove("content-length");
                let response = live_client.post(format!("{}{uri}", endpoint.trim_end_matches('/')))
                    .headers(headers).json(&body).send().await.unwrap();
                let status = response.status();
                let content_type = response.headers().get("content-type").cloned().unwrap_or_else(|| "application/json".parse().unwrap());
                let bytes = response.bytes().await.unwrap();
                return (status, [("content-type", content_type)], bytes).into_response();
            }
            let text = if index == 1 {
                "I remember the project codeword: ATLAS_CONTEXT_RECOVERED."
            } else if is_summary {
                "The user supplied reference material. The project codeword is ATLAS_CONTEXT_RECOVERED. The pending request is to return that codeword. Continue with that answer."
            } else {
                "ATLAS_CONTEXT_RECOVERED"
            };
            let wire = format!("event: response.completed\ndata: {}\n\n", json!({"type":"response.completed","response":{"id":format!("resp_fixture_{index}"),"model":"gpt-6-astra","status":"completed","output":[{"id":format!("msg_fixture_{index}"),"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}],"usage":{"input_tokens":if index == 1 {20000} else {1000},"output_tokens":50}}}));
            ([("content-type", "text/event-stream")], wire).into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_endpoint = format!("http://{}", listener.local_addr().unwrap());
    let upstream_task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let mut fixture = CopilotFixture {
        endpoint: upstream_endpoint,
        token: "fixture-token".into(),
        models: vec![CopilotModel {
            id: "gpt-6-astra".into(),
            name: "Local compaction fixture".into(),
            vendor: "OpenAI".into(),
            model_picker_enabled: true,
            supported_endpoints: vec!["/responses".into()],
            ..Default::default()
        }],
        client: reqwest::Client::builder().no_proxy().build().unwrap(),
    };
    if let Some((_, token, models)) = live_config {
        fixture.token = token;
        fixture.models = models;
    }
    let gateway = router(state(db())).layer(axum::middleware::from_fn(
        move |request, next: axum::middleware::Next| {
            let fixture = fixture.clone();
            async move {
                let response = TEST_COPILOT.scope(fixture, next.run(request)).await;
                if old_wording && response.status() == StatusCode::BAD_REQUEST {
                    return error(StatusCode::BAD_REQUEST, "Copilot Bridge Atlas local proxy failed while handling Codex endpoint /responses. Provider: GitHub Copilot; model: gpt-6-astra; upstream_status: HTTP 400; cause: Your input exceeds the context window of this model. Please adjust your input and try again.");
                }
                response
            }
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let gateway_task = tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
    let session = uuid::Uuid::new_v4().to_string();
    let mut outputs = Vec::new();
    for (turn, prompt) in [
        format!(
            "Remember the project codeword ATLAS_CONTEXT_RECOVERED. Reference material: {}",
            "alpha beta gamma delta epsilon. ".repeat(if live { 500 } else { 10000 })
        ),
        "Return the project codeword.".into(),
    ]
    .into_iter()
    .enumerate()
    {
        let binary = binary.clone();
        let run = run.clone();
        let endpoint = endpoint.clone();
        let session = session.clone();
        let output = tokio::task::spawn_blocking(move || {
            let mut command = Command::new(binary);
            command
                .env_clear()
                .env("PATH", std::env::var("PATH").unwrap_or_default())
                .env("HOME", &run)
                .env("CLAUDE_CONFIG_DIR", run.join("config"))
                .env("ANTHROPIC_BASE_URL", endpoint)
                .env("ANTHROPIC_API_KEY", "local-fixture-only")
                .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
                .current_dir(&run)
                .args([
                    "--bare",
                    "--print",
                    "--verbose",
                    "--output-format",
                    "stream-json",
                    "--model",
                    "claude-fable-5",
                    "--tools",
                    "",
                    "--strict-mcp-config",
                    "--setting-sources",
                    "",
                    "--system-prompt",
                    "You are a test assistant. Remember the supplied codeword.",
                ])
                .arg("--debug-file")
                .arg(run.join(format!("turn-{turn}.debug.log")))
                .args([
                    if turn == 0 {
                        "--session-id"
                    } else {
                        "--resume"
                    },
                    &session,
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = command.spawn().unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(prompt.as_bytes())
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            while child.try_wait().unwrap().is_none() {
                if std::time::Instant::now() >= deadline {
                    child.kill().unwrap();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let output = child.wait_with_output().unwrap();
            std::fs::write(
                run.join(format!("turn-{turn}.stdout.jsonl")),
                &output.stdout,
            )
            .unwrap();
            std::fs::write(run.join(format!("turn-{turn}.stderr.log")), &output.stderr).unwrap();
            output
        })
        .await
        .unwrap();
        outputs.push(output);
        if !outputs.last().unwrap().status.success() {
            break;
        }
    }
    gateway_task.abort();
    upstream_task.abort();
    let requests = calls.lock().unwrap().clone();
    std::fs::write(
        run.join("upstream-requests.json"),
        serde_json::to_vec_pretty(&requests).unwrap(),
    )
    .unwrap();
    println!("Claude Code compaction artifacts: {}", run.display());
    assert_eq!(
        outputs.len(),
        2,
        "first turn failed; inspect {}",
        run.display()
    );
    let stdout = String::from_utf8_lossy(&outputs[1].stdout);
    if old_wording {
        assert!(
            !outputs[1].status.success(),
            "Old wording unexpectedly recovered: {stdout}"
        );
        assert!(
            !stdout.contains("compact_boundary"),
            "Old wording triggered compaction: {stdout}"
        );
        assert!(
            stdout.contains("Your input exceeds the context window"),
            "Unexpected control failure: {stdout}"
        );
        assert!(
            requests.iter().all(|request| !request
                .to_string()
                .contains("create a detailed summary of the conversation")),
            "Old wording should not trigger summarization"
        );
        return;
    }
    assert!(
        outputs[1].status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&outputs[1].stderr)
    );
    assert!(
        stdout.contains("compact_boundary"),
        "No actual Claude Code compaction event: {stdout}"
    );
    assert!(
        stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|value| value["type"] == "result"
                && value["is_error"] == false
                && value["result"]
                    .as_str()
                    .is_some_and(|text| text.contains("ATLAS_CONTEXT_RECOVERED"))),
        "No successful continuation: {stdout}"
    );
    assert!(
        requests.len() >= 4,
        "Expected initial turn, rejection, summarization, retry"
    );
    assert!(
        requests.iter().skip(2).any(|request| request
            .to_string()
            .contains("create a detailed summary of the conversation")),
        "No summarization request reached the upstream"
    );
}
