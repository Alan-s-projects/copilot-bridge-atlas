use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Responses,
    ChatCompletions,
    SystemOne,
    ResponsesWebSocket,
    Compact,
    Search,
    Images,
    ImageEdits,
    Models,
    Messages,
    CountTokens,
    MessageModels,
}

pub const ENDPOINTS: [Endpoint; 12] = [
    Endpoint::Responses,
    Endpoint::ChatCompletions,
    Endpoint::SystemOne,
    Endpoint::ResponsesWebSocket,
    Endpoint::Compact,
    Endpoint::Search,
    Endpoint::Images,
    Endpoint::ImageEdits,
    Endpoint::Models,
    Endpoint::Messages,
    Endpoint::CountTokens,
    Endpoint::MessageModels,
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyEndpoint {
    pub id: &'static str,
    pub listener: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub description: &'static str,
    pub sample_input: String,
    pub sample_output: String,
    pub implemented: bool,
}

impl Endpoint {
    pub fn id(self) -> &'static str {
        match self {
            Self::Responses => "responses",
            Self::ChatCompletions => "chat",
            Self::SystemOne => "systemone",
            Self::ResponsesWebSocket => "websocket",
            Self::Compact => "compact",
            Self::Search => "search",
            Self::Images => "images",
            Self::ImageEdits => "edits",
            Self::Models => "models",
            Self::Messages => "messages-create",
            Self::CountTokens => "messages-count",
            Self::MessageModels => "messages-models",
        }
    }

    pub fn listener(self) -> &'static str {
        match self {
            Self::Messages | Self::CountTokens | Self::MessageModels => "messages",
            _ => "responses",
        }
    }

    pub fn method(self) -> &'static str {
        match self {
            Self::Models | Self::MessageModels => "GET",
            Self::ResponsesWebSocket => "WS",
            _ => "POST",
        }
    }

    pub fn paths(self) -> &'static [&'static str] {
        match self {
            Self::Responses | Self::ResponsesWebSocket => &[
                "/v1/responses",
                "/responses",
                "/v1/v1/responses",
                "/codex/v1/responses",
            ],
            Self::ChatCompletions => &["/v1/chat/completions", "/chat/completions"],
            Self::SystemOne => &["/v1/systemone", "/systemone"],
            Self::Compact => &[
                "/v1/responses/compact",
                "/responses/compact",
                "/v1/v1/responses/compact",
                "/codex/v1/responses/compact",
            ],
            Self::Search => &[
                "/v1/alpha/search",
                "/alpha/search",
                "/v1/v1/alpha/search",
                "/codex/v1/alpha/search",
            ],
            Self::Images => &[
                "/v1/images/generations",
                "/images/generations",
                "/v1/v1/images/generations",
                "/codex/v1/images/generations",
            ],
            Self::ImageEdits => &[
                "/v1/images/edits",
                "/images/edits",
                "/v1/v1/images/edits",
                "/codex/v1/images/edits",
            ],
            Self::Models => &["/v1/models", "/models"],
            Self::Messages => &["/v1/messages"],
            Self::CountTokens => &["/v1/messages/count_tokens"],
            Self::MessageModels => &["/v1/models"],
        }
    }

    fn example(self) -> (&'static str, Value, Value) {
        match self {
            Self::Responses => ("Generate a response from text or conversation input.",
                json!({"model":"<model-id>","input":"Say hello."}),
                json!({"object":"response","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello!"}]}]})),
            Self::ChatCompletions => ("Send chat messages and receive a completion.",
                json!({"model":"<chat-model-id>","messages":[{"role":"user","content":"Say hello."}]}),
                json!({"object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"Hello!"},"finish_reason":"stop"}]})),
            Self::SystemOne => ("Evaluate a condition and return a probability.",
                json!({"model":"microsoft-decision-1","state":"I was charged twice.","questions":{"billing":{"type":"noul","instructions":"Is this a billing issue?"}}}),
                json!({"model":"microsoft-decision-1","answers":{"billing":{"type":"noul","noul":0.99}},"usage":{"input_tokens":28,"output_tokens":1}})),
            Self::ResponsesWebSocket => ("Send response events over a persistent WebSocket connection.",
                json!({"type":"response.create","model":"<model-id>","input":"Say hello."}),
                json!({"type":"response.completed","response":{"id":"resp_example","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello!"}]}]}})),
            Self::Compact => ("Compact conversation history for the next request.",
                json!({"model":"<model-id>","input":[{"role":"user","content":"My project is Atlas."}]}),
                json!({"object":"response.compaction","output":[{"type":"compaction","encrypted_content":"<opaque-content>"}]})),
            Self::Search => ("Search for matching content. The payload is illustrative; use your client's supported format.",
                json!({"query":"API documentation"}),
                json!({"results":[{"title":"API documentation","url":"https://example.com/docs"}]})),
            Self::Images => ("Generate an image from a text prompt.",
                json!({"model":"<image-model-id>","prompt":"A blue mountain icon","size":"1024x1024"}),
                json!({"data":[{"b64_json":"<base64-image-data>"}]})),
            Self::ImageEdits => ("Edit an image using the JSON request format supported by the client.",
                json!({"model":"<image-model-id>","prompt":"Make the background white.","images":[{"image_url":"data:image/png;base64,<image-data>"}]}),
                json!({"data":[{"b64_json":"<edited-image-data>"}]})),
            Self::Models => ("List the Codex-compatible models exposed by Atlas.",
                json!("GET /v1/models\nNo request body."),
                json!({"models":[{"slug":"<model-id>","display_name":"Example model"}]})),
            Self::Messages => ("Send a conversation using the Messages API format.",
                json!({"model":"claude-sonnet-5","max_tokens":64,"messages":[{"role":"user","content":"Say hello."}]}),
                json!({"id":"msg_example","type":"message","role":"assistant","model":"claude-sonnet-5","content":[{"type":"text","text":"Hello!"}],"stop_reason":"end_turn","usage":{"input_tokens":8,"output_tokens":3}})),
            Self::CountTokens => ("Estimate input tokens without generating a response.",
                json!({"model":"claude-sonnet-5","messages":[{"role":"user","content":"Say hello."}]}),
                json!({"input_tokens":8})),
            Self::MessageModels => ("List the model roles configured for Messages API clients.",
                json!("GET /v1/models\nNo request body."),
                json!({"data":[{"id":"claude-sonnet-5","type":"model","display_name":"<configured-model-id>","created_at":"2026-01-01T00:00:00Z"}],"has_more":false,"first_id":"claude-sonnet-5","last_id":"claude-sonnet-5"})),
        }
    }

    pub fn info(self) -> ProxyEndpoint {
        let (description, input, output) = self.example();
        let display = |value: Value| match value {
            Value::String(text) => text,
            value => serde_json::to_string_pretty(&value)
                .expect("static endpoint sample is serializable"),
        };
        ProxyEndpoint {
            id: self.id(),
            listener: self.listener(),
            method: self.method(),
            path: self.paths()[0],
            description,
            sample_input: display(input),
            sample_output: display(output),
            implemented: true,
        }
    }
}

pub fn list_endpoints() -> Vec<ProxyEndpoint> {
    ENDPOINTS.into_iter().map(Endpoint::info).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_has_twelve_unique_protocol_entries_and_complete_samples() {
        let mut keys = std::collections::HashSet::new();
        let entries = list_endpoints();
        assert_eq!(
            entries.iter().filter(|e| e.listener == "responses").count(),
            9
        );
        assert_eq!(
            entries.iter().filter(|e| e.listener == "messages").count(),
            3
        );
        for entry in entries {
            assert!(keys.insert((entry.listener, entry.method, entry.path)));
            assert!(!entry.description.is_empty());
            assert!(!entry.sample_input.is_empty());
            assert!(serde_json::from_str::<Value>(&entry.sample_output).is_ok());
            assert_ne!(entry.path, "/v1/decisions");
        }
    }
}
