//! Context-overflow errors must use wording recognized by Claude Code's recovery path.
use serde_json::Value;

pub(crate) fn context_overflow_message(value: &Value) -> Option<String> {
    let response = value.get("response").unwrap_or(value);
    let error = response.get("error").unwrap_or(response);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .unwrap_or("");
    let message = if message.starts_with("Copilot Bridge Atlas local proxy failed ") {
        message
            .split_once("; cause: ")
            .map(|(_, cause)| cause)
            .unwrap_or(message)
    } else {
        message
    };
    let lower = message.to_ascii_lowercase();
    let code = error.get("code").and_then(Value::as_str);
    let recognized = matches!(
        code,
        Some("context_length_exceeded" | "model_max_prompt_tokens_exceeded")
    ) || lower.starts_with("prompt is too long")
        || lower.starts_with("input is too long for requested model")
        || lower.starts_with("your input exceeds the context window of this model")
        || (lower.starts_with("prompt token count of ")
            && lower.contains(" exceeds the limit of "));
    if !recognized {
        return None;
    }

    let counts = lower
        .strip_prefix("prompt token count of ")
        .and_then(|text| text.split_once(" exceeds the limit of "))
        .or_else(|| {
            lower.strip_prefix("prompt is too long: ").and_then(|text| {
                let (tokens, limit) = text.split_once(" tokens > ")?;
                Some((tokens, limit.strip_suffix(" maximum")?))
            })
        })
        .and_then(|(tokens, limit)| {
            let tokens = tokens.parse::<u64>().ok()?;
            let limit = limit.parse::<u64>().ok()?;
            (tokens > limit && limit > 0).then_some((tokens, limit))
        });
    Some(match counts {
        Some((tokens, limit)) => format!("prompt is too long: {tokens} tokens > {limit} maximum"),
        None => "prompt is too long".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_known_overflow_shapes_without_inventing_counts() {
        for value in [
            json!({"error":{"code":"context_length_exceeded"}}),
            json!({"error":{"code":"model_max_prompt_tokens_exceeded","message":"Prompt exceeds limit"}}),
            json!({"error":{"message":"Your input exceeds the context window of this model. Please adjust your input and try again."}}),
            json!({"response":{"error":{"message":"Input is too long for requested model"}}}),
            json!({"error":{"message":"Copilot Bridge Atlas local proxy failed while handling Codex endpoint /responses. Provider: GitHub Copilot; model: gpt-6-astra; upstream_status: HTTP 400; cause: Your input exceeds the context window of this model. Please adjust your input and try again."}}),
            json!({"error":{"message":"prompt is too long: 999999999999999999999 tokens > 200000 maximum"}}),
        ] {
            assert_eq!(
                context_overflow_message(&value).as_deref(),
                Some("prompt is too long"),
                "{value}"
            );
        }
    }

    #[test]
    fn preserves_only_valid_upstream_counts() {
        for message in [
            "prompt token count of 210000 exceeds the limit of 200000",
            "prompt is too long: 210000 tokens > 200000 maximum",
        ] {
            assert_eq!(
                context_overflow_message(&json!({"error":{"message":message}})).unwrap(),
                "prompt is too long: 210000 tokens > 200000 maximum"
            );
        }
        for message in [
            "prompt is too long: 10 tokens > 200000 maximum",
            "prompt is too long: 210000 tokens > 0 maximum",
            "prompt is too long: 210000 tokens > 200000 maximum. Other details",
        ] {
            assert_eq!(
                context_overflow_message(&json!({"error":{"message":message}})).unwrap(),
                "prompt is too long"
            );
        }
    }

    #[test]
    fn unrelated_failures_and_successful_context_stop_are_not_overflow_errors() {
        for value in [
            json!({"error":{"code":"invalid_request_body","message":"Invalid schema for function Read"}}),
            json!({"error":{"message":"Too many images"}}),
            json!({"error":{"message":"Rate limit exceeded"}}),
            json!({"error":{"message":"Invalid schema: description contains prompt is too long"}}),
            json!({"stop_reason":"model_context_window_exceeded"}),
            json!({"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}),
            Value::Null,
        ] {
            assert_eq!(context_overflow_message(&value), None, "{value}");
        }
    }
}
