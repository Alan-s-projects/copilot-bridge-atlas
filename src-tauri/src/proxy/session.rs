//! Extract a stable Codex session identity for usage and cache affinity.
use axum::http::HeaderMap;
use uuid::Uuid;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIdSource {
    MetadataSessionId,
    Header,
    Generated,
}
#[derive(Debug, Clone)]
pub struct SessionIdResult {
    pub session_id: String,
    pub source: SessionIdSource,
    pub client_provided: bool,
}
pub fn extract_session_id(headers: &HeaderMap, body: &serde_json::Value) -> SessionIdResult {
    extract_responses_session(headers, body, "codex").unwrap_or_else(generate_new_session_id)
}
fn extract_responses_session(
    headers: &HeaderMap,
    body: &serde_json::Value,
    prefix: &str,
) -> Option<SessionIdResult> {
    // 1. Extract from headers
    let header_names = &["session_id", "x-session-id"];
    for header_name in header_names {
        if let Some(value) = headers.get(*header_name) {
            if let Ok(session_id) = value.to_str() {
                let session_id = session_id.trim();
                // Responses The client's Session ID is usually longer (UUID format)
                if session_id.len() > 20 {
                    return Some(SessionIdResult {
                        session_id: format!("{prefix}_{session_id}"),
                        source: SessionIdSource::Header,
                        client_provided: true,
                    });
                }
            }
        }
    }

    // 2. Extract from body.metadata.session_id
    if let Some(session_id) = body
        .get("metadata")
        .and_then(|m| m.get("session_id"))
        .and_then(|v| v.as_str())
    {
        let session_id = session_id.trim();
        if session_id.len() > 10 {
            return Some(SessionIdResult {
                session_id: format!("{prefix}_{session_id}"),
                source: SessionIdSource::MetadataSessionId,
                client_provided: true,
            });
        }
    }

    // previous_response_id is the response cursor in the Responses protocol, not a stable session identity.
    // When Chat/Responses is bridged, this value usually comes from the random response id returned by the upstream in each round;
    // If it is used as prompt_cache_key or Codex session header, it will cause the cache key to be changed in each round of requests.

    None
}

fn generate_new_session_id() -> SessionIdResult {
    SessionIdResult {
        session_id: Uuid::new_v4().to_string(),
        source: SessionIdSource::Generated,
        client_provided: false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn test_codex_previous_response_id_is_not_stable_session_identity() {
        let headers = HeaderMap::new();
        let body = json!({
            "input": "Write a function",
            "previous_response_id": "resp_abc123def456789"
        });

        let result = extract_session_id(&headers, &body);

        assert!(!result.session_id.is_empty());
        assert_eq!(result.source, SessionIdSource::Generated);
        assert!(!result.client_provided);
    }
    #[test]
    fn test_codex_keeps_existing_response_session_headers() {
        let body = json!({ "input": "Write a function" });

        for header_name in ["session_id", "x-session-id"] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header_name,
                "d937243f-2702-4f20-97b6-c9682235ab81".parse().unwrap(),
            );

            let result = extract_session_id(&headers, &body);

            assert_eq!(
                result.session_id,
                "codex_d937243f-2702-4f20-97b6-c9682235ab81"
            );
            assert_eq!(result.source, SessionIdSource::Header);
            assert!(result.client_provided);
        }
    }
}
