//! Request body filtering module
//!
//! Filter private parameters that should not be transparently transmitted to the upstream to prevent leakage of internal information.
//!
//! ## Filter rules
//! - Fields starting with `_` are considered private parameters and will be filtered recursively
//! - Supports deep filtering of nested objects and arrays
//! - JSON Schema's properties / patternProperties / definitions / $defs names
//!   It is a user-defined field name and is not filtered by private parameters.
//!
//! ## Usage scenarios
//! - `_internal_id`: Internal tracking ID
//! - `_debug_mode`: Debug flag
//! - `_session_token`: session token
//! - `_client_version`: client version

use serde_json::Value;

/// Filter private parameters (fields starting with `_`)
///
/// Recursively traverse the JSON structure, removing all fields starting with an underscore.
///
/// # Arguments
/// * `body` - original request body
///
/// # Returns
/// Filtered request body
///
/// # Example
/// ```ignore
/// let input = json!({
///     "model": "gpt-6-astra",
///     "_internal_id": "abc123",
///     "messages": [{"role": "user", "content": "hello", "_token": "secret"}]
/// });
/// let output = filter_private_params(input);
/// // output does not contain _internal_id and _token
/// ```
pub fn filter_private_params(body: Value) -> Value {
    filter_recursive(body, &mut Vec::new(), &mut Vec::new())
}

/// Recursive filtering implementation
fn filter_recursive(value: Value, path: &mut Vec<String>, removed_keys: &mut Vec<String>) -> Value {
    match value {
        Value::Object(map) => {
            let is_schema_name_map = path.last().is_some_and(|key| matches_schema_name_map(key));
            let filtered: serde_json::Map<String, Value> = map
                .into_iter()
                .filter_map(|(key, val)| {
                    if key.starts_with('_') && !is_schema_name_map {
                        removed_keys.push(key);
                        None
                    } else {
                        path.push(key.clone());
                        let filtered_value = filter_recursive(val, path, removed_keys);
                        path.pop();
                        Some((key, filtered_value))
                    }
                })
                .collect();

            // Only log when filtered (avoid printing on every request)
            if !removed_keys.is_empty() {
                log::debug!("[BodyFilter] Filter private parameters: {removed_keys:?}");
                removed_keys.clear();
            }

            Value::Object(filtered)
        }
        Value::Array(arr) => Value::Array(
            arr.into_iter()
                .map(|v| filter_recursive(v, path, removed_keys))
                .collect(),
        ),
        other => other,
    }
}

fn matches_schema_name_map(key: &str) -> bool {
    matches!(
        key,
        "properties" | "patternProperties" | "definitions" | "$defs"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_filter_top_level_private_params() {
        let input = json!({
            "model": "gpt-6-astra",
            "_internal_id": "abc123",
            "_debug": true,
            "max_tokens": 1024
        });

        let output = filter_private_params(input);

        assert!(output.get("model").is_some());
        assert!(output.get("max_tokens").is_some());
        assert!(output.get("_internal_id").is_none());
        assert!(output.get("_debug").is_none());
    }

    #[test]
    fn test_filter_nested_private_params() {
        let input = json!({
            "model": "gpt-6-astra",
            "messages": [
                {
                    "role": "user",
                    "content": "hello",
                    "_session_token": "secret"
                }
            ],
            "metadata": {
                "user_id": "user-1",
                "_tracking_id": "track-1"
            }
        });

        let output = filter_private_params(input);

        // Top-level fields reserved
        assert!(output.get("model").is_some());
        assert!(output.get("messages").is_some());
        assert!(output.get("metadata").is_some());

        // Private parameters in messages array are filtered
        let messages = output.get("messages").unwrap().as_array().unwrap();
        assert!(messages[0].get("role").is_some());
        assert!(messages[0].get("content").is_some());
        assert!(messages[0].get("_session_token").is_none());

        // Private parameters in metadata objects are filtered
        let metadata = output.get("metadata").unwrap();
        assert!(metadata.get("user_id").is_some());
        assert!(metadata.get("_tracking_id").is_none());
    }

    #[test]
    fn test_filter_deeply_nested() {
        let input = json!({
            "level1": {
                "level2": {
                    "level3": {
                        "keep": "value",
                        "_remove": "secret"
                    }
                }
            }
        });

        let output = filter_private_params(input);

        let level3 = output
            .get("level1")
            .unwrap()
            .get("level2")
            .unwrap()
            .get("level3")
            .unwrap();

        assert!(level3.get("keep").is_some());
        assert!(level3.get("_remove").is_none());
    }

    #[test]
    fn test_filter_array_of_objects() {
        let input = json!({
            "items": [
                {"id": 1, "_secret": "a"},
                {"id": 2, "_secret": "b"},
                {"id": 3, "_secret": "c"}
            ]
        });

        let output = filter_private_params(input);
        let items = output.get("items").unwrap().as_array().unwrap();

        for item in items {
            assert!(item.get("id").is_some());
            assert!(item.get("_secret").is_none());
        }
    }

    #[test]
    fn test_no_private_params() {
        let input = json!({
            "model": "gpt-6-astra",
            "messages": [{"role": "user", "content": "hello"}]
        });

        let output = filter_private_params(input.clone());

        // Without private parameters, the output should be the same as the input
        assert_eq!(input, output);
    }

    #[test]
    fn test_empty_object() {
        let input = json!({});
        let output = filter_private_params(input);
        assert_eq!(output, json!({}));
    }

    #[test]
    fn test_primitive_values() {
        // The original value should not be modified
        assert_eq!(filter_private_params(json!(42)), json!(42));
        assert_eq!(filter_private_params(json!("string")), json!("string"));
        assert_eq!(filter_private_params(json!(true)), json!(true));
        assert_eq!(filter_private_params(json!(null)), json!(null));
    }

    #[test]
    fn test_preserves_json_schema_property_names_with_underscore() {
        let input = json!({
            "tools": [
                {
                    "name": "lookup",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "_id": {"type": "string", "_internal_note": "remove"},
                            "_meta": {"type": "object"}
                        },
                        "_private_schema_note": "remove"
                    }
                }
            ]
        });

        let output = filter_private_params(input);
        let schema = &output["tools"][0]["input_schema"];

        assert!(schema["properties"].get("_id").is_some());
        assert!(schema["properties"].get("_meta").is_some());
        assert!(schema["properties"]["_id"].get("_internal_note").is_none());
        assert!(schema.get("_private_schema_note").is_none());
    }
}
