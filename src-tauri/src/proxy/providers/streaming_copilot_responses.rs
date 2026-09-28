//! Stabilize native Copilot Responses item IDs for streaming clients.
//!
//! Copilot can emit a different opaque ID for every event belonging to the same
//! output item. Keep the first upstream ID per output slot so clients reconcile
//! the streamed and completed item. Response cursors and tool call IDs are not
//! item identities and must remain untouched.

use crate::proxy::{
    content_encoding::get_content_encoding,
    response_processor::strip_entity_headers_for_rebuilt_body,
    sse::{append_utf8_safe, strip_sse_field, take_sse_block},
    upstream_response::ProxyResponse,
};
use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde_json::Value;
use std::collections::HashMap;

pub(crate) fn normalize_response(response: ProxyResponse) -> ProxyResponse {
    if !response.status().is_success() || !response.is_sse() {
        return response;
    }

    let status = response.status();
    let mut headers = response.headers().clone();
    // Identity encoding is requested upstream. If it is ignored, pass through
    // the original encoded body and entity headers.
    if get_content_encoding(&headers).is_some() {
        return ProxyResponse::streamed(status, headers, response.bytes_stream());
    }
    strip_entity_headers_for_rebuilt_body(&mut headers);
    ProxyResponse::streamed(status, headers, normalize_stream(response.bytes_stream()))
}

#[derive(Default)]
struct ItemIds(HashMap<u64, String>);

impl ItemIds {
    fn rewrite_id(&mut self, output_index: u64, value: Option<&mut Value>) -> bool {
        let Some(value) = value else {
            return false;
        };
        let Some(id) = value.as_str().filter(|id| !id.is_empty()) else {
            return false;
        };
        let canonical = self.0.entry(output_index).or_insert_with(|| id.to_string());
        if id == canonical {
            return false;
        }
        *value = Value::String(canonical.clone());
        true
    }

    fn rewrite_event(&mut self, event: &mut Value) -> bool {
        if !event
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.starts_with("response."))
        {
            return false;
        }

        let mut changed = false;
        if let Some(index) = event.get("output_index").and_then(Value::as_u64) {
            changed |= self.rewrite_id(index, event.pointer_mut("/item/id"));
            changed |= self.rewrite_id(index, event.get_mut("item_id"));
        }
        if let Some(output) = event
            .pointer_mut("/response/output")
            .and_then(Value::as_array_mut)
        {
            for (index, item) in output.iter_mut().enumerate() {
                changed |= self.rewrite_id(index as u64, item.get_mut("id"));
            }
        }
        changed
    }

    fn rewrite_block(&mut self, block: &str) -> String {
        let data = block
            .lines()
            .filter_map(|line| strip_sse_field(line, "data"))
            .collect::<Vec<_>>()
            .join("\n");
        let Ok(mut event) = serde_json::from_str::<Value>(&data) else {
            // Includes comments, heartbeats, [DONE], and unknown non-JSON data.
            return block.to_string();
        };
        if !self.rewrite_event(&mut event) {
            return block.to_string();
        }
        let Ok(data) = serde_json::to_string(&event) else {
            return block.to_string();
        };

        // Replace only the data field, retaining event/id/retry fields and
        // comments. Multiple data lines form one JSON event, not several events.
        let newline = if block.contains("\r\n") { "\r\n" } else { "\n" };
        let mut replaced = false;
        let mut lines = Vec::new();
        for line in block.lines() {
            if strip_sse_field(line, "data").is_some() {
                if !replaced {
                    lines.push(format!("data: {data}"));
                    replaced = true;
                }
            } else {
                lines.push(line.to_string());
            }
        }
        let mut output = lines.join(newline);
        if block.ends_with('\n') {
            output.push_str(newline);
        }
        output
    }
}

