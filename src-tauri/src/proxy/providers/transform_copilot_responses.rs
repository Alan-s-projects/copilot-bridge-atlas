//! Adapt Codex-only tools to the function-only Responses dialect exposed by xAI.
use super::transform_codex_chat::{
    build_codex_tool_context_from_request, response_tool_call_item_from_chat_name,
    responses_custom_tool_call_to_chat_tool_call, responses_function_call_to_chat_tool_call,
    responses_tool_search_call_to_chat_tool_call, CodexToolContext,
};
use crate::proxy::{
    content_encoding::{decompress_body_with_limit, get_content_encoding},
    response_processor::strip_entity_headers_for_rebuilt_body,
    sse::{append_utf8_safe, strip_sse_field, take_sse_block},
    upstream_response::{ProxyResponse, MAX_RESPONSE_BODY_BYTES},
    ProxyError,
};
use bytes::Bytes;
use futures::StreamExt;
use serde_json::{json, Value};

pub(crate) fn adapt_request(body: &mut Value) -> Result<(), ProxyError> {
    let context = build_codex_tool_context_from_request(body);
    let tools: Vec<_> = context
        .chat_tools()
        .iter()
        .map(|tool| {
            let mut function = tool["function"].clone();
            function["type"] = json!("function");
            function
        })
        .collect();
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    } else if body.get("tools").is_some() {
        body["tools"] = json!([]);
    }
    if let Some(choice) = body
        .get_mut("tool_choice")
        .filter(|value| value.is_object())
    {
        let name = choice.get("name").and_then(Value::as_str).unwrap_or("");
        if !name.is_empty() {
            *choice = json!({"type":"function", "name":context.chat_name_for_response_function(
                name, choice.get("namespace").and_then(Value::as_str))});
        }
    }
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        let mut converted = Vec::new();
        for item in input.iter() {
            let kind = item.get("type").and_then(Value::as_str);
            let call = match kind {
                Some("function_call") => {
                    Some(responses_function_call_to_chat_tool_call(item, &context))
                }
                Some("custom_tool_call") => {
                    Some(responses_custom_tool_call_to_chat_tool_call(item))
                }
                Some("tool_search_call") => {
                    Some(responses_tool_search_call_to_chat_tool_call(item))
                }
                _ => None,
            };
            if let Some(call) = call {
                converted.push(json!({"type":"function_call", "call_id":call["id"],
                    "name":call["function"]["name"], "arguments":call["function"]["arguments"]}));
                continue;
            }
            match kind {
                Some("additional_tools" | "reasoning") => continue,
                Some("compaction" | "item_reference") => return Err(ProxyError::InvalidRequest(
                    "This model cannot replay an opaque item from another model. Start a new chat or switch back to the previous model.".into())),
                Some("custom_tool_call_output" | "tool_search_output") => {
                    converted.push(json!({"type":"function_call_output", "call_id":item["call_id"],
                        "output": serde_json::to_string(item).unwrap_or_default()}));
                }
                _ => {
                    let mut item = item.clone();
                    if let Some(object) = item.as_object_mut() {
                        for key in ["id", "phase", "status", "encrypted_content", "reasoning_content"] {
                            object.remove(key);
                        }
                    }
                    converted.push(item);
                }
            }
        }
        *input = converted;
    }
    Ok(())
}

fn restore_item(item: &mut Value, context: &CodexToolContext) {
    if item["type"] != "function_call" {
        return;
    }
    let Some(name) = item.get("name").and_then(Value::as_str) else {
        return;
    };
    if context.lookup_chat_name(name).is_none() {
        return;
    }
    *item = response_tool_call_item_from_chat_name(
        item.get("id").and_then(Value::as_str).unwrap_or(""),
        item.get("status")
            .and_then(Value::as_str)
            .unwrap_or("completed"),
        item.get("call_id").and_then(Value::as_str).unwrap_or(""),
        name,
        item.get("arguments").and_then(Value::as_str).unwrap_or(""),
        None,
        context,
    );
}

fn restore_output(response: &mut Value, context: &CodexToolContext) {
    if let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) {
        for item in output {
            restore_item(item, context);
        }
    }
}

