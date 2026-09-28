//! Mapping of error types to HTTP status codes
//!
//! Map ProxyError to the appropriate HTTP status code for logging and manually building error responses

use super::ProxyError;

/// Map ProxyError to HTTP status code
///
/// Mapping rules:
/// - Upstream error: directly use the status code returned by the upstream
/// - Timeout: 504 Gateway Timeout
/// - Connection failed: 502 Bad Gateway
/// - No Provider available: 503 Service Unavailable
/// - Authentication error: 401 Unauthorized
/// - Configuration/Request Error: 400 Bad Request
/// - Conversion error: 422 Unprocessable Entity
/// - Other errors: 500 Internal Server Error
pub fn map_proxy_error_to_status(error: &ProxyError) -> u16 {
    match error {
        // Service status error: consistent with IntoResponse
        ProxyError::AlreadyRunning => 409,
        ProxyError::NotRunning => 503,

        // Upstream error: use actual status code
        ProxyError::UpstreamError { status, .. } => *status,

        // Timeout error: 504 Gateway Timeout
        ProxyError::Timeout(_) => 504,

        // Forwarding failure/connection failure: 502 Bad Gateway
        ProxyError::ForwardFailed(_) => 502,
        ProxyError::ResponseBodyTooLarge(_) => 502,
        ProxyError::RequestBodyTooLarge(_) => 413,

        // No Provider available: 503 Service Unavailable

        // No provider configured: 503 Service Unavailable
        ProxyError::NoProvidersConfigured => 503,

        // Configuration error/invalid request: 400 Bad Request
        ProxyError::ConfigError(_) | ProxyError::InvalidRequest(_) => 400,

        // Authentication error: 401 Unauthorized
        ProxyError::AuthError(_) => 401,

        // Database error: 500 Internal Server Error
        ProxyError::DatabaseError(_) => 500,

        // Conversion error: 422 Unprocessable Entity
        ProxyError::TransformError(_) => 422,

        // Other unknown errors: 500 Internal Server Error
        _ => 500,
    }
}

/// Convert ProxyError into user-friendly error message
pub fn get_error_message(error: &ProxyError) -> String {
    match error {
        ProxyError::UpstreamError { status, body } => {
            if let Some(body) = body {
                format!("Upstream error ({status}): {body}")
            } else {
                format!("Upstream error ({status})")
            }
        }
        ProxyError::Timeout(msg) => format!("Request timeout: {msg}"),
        ProxyError::ForwardFailed(msg) => format!("Forwarding failed: {msg}"),
        ProxyError::NoProvidersConfigured => "No provider configured".to_string(),
        ProxyError::DatabaseError(msg) => format!("Database error: {msg}"),
        ProxyError::TransformError(msg) => format!("Request/response conversion error: {msg}"),
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_upstream_error() {
        let error = ProxyError::UpstreamError {
            status: 401,
            body: Some("Unauthorized".to_string()),
        };
        assert_eq!(map_proxy_error_to_status(&error), 401);
    }

    #[test]
    fn test_map_timeout_error() {
        let error = ProxyError::Timeout("Request timeout".to_string());
        assert_eq!(map_proxy_error_to_status(&error), 504);
    }

    #[test]
    fn test_map_connection_error() {
        let error = ProxyError::ForwardFailed("Connection refused".to_string());
        assert_eq!(map_proxy_error_to_status(&error), 502);
    }

    #[test]
    fn test_map_no_provider_error() {
        let error = ProxyError::NoProvidersConfigured;
        assert_eq!(map_proxy_error_to_status(&error), 503);
    }

    #[test]
    fn test_map_status_matches_proxy_error_response_semantics() {
        assert_eq!(
            map_proxy_error_to_status(&ProxyError::AuthError("bad token".to_string())),
            401
        );
        assert_eq!(
            map_proxy_error_to_status(&ProxyError::ConfigError("bad config".to_string())),
            400
        );
        assert_eq!(
            map_proxy_error_to_status(&ProxyError::InvalidRequest("bad request".to_string())),
            400
        );
        assert_eq!(
            map_proxy_error_to_status(&ProxyError::TransformError("bad transform".to_string())),
            422
        );
        for error in [
            ProxyError::RequestBodyTooLarge(1024),
            ProxyError::ResponseBodyTooLarge(1024),
        ] {
            use axum::response::IntoResponse;
            let mapped = map_proxy_error_to_status(&error);
            assert_eq!(mapped, error.into_response().status().as_u16());
        }
    }

    #[test]
    fn test_get_error_message() {
        let error = ProxyError::UpstreamError {
            status: 500,
            body: Some("Internal Server Error".to_string()),
        };
        let msg = get_error_message(&error);
        assert!(msg.contains("Upstream error"));
        assert!(msg.contains("500"));
        assert!(msg.contains("Internal Server Error"));
    }
}