fn normalize_stream(
    stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut ids = ItemIds::default();
        let mut buffer = String::new();
        let mut utf8_remainder = Vec::new();
        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };
            append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);
            loop {
                let previous_len = buffer.len();
                let Some(block) = take_sse_block(&mut buffer) else {
                    break;
                };
                let delimiter = if previous_len - block.len() - buffer.len() == 4 {
                    "\r\n\r\n"
                } else {
                    "\n\n"
                };
                yield Ok(Bytes::from(format!("{}{delimiter}", ids.rewrite_block(&block))));
            }
        }

        if !utf8_remainder.is_empty() {
            buffer.push_str(&String::from_utf8_lossy(&utf8_remainder));
        }
        if !buffer.is_empty() {
            yield Ok(Bytes::from(ids.rewrite_block(&buffer)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use http::{HeaderMap, HeaderValue, StatusCode};
    use serde_json::json;

    const CAPTURE: &str = include_str!("../../../tests/fixtures/copilot_response_item_ids.sse");

    async fn collect(stream: impl Stream<Item = Result<Bytes, std::io::Error>>) -> String {
        let chunks = stream.try_collect::<Vec<_>>().await.unwrap();
        String::from_utf8(chunks.concat()).unwrap()
    }

    async fn normalize(input: &str, chunk_size: usize) -> String {
        let chunks = input
            .as_bytes()
            .chunks(chunk_size)
            .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
            .collect::<Vec<_>>();
        collect(normalize_stream(futures::stream::iter(chunks))).await
    }

    fn events(input: &str) -> Vec<Value> {
        let mut buffer = input.to_string();
        let mut blocks = Vec::new();
        while let Some(block) = take_sse_block(&mut buffer) {
            blocks.push(block);
        }
        blocks.push(buffer);
        blocks
            .iter()
            .filter_map(|block| {
                let data = block
                    .lines()
                    .filter_map(|line| strip_sse_field(line, "data"))
                    .collect::<Vec<_>>()
                    .join("\n");
                serde_json::from_str(&data).ok()
            })
            .collect()
    }

    #[tokio::test]
    async fn stabilizes_captured_copilot_items_without_changing_other_fields() {
        let original = events(CAPTURE);
        let normalized = events(&normalize(CAPTURE, 7).await);
        assert_eq!(normalized.len(), 17);
        for (before, after) in original.iter().zip(&normalized) {
            let mut expected = before.clone();
            if let Some(index) = before["output_index"].as_u64() {
                let id = ["msg_added", "fc_added"][index as usize];
                if let Some(value) = expected.pointer_mut("/item/id") {
                    *value = json!(id);
                }
                if let Some(value) = expected.get_mut("item_id") {
                    *value = json!(id);
                }
            }
            if before["type"] == "response.completed" {
                expected["response"]["output"][0]["id"] = json!("msg_added");
                expected["response"]["output"][1]["id"] = json!("fc_added");
            }
            assert_eq!(*after, expected);
        }
        let completed = normalized.last().unwrap();
        assert_eq!(completed["response"]["id"], "resp_completed");
        assert_eq!(completed["response"]["output"][1]["call_id"], "call_probe");
        assert_eq!(completed["response"]["output"][0]["phase"], "commentary");
    }

    #[tokio::test]
    async fn preserves_valid_stream_bytes_and_unrecognized_events() {
        let input = concat!(
            ": heartbeat\r\n\r\n",
            "event: response.output_item.added\r\n",
            "id: transport-1\r\nretry: 500\r\n",
            "data:{\"type\":\"response.output_item.added\",\"output_index\":0,",
            "\"item\":{\"id\":\"msg_one\",\"type\":\"message\"}}\r\n\r\n",
            "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,",
            "\"item_id\":\"msg_one\",\"delta\":\"€≈\"}\r\n\r\n",
            "data: {\"type\":\"vendor.event\",\"output_index\":0,\"item_id\":\"keep\"}\r\n\r\n",
            "data: not-json\r\n\r\ndata: [DONE]\r\n\r\n",
        );
        assert_eq!(normalize(input, 1).await, input);
    }

    #[tokio::test]
    async fn preserves_multiline_metadata_utf8_and_unterminated_tail() {
        let input = concat!(
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,",
            "\"item\":{\"id\":\"msg_one\",\"type\":\"message\"}}\r\n\r\n",
            ": keep this comment\r\nid: transport-2\r\nretry: 500\r\n",
            "event: response.output_text.done\r\n",
            "data: {\"type\":\"response.output_text.done\",\r\n",
            "data:\"output_index\":0,\"item_id\":\"msg_changed\",\"text\":\"€≈🌟\"}",
        );
        for size in [1, 2, 3, 7, input.len()] {
            let output = normalize(input, size).await;
            assert!(output.contains(": keep this comment\r\nid: transport-2\r\nretry: 500"));
            assert!(output.contains("event: response.output_text.done"));
            assert!(!output.ends_with('\n'));
            let events = events(&output);
            assert_eq!(events.len(), 2);
            assert_eq!(events[1]["item_id"], "msg_one");
            assert_eq!(events[1]["text"], "€≈🌟");
        }
    }

    #[tokio::test]
    async fn keeps_distinct_equal_messages_and_opaque_reasoning() {
        let items = [
            json!({"id":"reasoning", "type":"reasoning", "encrypted_content":"opaque"}),
            json!({"id":"comment", "type":"message", "phase":"commentary", "content":[{"type":"output_text","text":"Same"}]}),
            json!({"id":"final", "type":"message", "phase":"final_answer", "content":[{"type":"output_text","text":"Same"}]}),
        ];
        let mut input = String::new();
        for (index, item) in items.iter().enumerate() {
            input.push_str(&format!(
                "data: {}\n\n",
                json!({"type":"response.output_item.added","output_index":index,"item":item})
            ));
        }
        let mut completed_items = items.clone();
        for item in &mut completed_items {
            item["id"] = json!(format!("{}_changed", item["id"].as_str().unwrap()));
        }
        input.push_str(&format!(
            "data: {}\n\n",
            json!({"type":"response.completed","response":{"id":"cursor","output":completed_items}})
        ));
        let output = events(&normalize(&input, 1).await);
        assert_eq!(output.len(), 4);
        assert_eq!(output[3]["response"]["output"], json!(items));
    }

    #[tokio::test]
    async fn forwards_stream_errors() {
        let stream = futures::stream::iter([
            Ok(Bytes::from_static(b": heartbeat\n\n")),
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "upstream reset",
            )),
        ]);
        let output = normalize_stream(stream).collect::<Vec<_>>().await;
        assert_eq!(output[0].as_ref().unwrap(), ": heartbeat\n\n");
        assert_eq!(
            output[1].as_ref().unwrap_err().kind(),
            std::io::ErrorKind::ConnectionReset
        );
    }

    #[tokio::test]
    async fn rewrites_only_successful_unencoded_sse_and_removes_stale_lengths() {
        for (status, content_type, encoding, rewrite) in [
            (StatusCode::OK, "text/event-stream", None, true),
            (StatusCode::OK, "text/event-stream", Some("identity"), true),
            (StatusCode::BAD_GATEWAY, "text/event-stream", None, false),
            (StatusCode::OK, "application/json", None, false),
            (StatusCode::OK, "text/event-stream", Some("gzip"), false),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert("content-type", HeaderValue::from_static(content_type));
            headers.insert("content-length", HeaderValue::from(CAPTURE.len()));
            headers.insert("transfer-encoding", HeaderValue::from_static("chunked"));
            if let Some(encoding) = encoding {
                headers.insert("content-encoding", HeaderValue::from_static(encoding));
            }
            let response = normalize_response(ProxyResponse::buffered(
                status,
                headers.clone(),
                Bytes::from_static(CAPTURE.as_bytes()),
            ));
            assert_eq!(response.status(), status);
            if rewrite {
                assert!(!response.headers().contains_key("content-length"));
                assert!(!response.headers().contains_key("transfer-encoding"));
                assert!(!response.headers().contains_key("content-encoding"));
                let output = events(&collect(response.bytes_stream()).await);
                assert_eq!(output[3]["item_id"], "msg_added");
            } else {
                assert_eq!(response.headers(), &headers);
                assert_eq!(collect(response.bytes_stream()).await, CAPTURE);
            }
        }
    }

    #[tokio::test]
    async fn preserves_upstream_reasoning_header_and_usage() {
        let input = "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":125,\"input_tokens_details\":{\"cached_tokens\":60},\"output_tokens\":5,\"total_tokens\":130}}}\n\n";
        for upstream_header in [None, Some("true"), Some("false")] {
            let mut headers = HeaderMap::new();
            headers.insert(
                "content-type",
                HeaderValue::from_static("text/event-stream"),
            );
            if let Some(value) = upstream_header {
                headers.insert(
                    "x-reasoning-included",
                    HeaderValue::from_str(value).expect("valid upstream header"),
                );
            }
            let response = normalize_response(ProxyResponse::buffered(
                StatusCode::OK,
                headers.clone(),
                Bytes::copy_from_slice(input.as_bytes()),
            ));
            assert_eq!(
                response.headers().get("x-reasoning-included"),
                headers.get("x-reasoning-included")
            );
            assert_eq!(collect(response.bytes_stream()).await, input);
        }
    }
}