fn restore_block(block: &str, context: &CodexToolContext) -> Option<String> {
    let data = block
        .lines()
        .filter_map(|line| strip_sse_field(line, "data"))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(mut event) = serde_json::from_str::<Value>(&data) else {
        return Some(block.into());
    };
    // Arguments can carry an envelope or raw custom-tool input. Publish the
    // complete restored item, never partial JSON from the transport encoding.
    if event["type"]
        .as_str()
        .is_some_and(|kind| kind.starts_with("response.function_call_arguments."))
    {
        return None;
    }
    if let Some(item) = event.get_mut("item") {
        restore_item(item, context);
    }
    if let Some(response) = event.get_mut("response") {
        restore_output(response, context);
    }
    let mut output = String::new();
    for line in block
        .lines()
        .filter(|line| strip_sse_field(line, "data").is_none())
    {
        output.push_str(line);
        output.push('\n');
    }
    output.push_str(&format!("data: {}", event));
    Some(output)
}

pub(crate) async fn adapt_response(
    response: ProxyResponse,
    context: CodexToolContext,
) -> Result<ProxyResponse, ProxyError> {
    let status = response.status();
    let mut headers = response.headers().clone();
    let sse = response.is_sse();
    let encoding = get_content_encoding(&headers);
    let response = if let Some(encoding) = encoding {
        let bytes = response.bytes_with_limit(MAX_RESPONSE_BODY_BYTES).await?;
        let decoded = decompress_body_with_limit(&encoding, &bytes, MAX_RESPONSE_BODY_BYTES)
            .map_err(|error| {
                ProxyError::ForwardFailed(format!("Cannot decode Copilot response: {error}"))
            })?
            .ok_or_else(|| ProxyError::Internal("Unsupported Copilot response encoding".into()))?;
        ProxyResponse::streamed(
            status,
            headers.clone(),
            futures::stream::once(async move { Ok(Bytes::from(decoded)) }),
        )
    } else {
        response
    };
    strip_entity_headers_for_rebuilt_body(&mut headers);
    if !sse {
        let bytes = response.bytes_with_limit(MAX_RESPONSE_BODY_BYTES).await?;
        let mut body: Value =
            serde_json::from_slice(&bytes).map_err(|e| ProxyError::Internal(e.to_string()))?;
        restore_output(&mut body, &context);
        return Ok(ProxyResponse::streamed(
            status,
            headers,
            futures::stream::once(async move { Ok(Bytes::from(body.to_string())) }),
        ));
    }
    let mut upstream = response.bytes_stream();
    let stream = async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8 = Vec::new();
        while let Some(chunk) = upstream.next().await {
            let chunk = match chunk { Ok(chunk) => chunk, Err(e) => { yield Err(e); return; } };
            append_utf8_safe(&mut buffer, &mut utf8, &chunk);
            if buffer.len() > MAX_RESPONSE_BODY_BYTES {
                yield Err(std::io::Error::other("Responses SSE event exceeds size limit"));
                return;
            }
            while let Some(block) = take_sse_block(&mut buffer) {
                if let Some(block) = restore_block(&block, &context) {
                    yield Ok(Bytes::from(format!("{block}\n\n")));
                }
            }
        }
        if !buffer.is_empty() {
            if let Some(block) = restore_block(&buffer, &context) { yield Ok(Bytes::from(format!("{block}\n\n"))); }
        }
    };
    Ok(ProxyResponse::streamed(status, headers, stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderMap, HeaderValue, StatusCode};

    fn request() -> Value {
        json!({
            "model":"grok-4.7",
            "tools":[
                {"type":"namespace","name":"functions","tools":[
                    {"type":"function","name":"lookup","parameters":{"anyOf":[
                        {"type":"object","properties":{"q":{"type":"string"}}},
                        {"type":"object","properties":{"id":{"type":"integer"}}}
                    ]}}
                ]},
                {"type":"custom","name":"patch","format":{"type":"text"}},
                {"type":"tool_search","execution":"client"}
            ],
            "input":[
                {"role":"user","content":"Lookup the greeting"},
                {"type":"reasoning","encrypted_content":"foreign","summary":[]},
                {"type":"message","role":"assistant","id":"foreign_id","phase":"commentary",
                 "content":[{"type":"output_text","text":"Looking it up."}]},
                {"type":"function_call","call_id":"call_1","namespace":"functions","name":"lookup","arguments":"{\"q\":\"hello\"}"},
                {"type":"function_call_output","call_id":"call_1","output":"found"},
                {"type":"custom_tool_call","call_id":"call_2","name":"patch","input":"raw\npatch"},
                {"type":"custom_tool_call_output","call_id":"call_2","output":"ok"},
                {"type":"tool_search_call","call_id":"call_3","arguments":{"query":"tools"}},
                {"type":"tool_search_output","call_id":"call_3","tools":[
                    {"type":"function","name":"loaded","parameters":{"type":"object","properties":{}}}
                ]}
            ],
            "tool_choice":{"type":"function","namespace":"functions","name":"lookup"},
            "reasoning":{"effort":"low"}, "stream":true
        })
    }

    #[test]
    fn adapts_namespaces_custom_tools_search_and_cross_model_history() {
        let mut request = request();
        adapt_request(&mut request).unwrap();
        assert_eq!(request["model"], "grok-4.7");
        assert_eq!(request["tools"].as_array().unwrap().len(), 4);
        assert!(request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["type"] == "function"));
        assert_eq!(request["tools"][0]["name"], "functions__lookup");
        assert_eq!(
            request["tools"][0]["parameters"]["properties"]["arguments"]["type"],
            "string"
        );
        assert_eq!(request["tool_choice"]["name"], "functions__lookup");
        assert_eq!(request["input"][1]["content"][0]["text"], "Looking it up.");
        assert!(request["input"][1].get("id").is_none());
        assert_eq!(
            request["input"][2]["arguments"],
            r#"{"arguments":"{\"q\":\"hello\"}"}"#
        );
        assert_eq!(
            request["input"][4]["arguments"],
            r#"{"input":"raw\npatch"}"#
        );
        assert_eq!(request["input"][5]["type"], "function_call_output");
        assert_eq!(request["input"][6]["name"], "tool_search");
        assert_eq!(request["input"][7]["type"], "function_call_output");
    }

    #[test]
    fn opaque_compaction_is_not_silently_discarded() {
        let mut body = json!({"input":[{"type":"compaction","encrypted_content":"opaque"}]});
        assert!(matches!(
            adapt_request(&mut body),
            Err(ProxyError::InvalidRequest(_))
        ));
    }

    #[tokio::test]
    async fn json_and_fragmented_sse_restore_tools_and_preserve_usage() {
        let context = build_codex_tool_context_from_request(&request());
        let output = json!([
            {"type":"function_call","id":"fc_1","call_id":"call_1","name":"functions__lookup","arguments":"{\"arguments\":{\"q\":\"hello\"}}","status":"completed"},
            {"type":"function_call","id":"fc_2","call_id":"call_2","name":"patch","arguments":"{\"input\":\"raw\\npatch\"}","status":"completed"},
            {"type":"function_call","id":"fc_3","call_id":"call_3","name":"tool_search","arguments":"{\"query\":\"tools\"}","status":"completed"}
        ]);
        let response = json!({"id":"resp_test","model":"grok-4.7","output":output,"usage":{"input_tokens":200001,"output_tokens":20}});
        let native = ProxyResponse::buffered(
            StatusCode::OK,
            HeaderMap::new(),
            Bytes::from(response.to_string()),
        );
        let decoded = adapt_response(native, context.clone())
            .await
            .unwrap()
            .bytes_with_limit(10000)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(body["output"][0]["name"], "lookup");
        assert_eq!(body["output"][0]["namespace"], "functions");
        assert_eq!(body["output"][0]["arguments"], r#"{"q":"hello"}"#);
        assert_eq!(body["output"][1]["type"], "custom_tool_call");
        assert_eq!(body["output"][1]["input"], "raw\npatch");
        assert_eq!(body["output"][2]["type"], "tool_search_call");
        assert_eq!(body["usage"], response["usage"]);
        let mut sse = String::from(": heartbeat\r\n\r\n");
        for (index, item) in output.as_array().unwrap().iter().enumerate() {
            sse.push_str(&format!(
                "data: {}\r\n\r\n",
                json!({"type":"response.output_item.done","output_index":index,"item":item})
            ));
        }
        sse.push_str(&format!(
            "data: {}\n\n",
            json!({"type":"response.completed","response":response})
        ));
        sse.push_str("data: [DONE]\n\n");
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            HeaderValue::from_static("text/event-stream"),
        );
        headers.insert("content-length", HeaderValue::from_static("999"));
        let native = ProxyResponse::streamed(
            StatusCode::OK,
            headers,
            futures::stream::iter(
                sse.into_bytes()
                    .into_iter()
                    .map(|byte| Ok(Bytes::from(vec![byte]))),
            ),
        );
        let restored = adapt_response(native, context).await.unwrap();
        assert!(!restored.headers().contains_key("content-length"));
        let text =
            String::from_utf8(restored.bytes_with_limit(10000).await.unwrap().to_vec()).unwrap();
        assert!(text.contains(": heartbeat"));
        assert!(text.contains("custom_tool_call"));
        assert!(text.contains("tool_search_call"));
        assert!(!text.contains("functions__lookup"));
        assert!(text.contains("[DONE]"));
    }
}
