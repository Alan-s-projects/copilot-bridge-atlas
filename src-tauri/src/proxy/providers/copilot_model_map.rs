//! Model and automatic endpoint selection from the authenticated Copilot catalog.
use super::copilot_auth::CopilotModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopilotProtocol {
    Responses,
    Chat,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopilotTransport {
    pub protocol: CopilotProtocol,
    pub endpoint: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCopilotModel {
    pub id: String,
    pub vendor: String,
    pub transport: Option<CopilotTransport>,
    pub max_output_tokens: Option<u64>,
    pub supports_tool_calls: Option<bool>,
    pub supports_parallel_tool_calls: Option<bool>,
    pub reasoning_efforts: Option<Vec<String>>,
}

pub fn is_valid_model_id(model: &str) -> bool {
    let model = model.trim();
    !model.is_empty() && !model.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Only Grok's resolved outbound identity enables schema-guided tool repair.
pub(crate) fn is_grok_model(model: &str) -> bool {
    let model = model.rsplit('/').next().unwrap_or(model);
    model
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("grok-"))
}

pub fn is_selectable_model(model: &CopilotModel) -> bool {
    is_valid_model_id(&model.id)
        && model.model_picker_enabled
        && (model.model_type.is_empty() || model.model_type == "chat")
        && model.policy_state.as_deref() != Some("disabled")
        && transport_for(model).is_some()
}

pub fn resolve_model(client_id: &str, models: &[CopilotModel]) -> Option<ResolvedCopilotModel> {
    if !is_valid_model_id(client_id) {
        return None;
    }
    let model = models.iter().find(|model| {
        model.id.eq_ignore_ascii_case(client_id.trim()) && is_selectable_model(model)
    })?;
    Some(ResolvedCopilotModel {
        id: model.id.clone(),
        vendor: model.vendor.clone(),
        transport: transport_for(model),
        max_output_tokens: model.max_output_tokens,
        supports_tool_calls: model.supports_tool_calls,
        supports_parallel_tool_calls: model.supports_parallel_tool_calls,
        reasoning_efforts: model.reasoning_efforts.clone(),
    })
}

fn transport_for(model: &CopilotModel) -> Option<CopilotTransport> {
    let supported = |expected: &[&str]| {
        model.supported_endpoints.iter().find_map(|endpoint| {
            let path = endpoint
                .trim()
                .split_once('?')
                .map_or(endpoint.as_str(), |(path, _)| path)
                .trim_end_matches('/');
            expected
                .iter()
                .any(|candidate| path.eq_ignore_ascii_case(candidate))
                .then(|| path.to_string())
        })
    };

    let protocols = [CopilotProtocol::Responses, CopilotProtocol::Chat];
    protocols.iter().find_map(|&protocol| {
        let paths: &[&str] = match protocol {
            CopilotProtocol::Responses => &["/responses", "/v1/responses"],
            CopilotProtocol::Chat => &["/chat/completions", "/v1/chat/completions"],
        };
        supported(paths).map(|endpoint| CopilotTransport { protocol, endpoint })
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn model(id: &str, endpoints: &[&str]) -> CopilotModel {
        CopilotModel {
            id: id.into(),
            name: id.into(),
            vendor: "OpenAI".into(),
            model_picker_enabled: true,
            supported_endpoints: endpoints.iter().map(|value| value.to_string()).collect(),
            ..Default::default()
        }
    }
    #[test]
    fn selects_any_advertised_chat_model_and_automatically_chooses_its_protocol() {
        let models = vec![
            model("gpt-test", &["/responses", "/chat/completions"]),
            model("grok-4.7", &["/responses"]),
            model("gemini-3.8-flash", &["/chat/completions"]),
            model(
                "future-vendor/new-model",
                &["/chat/completions", "/responses"],
            ),
        ];
        let resolved = resolve_model("GPT-TEST", &models).unwrap();
        assert_eq!(resolved.id, "gpt-test");
        assert_eq!(
            resolved.transport.unwrap().protocol,
            CopilotProtocol::Responses
        );
        assert!(resolve_model("gpt-missing", &models).is_none());
        assert_eq!(
            resolve_model("gemini-3.8-flash", &models)
                .unwrap()
                .transport
                .unwrap()
                .protocol,
            CopilotProtocol::Chat
        );
        for id in ["grok-4.7", "future-vendor/new-model"] {
            assert_eq!(
                resolve_model(id, &models)
                    .unwrap()
                    .transport
                    .unwrap()
                    .protocol,
                CopilotProtocol::Responses
            );
        }
    }

    #[test]
    fn excludes_hidden_blocked_non_chat_and_unknown_protocol_models() {
        let mut hidden = model("hidden", &["/responses"]);
        hidden.model_picker_enabled = false;
        let mut blocked = model("blocked", &["/responses"]);
        blocked.policy_state = Some("disabled".into());
        let mut embedding = model("embedding", &["/responses"]);
        embedding.model_type = "embeddings".into();
        let unsupported = model("new-wire-format", &["/messages"]);
        let models = vec![hidden, blocked, embedding, unsupported];
        for model in &models {
            assert!(!is_selectable_model(model));
            assert!(resolve_model(&model.id, &models).is_none());
        }
        for id in ["", "   ", "bad\nid", "bad id"] {
            assert!(!is_valid_model_id(id));
        }
    }

    #[test]
    fn normalizes_advertised_paths_without_accepting_external_urls_or_websockets() {
        let model = model(
            "future",
            &[
                "ws:/responses",
                "https://example.com/responses",
                " /V1/RESPONSES/?version=1 ",
            ],
        );
        assert_eq!(transport_for(&model).unwrap().endpoint, "/V1/RESPONSES");
    }

    #[test]
    fn scopes_tool_integer_repair_to_grok_model_ids() {
        assert!(is_grok_model("grok-4.7"));
        assert!(is_grok_model("xai/GROK-4.7"));
        assert!(!is_grok_model("gpt-6-astra"));
        assert!(!is_grok_model("gemini-3.8-flash"));
        assert!(!is_grok_model("grokish-4.7"));
    }
}
