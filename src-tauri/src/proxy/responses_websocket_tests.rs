use super::*;
use tokio::net::TcpListener;
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::handshake::server::{Request as HandshakeRequest, Response as HandshakeResponse},
};

#[test]
fn models_must_be_explicit_or_proven_by_a_completed_response() {
    let mut protocol = Protocol::default();
    assert!(protocol
        .prepare(&mut json!({"type":"response.create"}))
        .is_err());
    assert!(protocol
        .prepare(&mut json!({"type":"response.create","previous_response_id":"foreign"}))
        .is_err());
    assert!(protocol
        .prepare(&mut json!({"type":"response.create","model":null}))
        .is_err());
    protocol.started("eligible-model".into());
    assert!(protocol
        .prepare(&mut json!({"type":"response.create","model":"other"}))
        .is_err());
    protocol.observe(&json!({"type":"response.created","response":{"id":"r1"}}));
    assert!(protocol
        .prepare(&mut json!({"type":"response.cancel","response_id":"foreign"}))
        .is_err());
    assert_eq!(
        protocol.prepare(&mut json!({"type":"response.cancel","response_id":"r1"})),
        Ok(false)
    );
    assert!(protocol
        .observe(&json!({"type":"response.completed","response":{"id":"r1"}}))
        .is_some());
    let mut inherited = json!({"type":"response.create","previous_response_id":"r1"});
    assert_eq!(protocol.prepare(&mut inherited), Ok(true));
    assert_eq!(inherited["model"], "eligible-model");
    // prepare does not authorize: production subsequently checks raw ws:/responses
    // eligibility for every create, including this inherited model.
}

#[test]
fn terminal_usage_identity_is_once_per_response_not_once_per_socket() {
    let mut protocol = Protocol::default();
    let mut usage = Vec::new();
    for (id, kind) in [
        ("one", "response.completed"),
        ("two", "response.incomplete"),
        ("three", "response.failed"),
    ] {
        protocol.started("eligible-model".into());
        let event =
            json!({"type":kind,"response":{"id":id,"usage":{"input_tokens":7,"output_tokens":3}}});
        if let Some(response) = protocol.observe(&event) {
            usage.push(TokenUsage::from_codex_response_auto(response).unwrap());
        }
        assert!(protocol.observe(&event).is_none());
    }
    assert_eq!(usage.len(), 3);
    assert_eq!(usage.iter().map(|u| u.input_tokens).sum::<u32>(), 21);
    assert!(protocol.completed.contains_key("one"));
    assert!(!protocol.completed.contains_key("two"));
    assert!(!protocol.completed.contains_key("three"));
    assert!(protocol
        .observe(&json!({"type":"response.completed","response":{"id":"unsolicited"}}))
        .is_none());
}

#[test]
fn rotated_terminal_ids_retire_both_aliases_and_history_is_bounded() {
    let mut protocol = Protocol::default();
    protocol.started("model".into());
    protocol.observe(&json!({"type":"response.created","response":{"id":"created-A"}}));
    assert!(protocol
        .observe(&json!({"type":"response.completed","response":{"id":"terminal-B"}}))
        .is_some());
    assert_eq!(protocol.completed_count, 1);
    assert_eq!(protocol.terminal.len(), 2);
    for alias in ["created-A", "terminal-B"] {
        let mut inherited = json!({"type":"response.create","previous_response_id":alias});
        assert_eq!(protocol.prepare(&mut inherited), Ok(true));
        assert_eq!(inherited["model"], "model");
        assert_eq!(inherited["previous_response_id"], "terminal-B");
    }
    protocol.started("second-model".into());
    protocol.observe(&json!({"type":"response.created","response":{"id":"created-C"}}));
    for stale in ["created-A", "terminal-B"] {
        assert!(protocol
            .observe(&json!({"type":"response.created","response":{"id":stale}}))
            .is_none());
        assert!(protocol
            .observe(&json!({"type":"response.completed","response":{"id":stale}}))
            .is_none());
        assert_eq!(protocol.active_id.as_deref(), Some("created-C"));
        assert_eq!(protocol.active_model.as_deref(), Some("second-model"));
    }
    assert!(protocol
        .observe(&json!({"type":"response.cancelled","response":{"id":"terminal-D"}}))
        .is_some());
    assert!(!protocol.completed.contains_key("created-C"));
    assert!(!protocol.completed.contains_key("terminal-D"));
    protocol.completed_count = MAX_RESPONSES;
    assert!(protocol
        .prepare(&mut json!({"type":"response.create","model":"model"}))
        .is_err());
    assert!(protocol
        .prepare(&mut json!({"type":"session.update","session":{"model":"bypass"}}))
        .is_err());
}

