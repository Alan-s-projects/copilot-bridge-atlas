//! Bounded HTTP response bodies and streaming adapters.
use super::ProxyError;
use bytes::Bytes;
use futures::{stream::Stream, StreamExt};
pub(crate) const MAX_RESPONSE_BODY_BYTES: usize = 128 * 1024 * 1024;

/// Pooled HTTP responses and reconstructed Codex streams.
pub enum ProxyResponse {
    Reqwest(reqwest::Response),
    Streamed {
        status: http::StatusCode,
        headers: http::HeaderMap,
        stream: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
    },
}

impl ProxyResponse {
    #[cfg(test)]
    pub fn buffered(status: http::StatusCode, headers: http::HeaderMap, body: Bytes) -> Self {
        Self::streamed(
            status,
            headers,
            futures::stream::once(async move { Ok(body) }),
        )
    }

    pub fn streamed(
        status: http::StatusCode,
        headers: http::HeaderMap,
        stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
    ) -> Self {
        Self::Streamed {
            status,
            headers,
            stream: Box::pin(stream),
        }
    }

    pub fn status(&self) -> http::StatusCode {
        match self {
            Self::Reqwest(r) => r.status(),
            Self::Streamed { status, .. } => *status,
        }
    }

    pub fn headers(&self) -> &http::HeaderMap {
        match self {
            Self::Reqwest(r) => r.headers(),
            Self::Streamed { headers, .. } => headers,
        }
    }

    /// Shortcut: extract `content-type` header value as `&str`.
    pub fn content_type(&self) -> Option<&str> {
        self.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
    }

    /// Check if the response is an SSE stream.
    pub fn is_sse(&self) -> bool {
        self.content_type()
            .map(|ct| ct.contains("text/event-stream"))
            .unwrap_or(false)
    }

    /// Consume the response and collect the full body into `Bytes`, aborting the
    /// read as soon as the accumulated body exceeds `max_bytes`.
    ///
    /// All variants are checked block by block during the accumulation process, and will be disconnected if the limit is exceeded (drop stream terminates the upstream connection),
    /// Instead of collecting it first and then comparing - otherwise the oversized plaintext body will still be completely entered into the memory, and the restriction will be in vain.
    pub async fn bytes_with_limit(self, max_bytes: usize) -> Result<Bytes, ProxyError> {
        let mut stream = self.bytes_stream();
        let mut body = bytes::BytesMut::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                ProxyError::ForwardFailed(format!("Failed to read response body: {e}"))
            })?;
            if body.len() + chunk.len() > max_bytes {
                return Err(ProxyError::ResponseBodyTooLarge(body.len() + chunk.len()));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body.freeze())
    }

    /// Consume the response and return a byte-chunk stream (for SSE pass-through).
    pub fn bytes_stream(
        self,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
        use futures::StreamExt;

        match self {
            Self::Reqwest(r) => {
                let stream = r
                    .bytes_stream()
                    .map(|r| r.map_err(|e| std::io::Error::other(e.to_string())));
                Box::pin(stream)
            }
            Self::Streamed { stream, .. } => stream,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_large_stream_before_collecting_the_rest() {
        let response = ProxyResponse::streamed(
            http::StatusCode::OK,
            http::HeaderMap::new(),
            futures::stream::iter(vec![
                Ok(Bytes::from_static(b"1234")),
                Ok(Bytes::from_static(b"5678")),
            ]),
        );
        assert!(matches!(
            response.bytes_with_limit(6).await,
            Err(ProxyError::ResponseBodyTooLarge(8))
        ));
    }
}
