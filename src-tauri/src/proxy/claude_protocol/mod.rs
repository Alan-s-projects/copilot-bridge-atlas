//! Anthropic protocol conversion recovered from a7c63c00 (before Atlas's Codex-only cleanup).
//! Kept isolated from Codex conversion modules. See docs/claude-gateway.md.
#![allow(dead_code)]
pub mod image_history;
mod reasoning_bridge;
pub mod streaming_responses;
mod tool_media;
mod transform;
pub mod transform_responses;