#[test]
fn opaque_ids_have_explicit_individual_and_aggregate_limits() {
    let mut protocol = Protocol::default();
    protocol.started("model".into());
    let created = "a".repeat(1024);
    let terminal = "b".repeat(1024);
    protocol.observe(&json!({"type":"response.created","response":{"id":created}}));
    assert!(protocol
        .observe(&json!({"type":"response.completed","response":{"id":terminal}}))
        .is_some());
    assert!(!protocol.identity_limit_exceeded);
    protocol.started("model".into());
    protocol.observe(
        &json!({"type":"response.created","response":{"id":"x".repeat(MAX_RESPONSE_ID_BYTES + 1)}}),
    );
    assert!(
        protocol.identity_limit_exceeded,
        "Oversized identities must fail explicitly, not skip accounting silently"
    );
    let mut protocol = Protocol::default();
    protocol.retained_id_bytes = MAX_RETAINED_ID_BYTES - 1;
    protocol.started("model".into());
    assert!(protocol
        .observe(&json!({"type":"response.completed","response":{"id":"last"}}))
        .is_some());
    assert!(
        protocol.identity_limit_exceeded,
        "Budget closes only after accounting for this terminal"
    );
}

#[test]
fn endpoint_origin_comes_only_from_https_authenticated_configuration() {
    assert_eq!(
        upstream_url("https://api.example.test").unwrap().as_str(),
        "https://api.example.test/responses"
    );
    assert_eq!(
        upstream_url("https://api.example.test/v1/")
            .unwrap()
            .as_str(),
        "https://api.example.test/v1/responses"
    );
    for bad in [
        "http://example.test",
        "https://user:secret@example.test",
        "https://example.test?target=evil",
        "https://example.test/#fragment",
        "file:///responses",
    ] {
        assert!(upstream_url(bad).is_err(), "{bad}");
    }
}

#[tokio::test]
async fn reqwest_upgrade_uses_proxy_and_preserves_multiple_events_cancel_and_close() {
    // HTTP proxy fixture deliberately has a nonresolvable destination: a direct
    // WebSocket connector bypassing the configured reqwest proxy cannot pass.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_hdr_async(
            stream,
            |request: &HandshakeRequest, response: HandshakeResponse| {
                assert_eq!(request.headers()["authorization"], "Bearer fixture-only");
                assert!(!request.headers().contains_key("cookie"));
                assert_eq!(request.uri().path(), "/responses");
                Ok(response)
            },
        )
        .await
        .unwrap();
        for id in ["first", "second"] {
            let message = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let body: Value = serde_json::from_str(&message).unwrap();
            assert_eq!(body["type"], "response.create");
            assert_eq!(body["model"], "eligible-model");
            socket
                .send(UpMessage::Text(
                    json!({"type":"response.created","response":{"id":id}}).to_string(),
                ))
                .await
                .unwrap();
            if id == "second" {
                let cancel = socket.next().await.unwrap().unwrap().into_text().unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(&cancel).unwrap()["type"],
                    "response.cancel"
                );
            }
            socket.send(UpMessage::Text(json!({"type":"response.completed","response":{"id":id,"usage":{"input_tokens":1,"output_tokens":2}}}).to_string())).await.unwrap();
        }
        socket.close(None).await.unwrap();
        // An ambiguous disconnect must not cause reconnect/replay.
        assert!(timeout(Duration::from_millis(150), listener.accept())
            .await
            .is_err());
    });
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy).unwrap())
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let identity = Identity {
        account: "fixture".into(),
        url: "http://nonexistent.invalid/responses".parse().unwrap(),
        headers: HeaderMap::from_iter([(
            http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer fixture-only"),
        )]),
        fixture: None,
    };
    let mut socket = connect(client, &identity).await.unwrap();
    let mut protocol = Protocol::default();
    for id in ["first", "second"] {
        let mut create = json!({"type":"response.create","model":"eligible-model"});
        assert_eq!(protocol.prepare(&mut create), Ok(true));
        protocol.started("eligible-model".into());
        socket
            .send(UpMessage::Text(create.to_string()))
            .await
            .unwrap();
        let created = socket.next().await.unwrap().unwrap().into_text().unwrap();
        protocol.observe(&serde_json::from_str::<Value>(&created).unwrap());
        if id == "second" {
            let mut cancel = json!({"type":"response.cancel","response_id":id});
            assert_eq!(protocol.prepare(&mut cancel), Ok(false));
            socket
                .send(UpMessage::Text(cancel.to_string()))
                .await
                .unwrap();
        }
        let completed = socket.next().await.unwrap().unwrap().into_text().unwrap();
        let event: Value = serde_json::from_str(&completed).unwrap();
        assert_eq!(event["response"]["id"], id);
        assert!(protocol.observe(&event).is_some());
    }
    assert!(matches!(socket.next().await, Some(Ok(UpMessage::Close(_)))));
    peer.await.unwrap();
}

