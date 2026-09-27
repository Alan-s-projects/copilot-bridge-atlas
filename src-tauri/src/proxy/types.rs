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

/// 代理服务器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// 监听地址
    pub listen_address: String,
    /// 监听端口
    pub listen_port: u16,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen_address: "127.0.0.1".to_string(),
            listen_port: 15722, // 使用较少占用的高位端口
        }
    }
}

/// 代理服务器状态
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxyStatus {
    #[serde(default)]
    pub active_requests: Vec<ActiveProxyRequest>,
    /// 是否运行中
    pub running: bool,
    /// 监听地址
    pub address: String,
    /// 监听端口
    pub port: u16,
    /// 活跃连接数
    pub active_connections: usize,
    /// 总请求数
    pub total_requests: u64,
    /// 成功请求数
    pub success_requests: u64,
    /// 失败请求数
    pub failed_requests: u64,
    /// 成功率 (0-100)
    pub success_rate: f32,
    /// 运行时间（秒）
    pub uptime_seconds: u64,
    /// 当前使用的Provider名称
    pub current_provider: Option<String>,
    /// 当前Provider的ID
    pub current_provider_id: Option<String>,
    /// 最后一次请求时间
    pub last_request_at: Option<String>,
    /// 最后一次错误信息
    pub last_error: Option<String>,
    /// 当前活跃的代理目标列表
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

/// 活跃的代理目标信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTarget {
    pub app_type: String, // "codex"
    pub provider_name: String,
    pub provider_id: String,
}

/// 代理服务器信息
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
    /// 代理总开关
    pub proxy_enabled: bool,
    /// 监听地址
    pub listen_address: String,
    /// 监听端口
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
