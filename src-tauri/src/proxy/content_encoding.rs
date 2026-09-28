//! HTTP content-encoding tools.
//!
//! The automatic decompression of reqwest is disabled (in order to pass through accept-encoding), and manual decompression is required.
//! The request side (for example, Codex Desktop sends a compressed request body in the login state) and the response side (upstream compresses the response body)
//! Share the same set of decompression logic.

use axum::http::header::HeaderMap;
use std::io::Read;

/// Split the content-encoding value into an ordered coding list (removing identity and null values).
///
/// HTTP allows stacked coding (such as `gzip, zstd`), each coding is separated by commas; repetition is also allowed
/// content-encoding header, semantically equivalent to comma splicing (see [`get_content_encoding`]).
fn split_codings(content_encoding: &str) -> Vec<&str> {
    content_encoding
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty() && *c != "identity")
        .collect()
}

/// Whether a single coding can be decompressed.
fn is_single_supported(coding: &str) -> bool {
    matches!(
        coding,
        "gzip" | "x-gzip" | "deflate" | "br" | "zstd" | "zst"
    )
}

/// Reason for decompression failure. Distinguish "output over budget" from "data corruption": the former is a security rejection signal,
/// The caller on the response side should reject the response (502) accordingly, instead of silently falling back as a normal decompression failure.
#[derive(Debug)]
pub(crate) enum DecompressError {
    /// The underlying decoding failed (data corruption/format mismatch).
    Io(std::io::Error),
    /// The decompression output will be terminated if it exceeds `limit` bytes; at this time, the actual output size is unknown and will only be larger than the limit.
    TooLarge { limit: usize },
}

impl std::fmt::Display for DecompressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::TooLarge { limit } => write!(
                f,
                "The decompression output exceeds the upper limit of {limit} bytes"
            ),
        }
    }
}

impl std::error::Error for DecompressError {}

impl From<std::io::Error> for DecompressError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Read the decompressed output from the decoder, up to `max_bytes`; abort the read and return as soon as the output exceeds the budget
/// [`DecompressError::TooLarge`] - Compression bombs are stopped at budget exhaustion instead of in memory first
/// Expand it completely and compare the sizes.
fn read_with_output_limit<R: Read>(
    reader: R,
    max_bytes: usize,
) -> Result<Vec<u8>, DecompressError> {
    // saturating_add: budget preservation when calling unbounded (max_bytes = usize::MAX) usize::MAX
    let budget = max_bytes.saturating_add(1) as u64;
    let mut limited = reader.take(budget);
    let mut out = Vec::new();
    limited.read_to_end(&mut out)?;
    if out.len() > max_bytes {
        return Err(DecompressError::TooLarge { limit: max_bytes });
    }
    Ok(out)
}

/// Decompress a single content-coding and output the upper limit `max_output_bytes`. Unknown encoding returns `Ok(None)`.
fn decompress_single(
    coding: &str,
    body: &[u8],
    max_output_bytes: usize,
) -> Result<Option<Vec<u8>>, DecompressError> {
    match coding {
        "gzip" | "x-gzip" => {
            let decoder = flate2::read::GzDecoder::new(body);
            Ok(Some(read_with_output_limit(decoder, max_output_bytes)?))
        }
        "deflate" => {
            // RFC 9110: deflate refers to the zlib wrapper format; but some upstream/clients send raw deflate streams.
            // First try zlib according to the specification, and then fall back to raw if it fails - otherwise the decompression of the compliant source will inevitably fail.
            // Raw compressed bytes are fail-opened to JSON parsers (#2234 Form C one).
            let zlib = flate2::read::ZlibDecoder::new(body);
            match read_with_output_limit(zlib, max_output_bytes) {
                Ok(decompressed) => Ok(Some(decompressed)),
                // Exceeding the output budget is not an encoding failure.
                // A raw fallback can hide this rejection behind a decode error.
                Err(error @ DecompressError::TooLarge { .. }) => Err(error),
                Err(DecompressError::Io(zlib_err)) => {
                    log::debug!("deflate failed to decompress according to zlib ({zlib_err}), and fell back to raw deflate");
                    let raw = flate2::read::DeflateDecoder::new(body);
                    Ok(Some(read_with_output_limit(raw, max_output_bytes)?))
                }
            }
        }
        "br" => {
            let decoder = brotli::Decompressor::new(std::io::Cursor::new(body), 4096);
            Ok(Some(read_with_output_limit(decoder, max_output_bytes)?))
        }
        "zstd" | "zst" => {
            // The Codex login state enables zstd (Compression::Zstd) for the request body; the upstream may also zstd compress the response.
            let decoder = zstd::stream::read::Decoder::new(std::io::Cursor::new(body))?;
            Ok(Some(read_with_output_limit(decoder, max_output_bytes)?))
        }
        _ => Ok(None),
    }
}