#[tokio::test]
async fn invalid_upgrade_accept_is_rejected_without_sending_a_request_frame() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/responses", listener.local_addr().unwrap());
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 8192];
        socket.read(&mut request).await.unwrap();
        socket.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: bad\r\n\r\n").await.unwrap();
    });
    let identity = Identity {
        account: "fixture".into(),
        url: url.parse().unwrap(),
        headers: HeaderMap::new(),
        fixture: None,
    };
    assert!(connect(
        reqwest::Client::builder().no_proxy().build().unwrap(),
        &identity
    )
    .await
    .is_err());
    peer.await.unwrap();
}

async fn next_text<S>(socket: &mut WebSocketStream<S>) -> String
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                UpMessage::Text(text) => return text,
                UpMessage::Ping(data) => socket.send(UpMessage::Pong(data)).await.unwrap(),
                other => panic!("Expected text, got {other:?}"),
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn full_relay_authorizes_each_create_logs_two_responses_and_honors_shutdown() {
    use super::super::{
        provider_router::ProviderRouter,
        providers::{codex_chat_history::CodexChatHistoryStore, copilot_auth::CopilotModel},
    };
    use std::sync::{Arc, Mutex};
    use tokio::sync::RwLock;
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/responses",
        upstream_listener.local_addr().unwrap()
    );
    let upstream_task = tokio::spawn(async move {
        let (stream, _) = upstream_listener.accept().await.unwrap();
        let mut ws = accept_hdr_async(
            stream,
            |request: &HandshakeRequest, response: HandshakeResponse| {
                assert_eq!(request.headers()["authorization"], "Bearer fixture-only");
                assert!(!request.headers().contains_key("cookie"));
                Ok(response)
            },
        )
        .await
        .unwrap();
        for id in ["r1", "r2"] {
            let create: Value = serde_json::from_str(&next_text(&mut ws).await).unwrap();
            assert_eq!(create["type"], "response.create");
            assert_eq!(create["model"], "allowed");
            ws.send(UpMessage::Text(
                json!({"type":"response.created","response":{"id":id}}).to_string(),
            ))
            .await
            .unwrap();
            if id == "r2" {
                assert_eq!(create["previous_response_id"], "r1-terminal");
                for stale in ["r1", "r1-terminal"] {
                    ws.send(UpMessage::Text(json!({"type":"response.completed","response":{"id":stale,"usage":{"input_tokens":5,"output_tokens":2}}}).to_string())).await.unwrap();
                }
                let cancel: Value = serde_json::from_str(&next_text(&mut ws).await).unwrap();
                assert_eq!(cancel["type"], "response.cancel");
            }
            let done = json!({"type":"response.completed","response":{"id":format!("{id}-terminal"),"model":"allowed","usage":{"input_tokens":5,"output_tokens":2}}}).to_string();
            ws.send(UpMessage::Text(done.clone())).await.unwrap();
            ws.send(UpMessage::Text(done)).await.unwrap();
        }
        loop {
            match timeout(Duration::from_secs(5), ws.next()).await.unwrap() {
                Some(Ok(UpMessage::Close(_))) | None => break,
                Some(Ok(UpMessage::Ping(data))) => ws.send(UpMessage::Pong(data)).await.unwrap(),
                other => panic!("Unexpected extra request or replay: {other:?}"),
            }
        }
        assert!(
            timeout(Duration::from_millis(100), upstream_listener.accept())
                .await
                .is_err()
        );
    });
    let db = Arc::new(crate::database::Database::memory().unwrap());
    let state = ProxyState {
        db: db.clone(),
        config: Arc::new(RwLock::new(db.get_proxy_config().await.unwrap())),
        status: Arc::new(RwLock::new(Default::default())),
        start_time: Arc::new(RwLock::new(None)),
        current_providers: Arc::new(RwLock::new(HashMap::new())),
        provider_router: Arc::new(ProviderRouter::new(db.clone())),
        codex_chat_history: Arc::new(CodexChatHistoryStore::default()),
        app_handle: None,
    };
    let provider = Provider::with_id("fixture".into(), "Fixture".into(), json!({}));
    db.save_provider("codex", &provider).unwrap();
    let identity = Identity {
        account: "pinned-fixture-account".into(),
        url: url.parse().unwrap(),
        headers: HeaderMap::from_iter([(
            http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer fixture-only"),
        )]),
        fixture: Some(TestFixture {
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            models: vec![
                CopilotModel {
                    id: "allowed".into(),
                    model_type: "chat".into(),
                    model_picker_enabled: true,
                    supported_endpoints: vec!["ws:/responses".into()],
                    ..Default::default()
                },
                CopilotModel {
                    id: "http-only".into(),
                    model_type: "chat".into(),
                    model_picker_enabled: true,
                    supported_endpoints: vec!["/responses".into()],
                    ..Default::default()
                },
            ],
        }),
    };
    let identity = Arc::new(Mutex::new(Some(identity)));
    let (stop, shutdown) = watch::channel(false);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_url = format!("ws://{}/responses", listener.local_addr().unwrap());
    let app = axum::Router::new().route(
        "/responses",
        axum::routing::get(move |upgrade: WebSocketUpgrade| {
            let identity = identity.lock().unwrap().take().unwrap();
            let (state, provider, shutdown) = (state.clone(), provider.clone(), shutdown.clone());
            async move {
                upgrade.on_upgrade(move |socket| {
                    relay(
                        socket,
                        state,
                        provider,
                        identity,
                        HeaderMap::new(),
                        shutdown,
                    )
                })
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (mut client, _) = tokio_tungstenite::connect_async(local_url).await.unwrap();
    let logs = || {
        db.get_request_logs(&crate::services::usage_stats::LogFilters::default(), 0, 20)
            .unwrap()
    };
    assert_eq!(logs().total, 0, "Upgrade-only must not be billed");
    for id in ["r1", "r2"] {
        if id == "r2" {
            let disabled = Provider::with_id(
                "fixture".into(),
                "Fixture".into(),
                json!({"modelCatalog":{"models":[{"model":"allowed","enabled":false}]}}),
            );
            db.save_provider("codex", &disabled).unwrap();
            client
                .send(UpMessage::Text(
                    json!({"type":"response.create","previous_response_id":"r1"}).to_string(),
                ))
                .await
                .unwrap();
            let denied: Value = serde_json::from_str(&next_text(&mut client).await).unwrap();
            assert_eq!(denied["type"], "error");
            assert_eq!(
                logs().total,
                1,
                "Disabling an open connection's model must prevent the next request"
            );
            db.save_provider(
                "codex",
                &Provider::with_id("fixture".into(), "Fixture".into(), json!({})),
            )
            .unwrap();
        }
        // Rejected both before the first response and after the successful one.
        client
            .send(UpMessage::Text(
                json!({"type":"response.create","model":"http-only"}).to_string(),
            ))
            .await
            .unwrap();
        let denied: Value = serde_json::from_str(&next_text(&mut client).await).unwrap();
        assert_eq!(denied["type"], "error");
        let create = if id == "r1" {
            json!({"type":"response.create","model":"allowed"})
        } else {
            json!({"type":"response.create","previous_response_id":"r1"})
        };
        client
            .send(UpMessage::Text(create.to_string()))
            .await
            .unwrap();
        let created: Value = serde_json::from_str(&next_text(&mut client).await).unwrap();
        assert_eq!(created["response"]["id"], id);
        if id == "r2" {
            for stale in ["r1", "r1-terminal"] {
                let event: Value = serde_json::from_str(&next_text(&mut client).await).unwrap();
                assert_eq!(
                    event["response"]["id"], stale,
                    "Native stale events remain unchanged"
                );
                assert_eq!(
                    logs().total,
                    1,
                    "Retired aliases must not consume the active request"
                );
            }
            client
                .send(UpMessage::Text(
                    json!({"type":"response.cancel","response_id":"r2"}).to_string(),
                ))
                .await
                .unwrap();
        }
        let first = next_text(&mut client).await;
        let duplicate = next_text(&mut client).await;
        assert_eq!(
            first, duplicate,
            "Native duplicate event must still be passed through unchanged"
        );
    }
    assert_eq!(
        logs().total,
        2,
        "One usage row per terminal ID, not one per socket/event"
    );
    db.conn
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM providers WHERE id='fixture' AND app_type='codex'",
            [],
        )
        .unwrap();
    client
        .send(UpMessage::Text(
            json!({"type":"response.create","model":"allowed"}).to_string(),
        ))
        .await
        .unwrap();
    let denied: Value = serde_json::from_str(&next_text(&mut client).await).unwrap();
    assert_eq!(
        denied["type"], "error",
        "Deleting pinned provider must fail closed"
    );
    assert_eq!(logs().total, 2);
    stop.send(true).unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            if matches!(client.next().await, Some(Ok(UpMessage::Close(_))) | None) {
                break;
            }
        }
    })
    .await
    .unwrap();
    upstream_task.await.unwrap();
    server.abort();
}

