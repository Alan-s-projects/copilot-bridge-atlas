//! GitHub Copilot transport and Codex Responses/Chat adaptation.
mod codex;
pub(crate) mod codex_chat_common;
pub mod codex_chat_history;
pub(crate) mod codex_responses_sse;
pub mod copilot_auth;
pub mod copilot_model_map;
pub mod streaming_codex_chat;
pub(crate) mod streaming_copilot_responses;
pub(crate) mod tool_integer_repair;
pub mod transform_codex_chat;
pub(crate) mod transform_copilot_responses;

pub use codex::{inject_codex_chat_prompt_cache_key, is_codex_responses_endpoint};
