use super::transform_responses;
use crate::proxy::ProxyError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

const OMITTED_IMAGE: &str =
    "[Atlas: earlier tool image omitted by the Messages API image-history limit.]";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImageHistoryConfig {
    pub enabled: bool,
    pub max_images: usize,
    pub model_limits: BTreeMap<String, usize>,
}

impl Default for ImageHistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_images: 50,
            model_limits: BTreeMap::new(),
        }
    }
}

impl ImageHistoryConfig {
    pub fn validate(&self) -> Result<(), crate::AppError> {
        if self.max_images == 0 || self.model_limits.values().any(|limit| *limit == 0) {
            return Err(crate::AppError::Config(
                "Messages API image limits must be positive; use enabled=false to disable the safeguard"
                    .into(),
            ));
        }
        if self
            .model_limits
            .keys()
            .any(|model| !crate::proxy::providers::copilot_model_map::is_valid_model_id(model))
        {
            return Err(crate::AppError::Config(
                "Invalid model identifier in Messages API image limits".into(),
            ));
        }
        Ok(())
    }

    fn limit_for(&self, model: &str) -> usize {
        self.model_limits
            .get(model)
            .copied()
            .unwrap_or(self.max_images)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImageHistoryStats {
    pub before: usize,
    pub protected: usize,
    pub omitted: usize,
    pub limit: usize,
}

impl ImageHistoryStats {
    pub fn retained(&self) -> usize {
        self.before - self.omitted
    }
}

pub fn prepare_request(
    body: Value,
    config: &ImageHistoryConfig,
) -> Result<(Value, ImageHistoryStats), ProxyError> {
    let latest_batch = latest_tool_batch(&body);
    let mut converted = transform_responses::anthropic_to_responses(body, None, false, false)?;
    let stats = apply_budget(&mut converted, config, &latest_batch)?;
    Ok((converted, stats))
}

fn latest_tool_batch(body: &Value) -> HashSet<String> {
    let mut ids = HashSet::new();
    let mut found = false;
    for message in body
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
    {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            if found {
                break;
            }
            continue;
        }
        for block in message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                found = true;
                if let Some(id) = block.get("tool_use_id").and_then(Value::as_str) {
                    ids.insert(id.to_owned());
                }
            }
        }
    }
    ids
}

fn is_image(part: &Value) -> bool {
    part.get("type").and_then(Value::as_str) == Some("input_image")
}

fn apply_budget(
    body: &mut Value,
    config: &ImageHistoryConfig,
    latest_batch: &HashSet<String>,
) -> Result<ImageHistoryStats, ProxyError> {
    let model = body.get("model").and_then(Value::as_str).unwrap_or("");
    let mut stats = ImageHistoryStats {
        limit: config.limit_for(model),
        ..Default::default()
    };
    let mut removable = Vec::new();
    for (item_index, item) in body
        .get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let tool_output = item.get("type").and_then(Value::as_str) == Some("function_call_output");
        let field = if tool_output { "output" } else { "content" };
        let protected = !tool_output
            || item
                .get("call_id")
                .and_then(Value::as_str)
                .is_none_or(|id| id.is_empty() || latest_batch.contains(id));
        for (part_index, part) in item
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if is_image(part) {
                stats.before += 1;
                if protected {
                    stats.protected += 1;
                } else {
                    removable.push((item_index, part_index));
                }
            }
        }
    }
    if !config.enabled || stats.before <= stats.limit {
        return Ok(stats);
    }
    if stats.protected > stats.limit {
        return Err(ProxyError::InvalidRequest(format!(
            "Messages API image limit exceeded: this request contains {} images, including {} protected user attachments and latest tool-result images, above the configured budget of {} for {}. Atlas will not discard those images. Send fewer attachments, reduce the latest tool-result batch, or start a new conversation with a text handoff and only essential images.",
            stats.before, stats.protected, stats.limit, model
        )));
    }
    let remove = stats.before - stats.limit;
    // Converted tool media is flat here; never walk arbitrary tool arguments or text as images.
    for (item, part) in removable.into_iter().take(remove) {
        body["input"][item]["output"][part] = json!({"type": "input_text", "text": OMITTED_IMAGE});
        stats.omitted += 1;
    }
    Ok(stats)
}

