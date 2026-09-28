//! Parse token usage from OpenAI Responses and Chat Completions.

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn openai_cache_read_tokens(usage: &Value) -> u32 {
    usage
        .get("cache_read_input_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .or_else(|| usage.pointer("/prompt_tokens_details/cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32
}

fn openai_cache_write_tokens(usage: &Value) -> u32 {
    usage
        .get("cache_creation_input_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cache_write_tokens"))
        .or_else(|| usage.pointer("/prompt_tokens_details/cache_write_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32
}

fn response_id(body: &Value, field: &str) -> Option<String> {
    body.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Token usage statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    /// The actual model name extracted from the response (if available)
    pub model: Option<String>,
    /// Message ID extracted from response (used for cross-origin deduplication)
    ///
    #[serde(skip)]
    pub message_id: Option<String>,
}

impl TokenUsage {
    /// Scope upstream response identities to the client and provider.
    pub fn dedup_request_id(&self, app_type: &str, provider_id: &str) -> String {
        self.message_id
            .as_ref()
            .map(|message_id| format!("session:{app_type}:{provider_id}:{message_id}"))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
    }

    /// Whether a token of any billing dimension is generated.
    ///
    /// Empty usage for filtering all 0s before writing: When OpenAI compatible upstream omits usage under streaming,
    /// The converter will synthesize an all-0 termination event. If there is no message_id, it will be `dedup_request_id`.
    /// It degenerates into a random UUID, causing each request to insert a meaningless blank line and inflate the number of requests.
    pub fn has_billable_tokens(&self) -> bool {
        self.input_tokens > 0
            || self.output_tokens > 0
            || self.cache_read_tokens > 0
            || self.cache_creation_tokens > 0
    }
}

impl TokenUsage {
    /// Parsing non-streaming responses from Codex API
    pub fn from_codex_response(body: &Value) -> Option<Self> {
        let usage = body.get("usage");
        if usage.is_none() {
            log::debug!(
                "[Codex] There is no usage field in the response, body keys: {:?}",
                body.as_object().map(|o| o.keys().collect::<Vec<_>>())
            );
            return None;
        }
        let usage = usage?;

        let input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64());
        let output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64());

        if input_tokens.is_none() || output_tokens.is_none() {
            log::debug!(
                "[Codex] usage field is missing input_tokens or output_tokens, usage: {usage:?}"
            );
            return None;
        }

        // Extract model name from response
        let model = body
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let cached_tokens = openai_cache_read_tokens(usage);
        let cache_write_tokens = openai_cache_write_tokens(usage);

        Some(Self {
            input_tokens: input_tokens? as u32,
            output_tokens: output_tokens? as u32,
            cache_read_tokens: cached_tokens,
            cache_creation_tokens: cache_write_tokens,
            model,
            message_id: response_id(body, "id"),
        })
    }

    /// Smart Codex response parsing - automatically detects OpenAI or Codex formats
    ///
    /// Codex supports two API formats:
    /// - `/v1/responses`: use input_tokens/output_tokens
    /// - `/v1/chat/completions`: use prompt_tokens/completion_tokens (OpenAI format)
    ///
    /// Note: Record the original input_tokens and subtract the cached_tokens when calculating the fee.
    pub fn from_codex_response_auto(body: &Value) -> Option<Self> {
        let usage = body.get("usage")?;

        // Detection format: OpenAI uses prompt_tokens, Codex uses input_tokens
        if usage.get("prompt_tokens").is_some() {
            log::debug!("[Codex] OpenAI format detected (prompt_tokens)");
            Self::from_openai_response(body)
        } else if usage.get("input_tokens").is_some() {
            log::debug!("[Codex] Codex format detected (input_tokens)");
            // Use the non-adjusted version, logging the original input_tokens
            Self::from_codex_response(body)
        } else {
            log::debug!("[Codex] Unable to recognize response format, usage: {usage:?}");
            None
        }
    }

    /// Intelligent Codex streaming response parsing - automatically detects Codex Responses / Images / OpenAI formats
    pub fn from_codex_stream_events_auto(events: &[Value]) -> Option<Self> {
        log::debug!(
            "[Codex] Intelligent analysis of streaming events, a total of {} events",
            events.len()
        );

        // Incomplete/failed native Responses can still report billable tokens.
        // Prefer the final reported usage without inventing any missing counts.
        for event in events.iter().rev() {
            if matches!(
                event.get("type").and_then(Value::as_str),
                Some("response.completed" | "response.incomplete" | "response.failed")
            ) {
                if let Some(usage) = event
                    .get("response")
                    .and_then(Self::from_codex_response_auto)
                {
                    return Some(usage);
                }
            }
        }

        // Images API streaming format (image_generation.completed event): usage hangs directly on
        // At the top level of the event, the field form is consistent with the Codex non-streaming response; taking the last one in reverse order can follow this form
        // Parsed events, skip the previous partial_image events without usage. When the analysis is not established
        // Continue to follow the OpenAI rollback below without changing the existing path.
        if let Some(usage) = events
            .iter()
            .rev()
            .filter(|event| event.pointer("/usage/input_tokens").is_some())
            .find_map(Self::from_codex_response)
        {
            log::debug!("[Codex] Find the top-level usage.input_tokens event");
            return Some(usage);
        }

        // Fallback to OpenAI Chat Completions format (last chunk contains usage)
        log::debug!("[Codex] Try the OpenAI streaming format");
        Self::from_openai_stream_events(events)
    }

    /// Parsing (prompt_tokens, completion_tokens) responses from OpenAI Chat Completions API
    pub fn from_openai_response(body: &Value) -> Option<Self> {
        let usage = body.get("usage")?;

        // OpenAI uses prompt_tokens and completion_tokens
        let prompt_tokens = usage.get("prompt_tokens").and_then(|v| v.as_u64())?;
        let completion_tokens = usage.get("completion_tokens").and_then(|v| v.as_u64())?;

        // Get cached_tokens (possibly in prompt_tokens_details)
        let cached_tokens = openai_cache_read_tokens(usage);
        let cache_write_tokens = openai_cache_write_tokens(usage);

        // Extract model name from response
        let model = body
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Some(Self {
            input_tokens: prompt_tokens as u32,
            output_tokens: completion_tokens as u32,
            cache_read_tokens: cached_tokens,
            cache_creation_tokens: cache_write_tokens,
            model,
            message_id: response_id(body, "id"),
        })
    }

    /// Streaming response parsing from the OpenAI Chat Completions API
    pub fn from_openai_stream_events(events: &[Value]) -> Option<Self> {
        log::debug!(
            "[Codex] Parse OpenAI streaming events, total {} events",
            events.len()
        );
        // OpenAI streaming response contains usage in last chunk
        for event in events.iter().rev() {
            if let Some(usage) = event.get("usage") {
                if !usage.is_null() {
                    log::debug!("[Codex] found usage: {usage:?}");
                    let mut parsed = Self::from_openai_response(event)?;
                    if parsed.message_id.is_none() {
                        parsed.message_id =
                            events.iter().find_map(|chunk| response_id(chunk, "id"));
                    }
                    return Some(parsed);
                }
            }
        }
        log::debug!("[Codex] usage information not found");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn response_ids_produce_scoped_dedup_keys_and_empty_ids_fall_back() {
        let response = json!({
            "id": "resp_123",
            "model": "gpt-5.6",
            "usage": { "input_tokens": 10, "output_tokens": 2 }
        });
        let usage = TokenUsage::from_codex_response(&response).unwrap();
        assert_eq!(usage.message_id.as_deref(), Some("resp_123"));
        assert_eq!(
            usage.dedup_request_id("codex", "provider-a"),
            "session:codex:provider-a:resp_123"
        );

        let empty = json!({
            "id": "",
            "usage": { "input_tokens": 10, "output_tokens": 2 }
        });
        let empty_usage = TokenUsage::from_codex_response(&empty).unwrap();
        assert!(empty_usage.message_id.is_none());
        assert!(!empty_usage
            .dedup_request_id("codex", "provider-a")
            .starts_with("session:"));
    }

    #[test]
    fn test_has_billable_tokens_gates_empty_usage() {
        // All-zero usage (such as the all-zero termination event synthesized when usage is omitted from the upstream) should not be billed -
        // This is the basis for the codex streaming empty line multi-repair fix (D).
        assert!(!TokenUsage::default().has_billable_tokens());
        // Only cache_read is also a real billing token and must be counted.
        let only_cache = TokenUsage {
            cache_read_tokens: 100,
            ..Default::default()
        };
        assert!(only_cache.has_billable_tokens());
        let normal = TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        };
        assert!(normal.has_billable_tokens());
    }

    #[test]
    fn test_codex_response_auto_returns_some_for_synthetic_all_zero() {
        // P3 regression: Converter synthesizes all 0 usage when usage is omitted in upstream non-streaming Chat, from_codex_response_auto
        // Still returns Some (field exists, no positivity check) - proves that handlers must use has_billable_tokens
        // The gate can block empty lines, `if let Some` alone is not enough.
        let synthetic = json!({
            "usage": { "input_tokens": 0, "output_tokens": 0, "total_tokens": 0 }
        });
        let usage = TokenUsage::from_codex_response_auto(&synthetic)
            .expect("When all 0 usage fields are present from_codex_response_auto returns Some");
        assert!(
            !usage.has_billable_tokens(),
            "All 0 usage must be judged as non-billable by has_billable_tokens and skipped by the handlers gate."
        );
    }

    #[test]
    fn test_codex_response_parsing_cached_tokens_in_details() {
        let response = json!({
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 500,
                "input_tokens_details": {
                    "cached_tokens": 300
                }
            }
        });

        let usage = TokenUsage::from_codex_response(&response).unwrap();
        // Non-tuned mode: input_tokens remain at their original values, but cache hits should be logged
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.output_tokens, 500);
        assert_eq!(usage.cache_read_tokens, 300);
    }

    #[test]
    fn test_codex_response_parsing_cache_write_tokens_in_details() {
        let response = json!({
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 500,
                "input_tokens_details": {
                    "cached_tokens": 300,
                    "cache_write_tokens": 200
                }
            }
        });

        let usage = TokenUsage::from_codex_response(&response).unwrap();
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.cache_read_tokens, 300);
        assert_eq!(usage.cache_creation_tokens, 200);
    }

    // ============================================================================
    // Smart Codex parsing test
    // ============================================================================

    #[test]
    fn test_codex_response_auto_openai_format() {
        // OpenAI format (prompt_tokens/completion_tokens)
        let response = json!({
            "model": "gpt-4o",
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 500,
                "prompt_tokens_details": {
                    "cached_tokens": 200
                }
            }
        });

        let usage = TokenUsage::from_codex_response_auto(&response).unwrap();
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.output_tokens, 500);
        assert_eq!(usage.cache_read_tokens, 200);
        assert_eq!(usage.model, Some("gpt-4o".to_string()));
    }

    #[test]
    fn test_codex_response_auto_codex_format() {
        // Codex format (input_tokens/output_tokens)
        let response = json!({
            "model": "gpt-5.4",
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 500,
                "input_tokens_details": {
                    "cached_tokens": 300
                }
            }
        });

        let usage = TokenUsage::from_codex_response_auto(&response).unwrap();
        // Record original input_tokens, do not adjust
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.output_tokens, 500);
        assert_eq!(usage.cache_read_tokens, 300);
        assert_eq!(usage.model, Some("gpt-5.4".to_string()));
    }

    #[test]
    fn test_codex_stream_events_auto_codex_format() {
        // Codex Responses API streaming format (response.completed event)
        let events = vec![
            json!({
                "type": "response.created",
                "response": {
                    "id": "resp_123"
                }
            }),
            json!({
                "type": "response.completed",
                "response": {
                    "model": "gpt-5.4",
                    "usage": {
                        "input_tokens": 1000,
                        "output_tokens": 500,
                        "input_tokens_details": {
                            "cached_tokens": 200
                        }
                    }
                }
            }),
        ];

        let usage = TokenUsage::from_codex_stream_events_auto(&events).unwrap();
        // Record original input_tokens, do not adjust
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.output_tokens, 500);
        assert_eq!(usage.cache_read_tokens, 200);
        assert_eq!(usage.model, Some("gpt-5.4".to_string()));
    }

    #[test]
    fn test_codex_stream_events_auto_openai_format() {
        // OpenAI Chat Completions streaming format (the last chunk contains usage)
        let events = vec![
            json!({
                "id": "chatcmpl-123",
                "model": "gpt-4o",
                "choices": [{"delta": {"content": "Hello"}}]
            }),
            json!({
                "id": "chatcmpl-123",
                "model": "gpt-4o",
                "choices": [{"delta": {}}],
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 50
                }
            }),
        ];

        let usage = TokenUsage::from_codex_stream_events_auto(&events).unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.model, Some("gpt-4o".to_string()));
    }

    #[test]
    fn test_codex_stream_events_auto_image_generation_completed() {
        // Images API streaming format: usage hangs on top of image_generation.completed event,
        // The field shape is consistent with the Codex non-streaming response (input_tokens / output_tokens)
        let events = vec![
            json!({
                "type": "image_generation.partial_image",
                "b64_json": "cGFydGlhbA==",
                "partial_image_index": 0
            }),
            json!({
                "type": "image_generation.completed",
                "b64_json": "aW1hZ2U=",
                "created_at": 1778832973,
                "usage": {
                    "input_tokens": 1474,
                    "input_tokens_details": {
                        "image_tokens": 1457,
                        "text_tokens": 17
                    },
                    "output_tokens": 1372,
                    "output_tokens_details": {
                        "image_tokens": 1372,
                        "text_tokens": 0
                    },
                    "total_tokens": 2846
                }
            }),
        ];

        let usage = TokenUsage::from_codex_stream_events_auto(&events)
            .expect("image_generation.completed usage should be parsed");
        assert_eq!(usage.input_tokens, 1474);
        assert_eq!(usage.output_tokens, 1372);
        assert_eq!(usage.cache_read_tokens, 0);
        assert_eq!(usage.cache_creation_tokens, 0);
        assert_eq!(usage.model, None);
    }
}
