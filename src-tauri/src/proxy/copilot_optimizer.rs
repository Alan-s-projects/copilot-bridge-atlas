//! Copilot request classification and stable interaction identity for Codex.
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;
#[derive(Debug, Clone)]
pub struct CopilotClassification {
    pub initiator: &'static str,
}
pub fn classify_responses_request(body: &Value) -> CopilotClassification {
    let initiator_for_item = |item: &Value| {
        let item_type = item.get("type").and_then(Value::as_str);
        if item_type.is_some_and(super::providers::codex_chat_history::is_call_output_item_type) {
            return Some("agent");
        }
        match item.get("role").and_then(Value::as_str) {
            Some("user") => Some("user"),
            Some("assistant" | "system" | "developer") => None,
            _ => match item_type {
                Some("reasoning" | "function_call" | "custom_tool_call" | "tool_search_call") => {
                    None
                }
                _ => Some("user"),
            },
        }
    };
    // Full-history requests can contain old tool outputs before a new user turn.
    let initiator = match body.get("input") {
        Some(Value::Array(items)) => items.iter().rev().find_map(initiator_for_item),
        Some(item @ Value::Object(_)) => initiator_for_item(item),
        _ => None,
    }
    .unwrap_or("user");

    CopilotClassification { initiator }
}

pub fn deterministic_interaction_id(session_id: &str) -> Option<String> {
    if session_id.is_empty() {
        return None;
    }

    let mut hasher = Sha256::new();
    hasher.update(b"interaction:");
    hasher.update(session_id.as_bytes());
    let result = hasher.finalize();

    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&result[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 1

    Some(Uuid::from_bytes(bytes).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn responses_classification_uses_the_latest_user_or_tool_input() {
        let tool = json!({
            "type": "function_call_output",
            "call_id": "call-1",
            "output": "done"
        });
        let user = json!({"role": "user", "content": "A new question"});
        for (input, expected) in [
            (json!([user.clone(), tool.clone()]), "agent"),
            (json!([tool.clone(), user.clone()]), "user"),
            (
                json!([
                    user.clone(),
                    tool.clone(),
                    {"type": "reasoning", "summary": []},
                    {"role": "developer", "content": "Use the output"}
                ]),
                "agent",
            ),
            (
                json!([
                    tool.clone(),
                    user,
                    {"role": "system", "content": "Continue"}
                ]),
                "user",
            ),
            (tool.clone(), "agent"),
            (json!("A new question"), "user"),
            (json!([]), "user"),
            (json!(null), "user"),
            (json!([tool, {"type": "unknown"}]), "user"),
        ] {
            let body = json!({"input": input, "previous_response_id": "resp-previous"});
            let result = classify_responses_request(&body);
            assert_eq!(result.initiator, expected, "{body}");
        }
    }
    #[test]
    fn test_interaction_id_stable_for_same_session() {
        let id1 = deterministic_interaction_id("session_abc");
        let id2 = deterministic_interaction_id("session_abc");
        assert_eq!(id1, id2);
    }
    #[test]
    fn test_interaction_id_differs_across_sessions() {
        let id1 = deterministic_interaction_id("session_abc");
        let id2 = deterministic_interaction_id("session_def");
        assert_ne!(id1, id2);
    }
    #[test]
    fn test_interaction_id_empty_session_is_none() {
        // Interaction ID should not be generated when there is no session (to avoid fragmentation)
        assert!(deterministic_interaction_id("").is_none());
    }
    #[test]
    fn test_interaction_id_is_valid_uuid() {
        let id = deterministic_interaction_id("test_session").unwrap();
        assert!(Uuid::parse_str(&id).is_ok());
    }
}
