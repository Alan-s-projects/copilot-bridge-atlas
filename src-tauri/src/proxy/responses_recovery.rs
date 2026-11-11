//! Bounded replay recovery for Copilot state issued to another account/connection.
//! Never edit the client's saved history or discard an opaque compaction.
use serde_json::Value;

pub(super) fn prepare_retry(body: &Value, status: u16, error: &str) -> Option<Value> {
    if !matches!(status, 400 | 401) || error.len() > 64 * 1024 {
        return None;
    }
    let error: Value = serde_json::from_str(error).ok()?;
    let error = error.get("error")?.as_object()?;
    if ["code", "type"].iter().any(|key| {
        error
            .get(*key)
            .is_some_and(|value| !value.is_null() && value.as_str() != Some(""))
    }) {
        return None;
    }
    let message = error.get("message")?.as_str()?.trim();
    if !message.eq_ignore_ascii_case("input item does not belong to this connection")
        && !message.eq_ignore_ascii_case("input item ID does not belong to this connection")
    {
        return None;
    }
    // A linked request may omit earlier context. Removing its link cannot rebuild it.
    if ["previous_response_id", "conversation"]
        .iter()
        .any(|key| body.get(*key).is_some_and(|value| !value.is_null()))
    {
        return None;
    }
    let input = body.get("input")?.as_array()?;
    // Only replay self-contained, known item types. In particular, compaction,
    // item_reference and opaque tool content must not silently disappear.
    for item in input {
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => {}
            None
            | Some(
                "message"
                | "function_call"
                | "function_call_output"
                | "custom_tool_call"
                | "custom_tool_call_output"
                | "web_search_call",
            ) if !contains_encrypted_content(item) => {}
            _ => return None,
        }
    }
    let mut retry = body.clone();
    let items = retry.get_mut("input")?.as_array_mut()?;
    items.retain(|item| item.get("type").and_then(Value::as_str) != Some("reasoning"));
    for item in items {
        // call_id is a durable call/result pairing key, not an upstream item ID.
        item.as_object_mut()?.remove("id");
    }
    (retry != *body).then_some(retry)
}

fn contains_encrypted_content(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key("encrypted_content")
                || object.values().any(contains_encrypted_content)
        }
        Value::Array(values) => values.iter().any(contains_encrypted_content),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rejection(message: &str) -> String {
        json!({"error":{"message":message,"code":""}}).to_string()
    }

    fn history() -> Value {
        json!({
            "model":"gpt-6-astra", "stream":true, "reasoning":{"effort":"high"},
            "input":[
                {"role":"user","content":"Remember the previous result."},
                {"type":"reasoning","id":"opaque-r","encrypted_content":"opaque"},
                {"type":"message","role":"assistant","id":"opaque-m","phase":"commentary",
                 "content":[{"type":"output_text","text":"Checking."}]},
                {"type":"function_call","id":"opaque-f","call_id":"call_1","name":"read","arguments":"{}"},
                {"type":"function_call_output","id":"opaque-o","call_id":"call_1","output":"original result"},
                {"type":"web_search_call","id":"opaque-w","status":"completed",
                 "action":{"type":"search","query":"example"}},
                {"role":"user","content":"Continue."}
            ]
        })
    }

    #[test]
    fn ownership_recovery_preserves_visible_history_and_pairing_without_mutating_source() {
        for status in [400, 401] {
            for message in [
                "input item does not belong to this connection",
                "input item ID does not belong to this connection",
            ] {
                let original = history();
                let snapshot = original.clone();
                let retry = prepare_retry(&original, status, &rejection(message)).unwrap();
                let mut expected = original.clone();
                expected["input"].as_array_mut().unwrap().remove(1);
                for item in expected["input"].as_array_mut().unwrap() {
                    item.as_object_mut().unwrap().remove("id");
                }
                assert_eq!(retry, expected);
                assert_eq!(original, snapshot);
                assert!(prepare_retry(&retry, status, &rejection(message)).is_none());
            }
        }
    }

    #[test]
    fn refuses_opaque_or_linked_history() {
        let error = rejection("input item does not belong to this connection");
        for item in [
            json!({"type":"compaction","encrypted_content":"only copy of earlier context"}),
            json!({"type":"item_reference","id":"opaque"}),
            json!({"type":"agent_message","content":"unknown replay semantics"}),
            json!({"type":"function_call_output","call_id":"call_1",
                   "output":[{"type":"encrypted_content","encrypted_content":"opaque"}]}),
        ] {
            let mut body = history();
            body["input"].as_array_mut().unwrap().push(item);
            assert!(prepare_retry(&body, 401, &error).is_none());
        }
        for key in ["previous_response_id", "conversation"] {
            let mut body = history();
            body[key] = json!("upstream-context");
            assert!(prepare_retry(&body, 401, &error).is_none());
        }
    }

    #[test]
    fn genuine_auth_and_unrelated_errors_do_not_trigger_replay() {
        let exact = rejection("input item does not belong to this connection");
        for status in [200, 403, 429, 500] {
            assert!(prepare_retry(&history(), status, &exact).is_none());
        }
        for error in [
            "Unauthorized".to_string(),
            rejection("Invalid token"),
            rejection("input item does not belong to this connection; invalid credentials"),
            json!({"error":{"message":"input item does not belong to this connection","code":"invalid_token"}}).to_string(),
            json!({"error":{"message":"input item does not belong to this connection","type":"authentication_error"}}).to_string(),
        ] {
            assert!(prepare_retry(&history(), 401, &error).is_none());
        }
    }
}
