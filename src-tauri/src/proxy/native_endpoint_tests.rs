use super::*;
use crate::proxy::{
    endpoint_registry::{Endpoint, ENDPOINTS},
    forwarder::{CopilotFixture, TEST_COPILOT},
    providers::{copilot_auth::CopilotModel, copilot_model_map},
};
use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use std::sync::Mutex;
use tower::Service;

fn server(settings: Value) -> ProxyServer {
    let db = Arc::new(Database::memory().unwrap());
    let mut provider =
        crate::provider::Provider::with_id("copilot".into(), "Copilot".into(), settings);
    provider.meta = Some(crate::ProviderMeta {
        provider_type: Some("github_copilot".into()),
        ..Default::default()
    });
    db.save_provider("codex", &provider).unwrap();
    db.set_current_provider("codex", &provider.id).unwrap();
    ProxyServer::new(ProxyConfig::default(), db, None)
}

fn model(id: &str, endpoint: &str) -> CopilotModel {
    CopilotModel {
        id: id.into(),
        model_type: if endpoint == "/v1/systemone" {
            "decisions"
        } else {
            "chat"
        }
        .into(),
        model_picker_enabled: endpoint != "/v1/systemone",
        supported_endpoints: vec![endpoint.into()],
        ..Default::default()
    }
}