pub fn is_image_limit_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("maximum number of images")
        || (lower.contains("images")
            && lower.contains("exceed")
            && (lower.contains("maximum") || lower.contains("per request")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(n: usize) -> Value {
        json!({"type":"image", "source":{"type":"url", "url":format!("https://example.com/{n}.png")}})
    }

    fn history() -> Value {
        json!({"model":"gpt-6-astra", "messages":[
            {"role":"user", "content":[image(0)]},
            {"role":"assistant", "content":[{"type":"tool_use","id":"old","name":"screen","input":{}}]},
            {"role":"user", "content":[{"type":"tool_result","tool_use_id":"old","content":[image(1), image(2), {"type":"text","text":"Old screen"}]}]},
            {"role":"assistant", "content":[{"type":"tool_use","id":"new","name":"screen","input":{}}]},
            {"role":"user", "content":[{"type":"tool_result","tool_use_id":"new","content":[image(3)]}]}
        ]})
    }

    #[test]
    fn removes_only_oldest_tool_images_and_is_idempotent() {
        let config = ImageHistoryConfig {
            max_images: 3,
            ..Default::default()
        };
        let original = history();
        let expected =
            transform_responses::anthropic_to_responses(original.clone(), None, false, false)
                .unwrap();
        let (mut actual, stats) = prepare_request(original.clone(), &config).unwrap();
        assert_eq!(
            stats,
            ImageHistoryStats {
                before: 4,
                protected: 2,
                omitted: 1,
                limit: 3
            }
        );
        let mut expected = expected;
        expected["input"][2]["output"][0] = json!({"type":"input_text", "text":OMITTED_IMAGE});
        assert_eq!(actual, expected);
        let again = apply_budget(&mut actual, &config, &latest_tool_batch(&original)).unwrap();
        assert_eq!(again.omitted, 0);
        assert_eq!(actual, expected);
    }

    #[test]
    fn protects_split_parallel_tool_results_and_does_not_mutate_on_error() {
        let mut original = history();
        original["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"role":"user","content":[
                {"type":"tool_result","tool_use_id":"parallel","content":[image(4)]}
            ]}));
        let ids = latest_tool_batch(&original);
        assert_eq!(ids, HashSet::from(["new".into(), "parallel".into()]));
        let mut converted =
            transform_responses::anthropic_to_responses(original, None, false, false).unwrap();
        let before = converted.clone();
        assert!(apply_budget(
            &mut converted,
            &ImageHistoryConfig {
                max_images: 2,
                ..Default::default()
            },
            &ids
        )
        .is_err());
        assert_eq!(converted, before);
    }

    #[test]
    fn model_override_and_disabled_mode_are_explicit() {
        let original = history();
        let expected =
            transform_responses::anthropic_to_responses(original.clone(), None, false, false)
                .unwrap();
        let mut config = ImageHistoryConfig {
            max_images: 1,
            ..Default::default()
        };
        config.model_limits.insert("gpt-6-astra".into(), 4);
        assert_eq!(
            prepare_request(original.clone(), &config).unwrap().0,
            expected
        );
        config.model_limits.clear();
        config.enabled = false;
        assert_eq!(prepare_request(original, &config).unwrap().0, expected);
    }

    #[test]
    fn counts_repeated_occurrences_but_not_image_shaped_tool_arguments() {
        let mut original = history();
        original["messages"][1]["content"][0]["input"] =
            json!({"type":"input_image","image_url":"not-an-attachment"});
        original["messages"][2]["content"][0]["content"] = json!([image(1), image(1)]);
        let (_, stats) = prepare_request(
            original,
            &ImageHistoryConfig {
                max_images: 3,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.before, 4);
        assert_eq!(stats.omitted, 1);
    }

    #[test]
    fn counts_mcp_and_stringified_tool_images_after_conversion() {
        let mut original = history();
        original["messages"][2]["content"][0]["content"] = json!([
            {"type":"image", "mimeType":"image/webp", "data":"OLD_MCP_IMAGE"},
            image(2)
        ]);
        original["messages"][4]["content"][0]["content"] = json!({"content":[{
            "type":"image_url", "image_url":{"url":"data:image/png;base64,LATEST_IMAGE", "detail":"high"}
        }]}).to_string().into();
        let config = ImageHistoryConfig {
            max_images: 3,
            ..Default::default()
        };
        let (converted, stats) = prepare_request(original, &config).unwrap();
        assert_eq!(stats.before, 4);
        assert_eq!(stats.omitted, 1);
        let wire = converted.to_string();
        assert!(!wire.contains("OLD_MCP_IMAGE"));
        assert!(wire.contains("LATEST_IMAGE"));
        let latest = converted["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["call_id"] == "new" && item["type"] == "function_call_output")
            .unwrap();
        let image = latest["output"]
            .as_array()
            .unwrap()
            .iter()
            .find(|part| is_image(part))
            .unwrap();
        assert_eq!(image["detail"], "high");
    }

    #[test]
    fn recognizes_image_limit_errors_not_size_or_auth_errors() {
        assert!(is_image_limit_error(
            "Exceeded maximum number of images (50) allowed in the request."
        ));
        assert!(is_image_limit_error(
            "request contains 51 images, exceeding the maximum of 50 allowed per request"
        ));
        assert!(!is_image_limit_error("Image input is not supported"));
        assert!(!is_image_limit_error(
            "Request content length exceeded 32 MB limit"
        ));
        assert!(!is_image_limit_error("Quota exceeded"));
    }
}
