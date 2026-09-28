use crate::proxy::usage::parser::TokenUsage;
use serde_json::Value;
pub type StreamUsageParser = fn(&[Value]) -> Option<TokenUsage>;
pub type ResponseUsageParser = fn(&Value) -> Option<TokenUsage>;
pub type StreamModelExtractor = fn(&[Value], &str) -> String;
pub type StreamUsageEventFilter = fn(&str) -> bool;
#[derive(Clone, Copy)]
pub struct UsageParserConfig {
    pub stream_parser: StreamUsageParser,
    pub response_parser: ResponseUsageParser,
    pub model_extractor: StreamModelExtractor,
    pub stream_event_filter: Option<StreamUsageEventFilter>,
    pub app_type_str: &'static str,
}
pub fn codex_stream_usage_event_filter(data: &str) -> bool {
    data.contains("\"response.completed\"") || data.contains("\"usage\"")
}

fn codex_auto_model_extractor(events: &[Value], fallback_model: &str) -> String {
    // First try to get the model from the parsed usage
    if let Some(usage) = TokenUsage::from_codex_stream_events_auto(events) {
        if let Some(model) = usage.model.filter(|m| !m.is_empty()) {
            return model;
        }
    }
    // Fallback: Extracted from response.completed event
    events
        .iter()
        .find_map(|e| {
            if e.get("type")?.as_str()? == "response.completed" {
                e.get("response")?
                    .get("model")?
                    .as_str()
                    .filter(|m| !m.is_empty())
            } else {
                None
            }
        })
        .or_else(|| {
            // Falling Back Again: Extracting from OpenAI Format Events
            events
                .iter()
                .find_map(|e| e.get("model")?.as_str().filter(|m| !m.is_empty()))
        })
        .unwrap_or(fallback_model)
        .to_string()
}
pub const CODEX_PARSER_CONFIG: UsageParserConfig = UsageParserConfig {
    stream_parser: TokenUsage::from_codex_stream_events_auto,
    response_parser: TokenUsage::from_codex_response_auto,
    model_extractor: codex_auto_model_extractor,
    stream_event_filter: Some(codex_stream_usage_event_filter),
    app_type_str: "codex",
};