fn request(path: &str, body: &Value) -> Request<Body> {
    Request::post(path)
        .header("content-type", "application/json")
        .header("authorization", "Bearer client-secret")
        .header("cookie", "session=private")
        .header("proxy-authorization", "Basic private")
        .header("connection", "x-private-hop")
        .header("x-private-hop", "private")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn registry_primary_paths_are_routed_and_unknown_decisions_is_not() {
    let proxy = server(json!({}));
    let mut app = proxy.build_router();
    for endpoint in ENDPOINTS
        .into_iter()
        .filter(|e| e.listener() == "responses")
    {
        for path in endpoint.paths() {
            let method = if matches!(endpoint, Endpoint::Models | Endpoint::ResponsesWebSocket) {
                "GET"
            } else {
                "POST"
            };
            let req = Request::builder()
                .method(method)
                .uri(*path)
                .body(Body::from("{}"))
                .unwrap();
            let response = app.call(req).await.unwrap();
            assert_ne!(response.status(), StatusCode::NOT_FOUND, "{method} {path}");
            assert_ne!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} {path}"
            );
        }
    }
    for path in ["/v1/decisions", "/v1/unknown", "/v1/systemone/extra"] {
        assert_eq!(
            app.call(request(path, &json!({}))).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }
    let mut messages = crate::proxy::claude::router(proxy.state.clone());
    assert_eq!(
        messages
            .call(request("/v1/systemone", &json!({})))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn native_http_preserves_payloads_errors_and_streams_and_replaces_credentials() {
    let captured = Arc::new(Mutex::new(Vec::<(String, HeaderMap, Value)>::new()));
    let capture = captured.clone();
    let upstream = Router::new().fallback(move |req: Request<Body>| {
        let capture = capture.clone();
        async move {
            let (parts, body) = req.into_parts();
            let value: Value = serde_json::from_slice(&to_bytes(body, 1024 * 1024).await.unwrap()).unwrap();
            capture.lock().unwrap().push((parts.uri.to_string(), parts.headers, value.clone()));
            if value.get("fail_plain") == Some(&json!(true)) {
                return (StatusCode::SERVICE_UNAVAILABLE, [("retry-after", "42")], "temporary upstream failure").into_response();
            }
            if value.get("fail") == Some(&json!(true)) {
                return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error":{"message":"fixture rejection"}}))).into_response();
            }
            if value.get("stream") == Some(&json!(true)) {
                return ([("content-type", "text/event-stream")],
                    "data: {\"id\":\"chat_test\",\"model\":\"chat-model\",\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"id\":\"chat_test\",\"model\":\"chat-model\",\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n").into_response();
            }
            if value.get("messages").is_some() {
                return Json(json!({"id":"chat_json_test","model":value["model"],"choices":[{"message":{"role":"assistant","content":"Hello"}}],"usage":{"prompt_tokens":4,"completion_tokens":1}})).into_response();
            }
            Json(json!({"model":value["model"],"answers":{"_billing":{"type":"noul","noul":0.99}},"usage":{"input_tokens":28,"output_tokens":1}})).into_response()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let fixture = CopilotFixture {
        endpoint: format!("http://{address}"),
        token: "upstream-token".into(),
        models: vec![
            model("microsoft-decision-1", "/v1/systemone"),
            model("chat-model", "/chat/completions"),
        ],
        client: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
    };
    TEST_COPILOT.scope(fixture, async {
        let proxy = server(json!({}));
        let mut app = proxy.build_router();
        let body = json!({"model":"microsoft-decision-1","state":{"_private":"keep","nested":{"_key":42}},"questions":{"_billing":{"type":"noul","instructions":"Is this billing?"}}});
        for path in ["/v1/systemone", "/systemone"] {
            let response = app.call(request(path, &body)).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let result: Value = serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
            assert_eq!(result["answers"]["_billing"]["noul"], 0.99);
        }
        let chat = json!({"model":"chat-model","messages":[{"role":"user","content":"Hello"}],"tools":[{"type":"function","function":{"name":"test","parameters":{"type":"object","properties":{"_user_field":{"type":"string"}}}}}],"stream":true});
        let response = app.call(request("/v1/chat/completions?test=1", &chat)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let text = String::from_utf8(to_bytes(response.into_body(), 65536).await.unwrap().to_vec()).unwrap();
        assert!(text.contains("chat_test") && text.contains("[DONE]"));
        assert!(!text.contains("response.completed"));
        let mut failing = body.clone(); failing["fail"] = json!(true);
        let response = app.call(request("/v1/systemone", &failing)).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let result: Value = serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(result["error"]["message"], "fixture rejection");
        let mut plain = body.clone(); plain["fail_plain"] = json!(true);
        let response = app.call(request("/v1/systemone", &plain)).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["retry-after"], "42");
        assert_eq!(to_bytes(response.into_body(), 65536).await.unwrap().as_ref(), b"temporary upstream failure");
        let mut compressed = request("/v1/systemone", &body);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, body.to_string().as_bytes()).unwrap();
        *compressed.body_mut() = Body::from(encoder.finish().unwrap());
        compressed.headers_mut().insert("content-encoding", "gzip".parse().unwrap());
        let response = app.call(compressed).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        to_bytes(response.into_body(), 65536).await.unwrap();
        let mut nonstream = chat.clone(); nonstream["stream"] = json!(false);
        let response = app.call(request("/chat/completions", &nonstream)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: Value = serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(result["choices"][0]["message"]["content"], "Hello");
        let sent = captured.lock().unwrap();
        assert_eq!(sent.len(), 7);
        assert_eq!(sent[5].2, body);
        assert!(!sent[5].1.contains_key("content-encoding"));
        assert_eq!(sent[0].0, "/v1/systemone");
        assert_eq!(sent[0].2, body);
        assert_eq!(sent[2].0, "/chat/completions?test=1");
        assert_eq!(sent[2].2, chat);
        for (_, headers, _) in sent.iter() {
            assert_eq!(headers["authorization"], "Bearer upstream-token");
            for secret in ["cookie", "proxy-authorization", "x-private-hop"] { assert!(!headers.contains_key(secret)); }
        }
    }).await;
    task.abort();
}

#[tokio::test]
async fn native_http_rejects_ineligible_models_and_invalid_shapes_before_network() {
    let mut disabled = model("disabled", "/v1/systemone");
    disabled.policy_state = Some("disabled".into());
    let fixture = CopilotFixture {
        endpoint: "http://127.0.0.1:1".into(),
        token: "unused".into(),
        models: vec![
            model("decision", "/v1/systemone"),
            disabled,
            model("chat", "/chat/completions"),
        ],
        client: reqwest::Client::builder().no_proxy().build().unwrap(),
    };
    TEST_COPILOT.scope(fixture, async {
        let mut app = server(json!({"modelCatalog":{"models":[{"model":"decision","enabled":false}]}})).build_router();
        for model in ["absent", "chat", "disabled", "decision"] {
            let response = app.call(request("/v1/systemone", &json!({"model":model,"state":"x","questions":{"q":{"type":"noul","instructions":"x"}}}))).await.unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{model}");
        }
        for body in [json!({}), json!({"model":"decision","input":"x","questions":[]}), json!({"model":"decision","state":"x","questions":{},"stream":true}), json!([])] {
            assert_eq!(app.call(request("/v1/systemone", &body)).await.unwrap().status(), StatusCode::BAD_REQUEST);
        }
        assert_eq!(app.call(request("/v1/chat/completions", &json!({"model":"chat","messages":[]}))).await.unwrap().status(), StatusCode::BAD_REQUEST);
    }).await;
}

#[tokio::test]
async fn browser_origins_cannot_spend_native_api_quota() {
    let db = Arc::new(Database::memory().unwrap());
    let proxy = ProxyServer::new(
        ProxyConfig {
            listen_port: 0,
            ..Default::default()
        },
        db,
        None,
    );
    let info = proxy.start().await.unwrap();
    let base = format!("http://127.0.0.1:{}", info.port);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for origin in ["https://attacker.example", "null", "http://localhost:1234"] {
        let response = client
            .get(format!("{base}/v1/responses"))
            .header("origin", origin)
            .header("connection", "Upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let response = client
            .post(format!("{base}/v1/systemone"))
            .header("origin", origin)
            .json(&json!({"model":"decision","state":"x","questions":{"q":{"type":"noul"}}}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    proxy.stop().await.unwrap();
}

#[test]
#[ignore = "exports real endpoint descriptions for an isolated renderer preview"]
fn export_endpoint_directory() {
    let path = std::env::var("ATLAS_ENDPOINT_DIRECTORY_OUTPUT").expect("output path required");
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&crate::proxy::endpoint_registry::list_endpoints()).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
#[ignore = "uses an explicitly supplied Copilot token and consumes a small amount of inference usage"]
async fn live_native_copilot_endpoints() {
    let token = std::env::var("ATLAS_COPILOT_TEST_TOKEN").expect("test token required");
    let endpoint = std::env::var("ATLAS_COPILOT_TEST_ENDPOINT").expect("test endpoint required");
    let chat_id = std::env::var("ATLAS_COPILOT_TEST_CHAT_MODEL").expect("chat model required");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let mut catalog_request = client.get(format!("{endpoint}/models"));
    for (name, value) in
        crate::proxy::providers::copilot_auth::build_copilot_request_headers(&token).unwrap()
    {
        catalog_request = catalog_request.header(name, value);
    }
    let response = catalog_request
        .send()
        .await
        .expect("catalog network request failed");
    assert!(
        response.status().is_success(),
        "catalog HTTP {}",
        response.status()
    );
    let catalog: Value = response.json().await.expect("invalid catalog");
    let models = catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| CopilotModel {
            id: row["id"].as_str().unwrap().into(),
            model_type: row
                .pointer("/capabilities/type")
                .and_then(Value::as_str)
                .unwrap_or("chat")
                .into(),
            model_picker_enabled: row["model_picker_enabled"].as_bool().unwrap_or(false),
            policy_state: row
                .pointer("/policy/state")
                .and_then(Value::as_str)
                .map(str::to_owned),
            supported_endpoints: row["supported_endpoints"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            ..Default::default()
        })
        .collect();
    let fixture = CopilotFixture {
        token,
        endpoint,
        models,
        client,
    };
    TEST_COPILOT.scope(fixture, async {
        let proxy = server(json!({}));
        let mut app = proxy.build_router();
        for (path, body) in [
            ("/v1/systemone", json!({"model":"microsoft-decision-1","state":"I was charged twice.","questions":{"billing":{"type":"noul","instructions":"Is this a billing issue?"}}})),
            ("/v1/chat/completions", json!({"model":chat_id,"messages":[{"role":"user","content":"Reply exactly OK."}],"max_tokens":16,"stream":false})),
        ] {
            let response = tokio::time::timeout(std::time::Duration::from_secs(90), app.call(request(path, &body))).await.expect("inference deadline").unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            assert!(status.is_success(), "{path}: HTTP {status}; synthetic response {}", String::from_utf8_lossy(&bytes));
            let result: Value = serde_json::from_slice(&bytes).unwrap();
            if path == "/v1/systemone" { assert!(result.pointer("/answers/billing/noul").and_then(Value::as_f64).is_some()); }
            else { assert!(result["choices"].as_array().is_some_and(|a| !a.is_empty())); }
            println!("LIVE {path}: HTTP {status}; model={}", body["model"]);
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let logs = proxy.state.db.get_request_logs(&crate::services::usage_stats::LogFilters::default(), 0, 20).unwrap();
                if logs.total >= 2 { break; }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }).await.expect("usage logs were not recorded");
    }).await;
}

#[test]
fn native_eligibility_does_not_broaden_chat_picker_or_trust_arbitrary_paths() {
    let decision = model("decision", "/v1/systemone");
    assert!(!copilot_model_map::is_selectable_model(&decision));
    assert!(copilot_model_map::resolve_endpoint_model(
        "decision",
        &[decision.clone()],
        "/v1/systemone"
    )
    .is_some());
    assert!(
        copilot_model_map::resolve_endpoint_model("decision", &[decision], "/v1/decisions")
            .is_none()
    );
    for endpoint in [
        "https://attacker.example/v1/systemone",
        "/v1/../systemone",
        "/v1/systemone?redirect=1",
    ] {
        let m = model("x", endpoint);
        assert!(copilot_model_map::resolve_endpoint_model("x", &[m], "/v1/systemone").is_none());
    }
}
