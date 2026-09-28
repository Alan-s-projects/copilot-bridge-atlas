//! Proxy server module
//!
//! Local OpenAI-compatible HTTP server for GitHub Copilot.

pub mod body_filter;
pub(crate) mod content_encoding;
pub mod copilot_optimizer;
pub(crate) mod diagnostics;
pub mod error;
pub mod error_mapper;
mod forwarder;
pub mod handler_config;
pub mod handler_context;
mod handlers;
pub mod http_client;
pub(crate) mod json_canonical;
pub mod log_codes;
pub mod provider_router;
pub mod providers;
pub mod response_processor;
pub(crate) mod server;
pub mod session;
pub(crate) mod sse;
pub(crate) mod tool_media;
pub(crate) mod types;
pub mod upstream_response;
pub mod usage;

pub use error::ProxyError;
pub use session::extract_session_id;
