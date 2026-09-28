use serde::{Deserialize, Serialize};

/// Requested effort and the explicit value sent upstream, not inferred model behavior.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReasoningEffort {
    pub requested: Option<String>,
    pub applied: Option<String>,
}

impl ReasoningEffort {
    pub fn from_request(body: &serde_json::Value) -> Self {
        Self {
            requested: Self::explicit_effort(body),
            applied: None,
        }
    }

    pub fn record_applied(&mut self, body: &serde_json::Value) {
        self.applied = Self::explicit_effort(body);
    }

    fn explicit_effort(body: &serde_json::Value) -> Option<String> {
        body.pointer("/reasoning/effort")
            .or_else(|| body.get("reasoning_effort"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }
}

/// Proxy server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// listening address
    pub listen_address: String,
    /// listening port
    pub listen_port: u16,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen_address: "127.0.0.1".to_string(),
            listen_port: 15722, // Use less occupied high-order ports
        }
    }
}

/// Proxy server status
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxyStatus {
    #[serde(default)]
    pub active_requests: Vec<ActiveProxyRequest>,
    /// Is it running?
    pub running: bool,
    /// listening address
    pub address: String,
    /// listening port
    pub port: u16,
    /// Number of active connections
    pub active_connections: usize,
    /// Total requests
    pub total_requests: u64,
    /// Number of successful requests
    pub success_requests: u64,
    /// Number of failed requests
    pub failed_requests: u64,
    /// Success rate (0-100)
    pub success_rate: f32,
    /// Run time (seconds)
    pub uptime_seconds: u64,
    /// Provider name currently in use
    pub current_provider: Option<String>,
    /// ID of the current provider
    pub current_provider_id: Option<String>,
    /// Last request time
    pub last_request_at: Option<String>,
    /// Last error message
    pub last_error: Option<String>,
    /// List of currently active proxy targets
    #[serde(default)]
    pub active_targets: Vec<ActiveTarget>,
}

/// Ephemeral display metadata only; active requests do not affect recorded usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveProxyRequest {
    pub request_id: String,
    pub model: String,
    pub request_model: String,
    pub requested_reasoning_effort: Option<String>,
    pub applied_reasoning_effort: Option<String>,
    pub created_at: i64,
}

/// Active agent target information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTarget {
    pub app_type: String, // "codex"
    pub provider_name: String,
    pub provider_id: String,
}

/// Proxy server information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyServerInfo {
    pub address: String,
    pub port: u16,
    pub started_at: String,
}

/// Global listener configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalProxyConfig {
    /// Agent main switch
    pub proxy_enabled: bool,
    /// listening address
    pub listen_address: String,
    /// listening port
    pub listen_port: u16,
}

fn default_true() -> bool {
    true
}

/// Copilot classification and session grouping configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopilotOptimizerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub request_classification: bool,
}
impl Default for CopilotOptimizerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            request_classification: true,
        }
    }
}