/// Explicitly opt-in: one catalog GET and at most one billable response.create.
/// Credentials are supplied by the operator's environment, never loaded from disk.
#[tokio::test]
#[ignore = "opt-in real Copilot WebSocket test; consumes tokens; requires temporary environment credentials"]
async fn live_copilot_responses_websocket_one_tiny_response() {
    use super::super::{
        provider_router::ProviderRouter,
        providers::{codex_chat_history::CodexChatHistoryStore, copilot_auth::CopilotModel},
    };
    use std::sync::{Arc, Mutex};
    use tokio::sync::RwLock;

    let token =
        std::env::var("ATLAS_COPILOT_TEST_TOKEN").expect("explicit live-test token required");
    let endpoint = std::env::var("ATLAS_COPILOT_TEST_ENDPOINT")
        .expect("explicit live-test HTTPS endpoint required");
    let model_id =
        std::env::var("ATLAS_COPILOT_TEST_MODEL").unwrap_or_else(|_| "gpt-6-luna".into());
    let url = upstream_url(&endpoint).expect("invalid live-test HTTPS API origin");
    let client = super::super::http_client::get_no_redirect()
        .expect("proxy-aware live-test transport unavailable");
    let mut headers = HeaderMap::new();
    for (name, value) in
        build_copilot_request_headers(&token).expect("invalid live-test authentication header")
    {
        headers.insert(name, value);
    }
    drop(token);
    let mut catalog_url = url.clone();
    catalog_url.set_path(&format!(
        "{}/models",
        url.path().trim_end_matches("/responses")
    ));
    let catalog_response = timeout(
        AUTH_TIMEOUT,
        client.get(catalog_url).headers(headers.clone()).send(),
    )
    .await
    .expect("live catalog timeout")
    .expect("live catalog network failure");
    assert!(
        catalog_response.status().is_success(),
        "live catalog HTTP status {}",
        catalog_response.status().as_u16()
    );
    let catalog = timeout(AUTH_TIMEOUT, async {
        let mut bytes = Vec::new();
        let mut chunks = catalog_response.bytes_stream();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|_| "catalog body read failure")?;
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err("catalog too large");
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice::<Value>(&bytes).map_err(|_| "catalog was not JSON")
    })
    .await
    .expect("live catalog body timeout")
    .expect("invalid live catalog body");
    let row = catalog
        .get("data")
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["id"].as_str() == Some(model_id.as_str()))
        })
        .expect("chosen live model absent from raw catalog");
    let effort = ["none", "minimal", "low"].into_iter().find(|candidate| {
        row.pointer("/capabilities/supports/reasoning_effort")
            .and_then(Value::as_array)
            .is_some_and(|levels| {
                levels
                    .iter()
                    .any(|level| level.as_str() == Some(*candidate))
            })
    });
    // Preserve raw catalog eligibility, rather than inventing a WS-enabled fixture.
    let model = CopilotModel {
        id: model_id.clone(),
        model_picker_enabled: row["model_picker_enabled"].as_bool().unwrap_or(false),
        model_type: row
            .pointer("/capabilities/type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        policy_state: row
            .pointer("/policy/state")
            .and_then(Value::as_str)
            .map(str::to_owned),
        supported_endpoints: row["supported_endpoints"]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        ..Default::default()
    };
    assert!(
        super::super::providers::copilot_model_map::resolve_endpoint_model(
            &model_id,
            std::slice::from_ref(&model),
            "ws:/responses"
        )
        .is_some(),
        "raw catalog does not authorize this model for ws:/responses; no billable request sent"
    );
    headers.insert(
        http::header::ORIGIN,
        HeaderValue::from_str(&url.origin().ascii_serialization())
            .expect("invalid authenticated origin"),
    );
    let identity = Arc::new(Mutex::new(Some(Identity {
        account: "explicit-live-test-account".into(),
        url,
        headers,
        fixture: Some(TestFixture {
            client,
            models: vec![model],
        }),
    })));
    let db = Arc::new(crate::database::Database::memory().unwrap());
    let state = ProxyState {
        db: db.clone(),
        config: Arc::new(RwLock::new(db.get_proxy_config().await.unwrap())),
        status: Arc::new(RwLock::new(Default::default())),
        start_time: Arc::new(RwLock::new(None)),
        current_providers: Arc::new(RwLock::new(HashMap::new())),
        provider_router: Arc::new(ProviderRouter::new(db.clone())),
        codex_chat_history: Arc::new(CodexChatHistoryStore::default()),
        app_handle: None,
    };
    let provider = Provider::with_id(
        "explicit-live-websocket-test".into(),
        "Live WebSocket Test".into(),
        json!({}),
    );
    db.save_provider("codex", &provider).unwrap();
    let (stop, shutdown) = watch::channel(false);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_url = format!("ws://{}/responses", listener.local_addr().unwrap());
    let app = axum::Router::new().route(
        "/responses",
        axum::routing::get(move |upgrade: WebSocketUpgrade| {
            let identity = identity
                .lock()
                .unwrap()
                .take()
                .expect("live test permits one connection only");
            let (state, provider, shutdown) = (state.clone(), provider.clone(), shutdown.clone());
            async move {
                upgrade
                    .max_message_size(MAX_MESSAGE)
                    .max_frame_size(MAX_MESSAGE)
                    .on_upgrade(move |socket| {
                        relay(
                            socket,
                            state,
                            provider,
                            identity,
                            HeaderMap::new(),
                            shutdown,
                        )
                    })
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let outcome = timeout(Duration::from_secs(180), async {
        let (mut socket, _) = tokio_tungstenite::connect_async(local_url)
            .await
            .map_err(|_| "local_upgrade_failed")?;
        let mut request = json!({"type":"response.create","model":model_id,
            "input":[{"role":"user","content":"Reply exactly OK"}],
            "max_output_tokens":512});
        if let Some(effort) = effort {
            request["reasoning"] = json!({"effort":effort});
        }
        // One attempt only, including when a write's outcome is ambiguous.
        socket
            .send(UpMessage::Text(request.to_string()))
            .await
            .map_err(|_| "request_write_failed")?;
        let mut output = String::new();
        loop {
            match socket.next().await {
                Some(Ok(UpMessage::Text(text))) => {
                    let event: Value = serde_json::from_str(&text).map_err(|_| "non_json_event")?;
                    match event["type"].as_str().unwrap_or_default() {
                        "response.output_text.delta" => {
                            if let Some(delta) = event["delta"].as_str() {
                                if output.len() + delta.len() > 8192 {
                                    return Err("unexpected_output_size");
                                }
                                output.push_str(delta);
                            }
                        }
                        "response.completed" => {
                            if output.is_empty() {
                                if let Some(items) =
                                    event.pointer("/response/output").and_then(Value::as_array)
                                {
                                    for item in items {
                                        if let Some(parts) = item["content"].as_array() {
                                            for part in parts {
                                                if part["type"] == "output_text" {
                                                    if let Some(text) = part["text"].as_str() {
                                                        output.push_str(text);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            return if output.trim() == "OK" {
                                Ok(())
                            } else {
                                Err("completed_but_not_exact_ok")
                            };
                        }
                        "error" => return Err(match event.pointer("/error/code").and_then(Value::as_str) {
                            Some("atlas_websocket_error") => match event.pointer("/error/message").and_then(Value::as_str) {
                                Some("Pinned provider no longer exists") => "local_pinned_provider_missing",
                                Some("Pinned provider settings unavailable") => "local_provider_settings_unavailable",
                                Some("Model is disabled in provider settings") => "local_model_disabled",
                                Some("Model does not advertise ws:/responses for this account") => "local_model_ineligible",
                                Some("Upstream did not accept the native Responses WebSocket protocol") => "upstream_handshake_rejected",
                                Some("Upstream response identity limit exceeded") => "upstream_identity_limit",
                                _ => "local_atlas_error_event",
                            },
                            Some("model_not_found") => "upstream_model_not_found",
                            Some("rate_limit_exceeded") => "upstream_rate_limited",
                            Some("insufficient_quota") => "upstream_insufficient_quota",
                            Some("invalid_request_error") => "upstream_invalid_request",
                            _ => "upstream_error_event",
                        }),
                        "response.failed" => return Err("terminal_response_failed"),
                        "response.incomplete" => return Err("terminal_response_incomplete"),
                        "response.cancelled" => return Err("terminal_response_cancelled"),
                        _ => {}
                    }
                }
                Some(Ok(UpMessage::Ping(data))) => socket
                    .send(UpMessage::Pong(data))
                    .await
                    .map_err(|_| "pong_failed")?,
                Some(Ok(UpMessage::Pong(_))) => {}
                _ => return Err("closed_before_terminal"),
            }
        }
    })
    .await
    .unwrap_or(Err("live_deadline_exceeded"));
    let _ = stop.send(true);
    server.abort();
    let logs = db
        .get_request_logs(&crate::services::usage_stats::LogFilters::default(), 0, 10)
        .unwrap();
    // Only fixed result labels/counts are printed; no credentials, headers, IDs,
    // upstream messages, catalog bodies, or generated content can reach stdout.
    eprintln!(
        "live Responses WebSocket result={} usage_rows={}",
        outcome.err().unwrap_or("completed_exact_ok"),
        logs.total
    );
    assert!(
        outcome.is_ok(),
        "live Responses WebSocket did not complete exactly OK (scrubbed result printed)"
    );
    assert_eq!(logs.total, 1, "expected one terminal usage record");
}