/// Decompress body bytes according to content-encoding, supporting stacked encodings (such as `gzip, zstd`),
/// And the decompression output of each coding (including the intermediate products of stacked coding) is subject to `max_output_bytes`
/// Limit, if the limit is exceeded, it will abort and return [`DecompressError::TooLarge`], which is used to defend against response side compression bombs.
///
/// RFC 9110 §8.4: codings are listed in **application order**, so decompression must be **reverse** (last applied first).
/// Returning `Ok(None)` indicates that there is an unsupported encoding and is transparently transmitted as is - the caller must retain
/// content-encoding header, otherwise downstream (diagnostics/clients) will mistake compressed bytes for plaintext.
pub(crate) fn decompress_body_with_limit(
    content_encoding: &str,
    body: &[u8],
    max_output_bytes: usize,
) -> Result<Option<Vec<u8>>, DecompressError> {
    let codings = split_codings(content_encoding);
    if codings.is_empty() {
        return Ok(None);
    }
    // If any coding is not supported, decompression and header-preserving transparent transmission will be completely abandoned to avoid half-decoded dirty data.
    if !codings.iter().all(|c| is_single_supported(c)) {
        log::warn!("Unsupported content-encoding: {content_encoding}, skip decompression");
        return Ok(None);
    }

    // Reverse decoding: The end of the list is the last encoding applied and must be solved first.
    let mut data: Option<Vec<u8>> = None;
    for coding in codings.iter().rev() {
        let input = data.as_deref().unwrap_or(body);
        match decompress_single(coding, input, max_output_bytes)? {
            Some(decompressed) => data = Some(decompressed),
            // The above is_single_supported has been verified, the theory will not happen; defensive cover.
            None => return Ok(None),
        }
    }
    Ok(data)
}

/// Whether the content-encoding (including stacking, such as `gzip, zstd`) can all be decompressed.
///
/// It is used as a gate on the request side: compressed bodies that cannot be decompressed cannot be passed through to JSON parsing and must be rejected directly.
pub(crate) fn is_supported_content_encoding(content_encoding: &str) -> bool {
    let codings = split_codings(content_encoding);
    !codings.is_empty() && codings.iter().all(|c| is_single_supported(c))
}

/// Extract content-encoding from header (merge duplicate headers, ignore identity and null values).
///
/// HTTP allows repeated content-encoding headers, and the semantics are equivalent to comma splicing, so use `get_all` to merge;
/// The return value may contain multiple comma-separated codings, which are reversely decoded by [`decompress_body_with_limit`].
pub(crate) fn get_content_encoding(headers: &HeaderMap) -> Option<String> {
    let combined = headers
        .get_all("content-encoding")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
        .to_lowercase();
    if split_codings(&combined).is_empty() {
        return None;
    }
    Some(combined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn decompress_body_deflate_handles_zlib_wrapped_per_rfc9110() {
        // RFC 9110 standard deflate = zlib package format (this is what the compliance source sends)
        let payload = br#"{"ok":true}"#;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, payload).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress_body_with_limit("deflate", &compressed, 1024)
            .unwrap()
            .unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn decompress_body_deflate_falls_back_to_raw_stream() {
        // Some sources illegally send raw deflate streams to maintain compatibility.
        let payload = br#"{"ok":true}"#;
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, payload).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress_body_with_limit("deflate", &compressed, 1024)
            .unwrap()
            .unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn decompress_body_zstd_roundtrip() {
        // What is sent in the Codex login state is the zstd compressed request body.
        let payload = br#"{"hello":"world","n":42}"#;
        let compressed = zstd::stream::encode_all(std::io::Cursor::new(&payload[..]), 0).unwrap();
        let decompressed = decompress_body_with_limit("zstd", &compressed, 1024)
            .unwrap()
            .unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn decompress_body_stacked_gzip_then_zstd_decodes_in_reverse() {
        // Content-Encoding: gzip, zstd means gzip first and then zstd, decompression must be reversed (zstd first and then gzip)
        let payload = br#"{"stacked":true}"#;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, payload).unwrap();
        let gzipped = gz.finish().unwrap();
        let stacked = zstd::stream::encode_all(std::io::Cursor::new(&gzipped[..]), 0).unwrap();

        let decompressed = decompress_body_with_limit("gzip, zstd", &stacked, 1024)
            .unwrap()
            .unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn decompress_body_stacked_with_unsupported_returns_none() {
        // As long as one of the stacks does not support it, the entire head-protected transparent transmission will be implemented.
        let result = decompress_body_with_limit("snappy, zstd", b"\x00\x01\x02\x03", 1024).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn decompress_body_unknown_encoding_returns_none_to_keep_headers() {
        // Unknown encoding must return None (instead of pretending to be "decoded"), otherwise content-encoding
        // header is stripped off, downstream diagnostics will misreport compressed bytes as clear text
        let result = decompress_body_with_limit("snappy", b"\x00\x01\x02\x03", 1024).unwrap();
        assert!(result.is_none());
    }

    /// Generate deterministic pseudo-random bytes (LCG) to avoid testing introducing rand dependencies.
    fn pseudo_random_bytes(len: usize) -> Vec<u8> {
        let mut state: u64 = 0x243F_6A88_85A3_08D3;
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (state >> 33) as u8
            })
            .collect()
    }

    fn gzip_compress(payload: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, payload).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn decompress_body_with_limit_passes_payload_under_limit() {
        let payload = br#"{"ok":true}"#;
        let compressed = gzip_compress(payload);

        let out = decompress_body_with_limit("gzip", &compressed, 1024)
            .unwrap()
            .unwrap();
        assert_eq!(out, payload);
    }

    #[test]
    fn decompress_body_with_limit_allows_exactly_limit_bytes() {
        let payload = vec![7u8; 64 * 1024];
        let compressed = gzip_compress(&payload);

        let out = decompress_body_with_limit("gzip", &compressed, 64 * 1024)
            .unwrap()
            .unwrap();
        assert_eq!(out.len(), 64 * 1024);
        assert_eq!(out, payload);
    }

    #[test]
    fn decompress_body_with_limit_aborts_gzip_bomb_mid_stream() {
        // 4 MiB of pseudo-random data (~1:1 compression) truncated to 2 MiB after gzip: stream yields ~2 MiB
        // Ends abruptly after decompressing the data. Bounded reads should report TooLarge at 1 MiB budget exhaustion;
        // Unbounded reading will read all the way to the incomplete stream tail report UnexpectedEof (Io) - the two can be distinguished,
        // This test therefore identifies the degradation of "complete expansion first and then comparison".
        let payload = pseudo_random_bytes(4 * 1024 * 1024);
        let compressed = gzip_compress(&payload);
        assert!(compressed.len() > 2 * 1024 * 1024);
        let truncated = &compressed[..2 * 1024 * 1024];

        let result = decompress_body_with_limit("gzip", truncated, 1024 * 1024);
        assert!(
            matches!(result, Err(DecompressError::TooLarge { .. })),
            "It should be stopped when the budget is exhausted (TooLarge), rather than reporting an error after reading the end of the stream: {:?}",
            result.as_ref().map(|o| o.as_ref().map(Vec::len))
        );
    }

    #[test]
    fn decompress_body_with_limit_rejects_zstd_bomb() {
        // High compression ratio payload: 8 MiB, all zeros → zstd is only a few KiB after compression, and full expansion must exceed the limit
        let payload = vec![0u8; 8 * 1024 * 1024];
        let compressed = zstd::stream::encode_all(std::io::Cursor::new(&payload[..]), 0).unwrap();
        assert!(compressed.len() < 1024 * 1024);

        let result = decompress_body_with_limit("zstd", &compressed, 1024 * 1024);
        assert!(
            matches!(result, Err(DecompressError::TooLarge { .. })),
            "zstd compression bomb should be stopped at budget exhaustion: {:?}",
            result.as_ref().map(|o| o.as_ref().map(Vec::len))
        );
    }

    #[test]
    fn decompress_body_with_limit_rejects_brotli_bomb() {
        let payload = vec![0u8; 8 * 1024 * 1024];
        let mut compressed = Vec::new();
        {
            let mut writer = brotli::CompressorWriter::new(&mut compressed, 4096, 5, 22);
            std::io::Write::write_all(&mut writer, &payload).unwrap();
        }
        assert!(compressed.len() < 1024 * 1024);

        let result = decompress_body_with_limit("br", &compressed, 1024 * 1024);
        assert!(
            matches!(result, Err(DecompressError::TooLarge { .. })),
            "brotli compression bombs should be stopped at budget exhaustion: {:?}",
            result.as_ref().map(|o| o.as_ref().map(Vec::len))
        );
    }

    #[test]
    fn decompress_body_with_limit_bounds_intermediate_stage_of_stacked_encodings() {
        // Stacked encoding gzip, zstd: zstd first decomposes the gzip stream (small), and then gzip expands it into 8 MiB.
        // Intermediate products are also subject to budget constraints and cannot be defended only at the last level.
        let payload = vec![0u8; 8 * 1024 * 1024];
        let gzipped = gzip_compress(&payload);
        let stacked = zstd::stream::encode_all(std::io::Cursor::new(&gzipped[..]), 0).unwrap();

        let result = decompress_body_with_limit("gzip, zstd", &stacked, 1024 * 1024);
        assert!(
            matches!(result, Err(DecompressError::TooLarge { .. })),
            "Stacked encoded intermediate decompression products should also be subject to budget constraints: {:?}",
            result.as_ref().map(|o| o.as_ref().map(Vec::len))
        );
    }

    #[test]
    fn is_supported_content_encoding_matches_decompressable() {
        for enc in [
            "gzip",
            "x-gzip",
            "deflate",
            "br",
            "zstd",
            "zst",
            "gzip, zstd",
        ] {
            assert!(
                is_supported_content_encoding(enc),
                "{enc} should be supported"
            );
        }
        for enc in ["identity", "snappy", "compress", "", "gzip, snappy"] {
            assert!(
                !is_supported_content_encoding(enc),
                "{enc} should not be supported"
            );
        }
    }

    #[test]
    fn get_content_encoding_combines_repeated_headers() {
        // Duplicate content-encoding headers are equivalent to comma splicing and must be merged with get_all
        let mut headers = HeaderMap::new();
        headers.append("content-encoding", HeaderValue::from_static("gzip"));
        headers.append("content-encoding", HeaderValue::from_static("zstd"));
        assert_eq!(
            get_content_encoding(&headers).as_deref(),
            Some("gzip, zstd")
        );
    }

    #[test]
    fn get_content_encoding_ignores_identity_only() {
        let mut headers = HeaderMap::new();
        headers.append("content-encoding", HeaderValue::from_static("identity"));
        assert_eq!(get_content_encoding(&headers), None);
    }
}
