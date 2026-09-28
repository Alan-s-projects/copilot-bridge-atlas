//! GitHub Copilot Authentication Module
//!
//! Implement GitHub OAuth device code process and Copilot token management.
//! Supports multi-account authentication, and each Provider can be associated with different GitHub accounts.
//!
//! ## Certification process
//! 1. Start the device code process and obtain device_code and user_code
//! 2. The user completes GitHub authorization in the browser
//! 3. Poll to obtain access_token
//! 4. Use GitHub token to obtain Copilot token
//! 5. Automatically refresh Copilot token (60 seconds before expiration)
//!
//! ## Multiple account support
//! - Each GitHub account stores tokens independently
//! - Provider associates accounts through meta.authBinding

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

use crate::proxy::error::ProxyError;

/// GitHub OAuth Client ID (VS Code) - for github.com
const GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

/// GitHub OAuth Client ID (same as OpenCode) - pre-registered on all GHES Copilot instances
const GITHUB_CLIENT_ID_GHES: &str = "Ov23li8tweQw6odWQebz";

/// Default GitHub domain name
const DEFAULT_GITHUB_DOMAIN: &str = "github.com";

/// Select OAuth client ID based on domain name
fn github_client_id(domain: &str) -> &'static str {
    if domain == DEFAULT_GITHUB_DOMAIN {
        GITHUB_CLIENT_ID
    } else {
        GITHUB_CLIENT_ID_GHES
    }
}

fn default_github_domain() -> String {
    DEFAULT_GITHUB_DOMAIN.to_string()
}

/// GitHub device code URL
fn github_device_code_url(domain: &str) -> String {
    format!("https://{domain}/login/device/code")
}

/// GitHub OAuth Token URL
fn github_oauth_token_url(domain: &str) -> String {
    format!("https://{domain}/login/oauth/access_token")
}

/// GitHub API base URL (github.com uses api.github.com, GHES uses {domain}/api/v3)
fn github_api_base(domain: &str) -> String {
    if domain == DEFAULT_GITHUB_DOMAIN {
        "https://api.github.com".to_string()
    } else {
        format!("https://{domain}/api/v3")
    }
}

/// Copilot Token URL
fn copilot_token_url(domain: &str) -> String {
    format!("{}/copilot_internal/v2/token", github_api_base(domain))
}

/// GitHub User API URL
fn github_user_url(domain: &str) -> String {
    format!("{}/user", github_api_base(domain))
}

/// Copilot Usage API URL
fn copilot_usage_url(domain: &str) -> String {
    format!("{}/copilot_internal/user", github_api_base(domain))
}

/// Copilot API base address (github.com uses api.githubcopilot.com, GHES uses copilot-api.{domain})
fn copilot_api_base(domain: &str) -> String {
    if domain == DEFAULT_GITHUB_DOMAIN {
        "https://api.githubcopilot.com".to_string()
    } else {
        format!("https://copilot-api.{domain}")
    }
}

/// Token refresh advance time (seconds)
const TOKEN_REFRESH_BUFFER_SECONDS: i64 = 60;
const MODEL_CATALOG_TTL: Duration = Duration::from_secs(300);

/// Determine whether it is GitHub Enterprise Server (not github.com)
fn is_ghes(domain: &str) -> bool {
    domain != DEFAULT_GITHUB_DOMAIN
}

/// Normalized GitHub domain name (SSOT):
/// - Lowercase
/// - Stripping Agreement (https://http://）
/// - Strip trailing slashes, path, query, fragment
/// - Reject input containing userinfo (@)
/// - Reserve port number (if any)
fn normalize_github_domain(raw: &str) -> Result<String, CopilotAuthError> {
    let s = raw.trim();
    // divestiture agreement
    let s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    // Take the host part (to the first / or ? or #)
    let host = s.split(&['/', '?', '#'][..]).next().unwrap_or(s);
    // Deny userinfo
    if host.contains('@') {
        return Err(CopilotAuthError::InvalidDomain(raw.to_string()));
    }
    let normalized = host.to_lowercase();
    if normalized.is_empty() {
        return Err(CopilotAuthError::InvalidDomain(raw.to_string()));
    }
    Ok(normalized)
}

/// Generate a composite account ID to ensure that user IDs of different GHES instances do not conflict.
/// github.com uses its numeric ID; GHES uses the `domain:user_id` format.
fn composite_account_id(domain: &str, user_id: u64) -> String {
    if domain == DEFAULT_GITHUB_DOMAIN {
        user_id.to_string()
    } else {
        format!("{}:{}", domain, user_id)
    }
}

/// Copilot API Header constants
pub const COPILOT_EDITOR_VERSION: &str = "vscode/1.110.1";
pub const COPILOT_PLUGIN_VERSION: &str = "copilot-chat/0.38.2";
pub const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.38.2";
pub const COPILOT_API_VERSION: &str = "2025-10-01";
pub const COPILOT_INTEGRATION_ID: &str = "vscode-chat";

/// Build the Copilot request identity used by Codex forwarding.
///
/// The forwarder adds the session-derived interaction ID separately because
/// authentication has no per-request session context.
pub fn build_copilot_request_headers(
    token: &str,
) -> Result<Vec<(http::HeaderName, http::HeaderValue)>, ProxyError> {
    let mut bearer = String::from("Bearer ");
    bearer.push_str(token);
    let request_id = uuid::Uuid::new_v4().to_string();
    Ok(vec![
        (
            http::HeaderName::from_static("authorization"),
            auth_header_value(&bearer)?,
        ),
        (
            http::HeaderName::from_static("editor-version"),
            http::HeaderValue::from_static(COPILOT_EDITOR_VERSION),
        ),
        (
            http::HeaderName::from_static("editor-plugin-version"),
            http::HeaderValue::from_static(COPILOT_PLUGIN_VERSION),
        ),
        (
            http::HeaderName::from_static("copilot-integration-id"),
            http::HeaderValue::from_static(COPILOT_INTEGRATION_ID),
        ),
        (
            http::HeaderName::from_static("user-agent"),
            http::HeaderValue::from_static(COPILOT_USER_AGENT),
        ),
        (
            http::HeaderName::from_static("x-github-api-version"),
            http::HeaderValue::from_static(COPILOT_API_VERSION),
        ),
        (
            http::HeaderName::from_static("openai-intent"),
            http::HeaderValue::from_static("conversation-agent"),
        ),
        (
            http::HeaderName::from_static("x-initiator"),
            http::HeaderValue::from_static("user"),
        ),
        (
            http::HeaderName::from_static("x-interaction-type"),
            http::HeaderValue::from_static("conversation-agent"),
        ),
        (
            http::HeaderName::from_static("x-vscode-user-agent-library-version"),
            http::HeaderValue::from_static("electron-fetch"),
        ),
        (
            http::HeaderName::from_static("x-request-id"),
            auth_header_value(&request_id)?,
        ),
        (
            http::HeaderName::from_static("x-agent-task-id"),
            auth_header_value(&request_id)?,
        ),
    ])
}

/// Copilot usage response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotUsageResponse {
    /// Copilot Plan Type
    pub copilot_plan: String,
    /// Quota reset date
    pub quota_reset_date: String,
    /// Quota Snapshot
    pub quota_snapshots: QuotaSnapshots,
    /// API endpoint information (used to dynamically obtain the API URL)
    #[serde(default)]
    pub endpoints: Option<CopilotEndpoints>,
}

/// Copilot API endpoint information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotEndpoints {
    /// API endpoint URL
    pub api: String,
    /// Telemetry endpoint URL
    #[serde(default)]
    pub telemetry: Option<String>,
}

/// Quota Snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaSnapshots {
    /// Chat quota
    pub chat: QuotaDetail,
    /// Completions quota
    pub completions: QuotaDetail,
    /// Premium interaction quota
    pub premium_interactions: QuotaDetail,
}

/// Quota details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaDetail {
    /// total quota
    pub entitlement: i64,
    /// remaining quota
    pub remaining: i64,
    /// remaining percentage
    pub percent_remaining: f64,
    /// Is it infinite?
    pub unlimited: bool,
}

/// Copilot available models
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CopilotModel {
    /// Model ID (used for API calls)
    pub id: String,
    /// Model display name
    pub name: String,
    /// model supplier
    pub vendor: String,
    /// Whether to show in the model selector
    pub model_picker_enabled: bool,
    #[serde(default)]
    pub model_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_state: Option<String>,
    /// Copilot-reported context window for this exact model ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// Total context window tokens advertised by Copilot limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_context_window_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_tool_calls: Option<bool>,
    /// Upstream protocols supported by this exact model ID.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_endpoints: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_efforts: Option<Vec<String>>,
}

/// Copilot Models API response
#[derive(Debug, Deserialize)]
struct CopilotModelsResponse {
    data: Vec<CopilotModelsResponseItem>,
}

/// Copilot Models API response items
#[derive(Debug, Deserialize)]
struct CopilotModelsResponseItem {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    vendor: String,
    #[serde(default)]
    model_picker_enabled: bool,
    #[serde(default)]
    supported_endpoints: Vec<String>,
    #[serde(default)]
    capabilities: Option<Value>,
    #[serde(default)]
    policy: Option<Value>,
}

fn extract_copilot_context_window(capabilities: Option<&Value>) -> Option<u64> {
    let capabilities = capabilities?;
    // Some Copilot models accept less input than their total context window.
    // Codex must compact before reaching either server-side limit.
    ["max_context_window_tokens", "max_prompt_tokens"]
        .into_iter()
        .filter_map(|key| capabilities.get("limits")?.get(key)?.as_u64())
        .filter(|tokens| *tokens > 0)
        .min()
}

fn extract_copilot_total_context_window(capabilities: Option<&Value>) -> Option<u64> {
    let capabilities = capabilities?;
    capabilities
        .get("limits")?
        .get("max_context_window_tokens")?
        .as_u64()
        .filter(|tokens| *tokens > 0)
}

impl From<CopilotModelsResponseItem> for CopilotModel {
    fn from(model: CopilotModelsResponseItem) -> Self {
        let supports = model.capabilities.as_ref().and_then(|c| c.get("supports"));
        Self {
            context_window: extract_copilot_context_window(model.capabilities.as_ref()),
            max_context_window_tokens: extract_copilot_total_context_window(
                model.capabilities.as_ref(),
            ),
            model_type: model
                .capabilities
                .as_ref()
                .and_then(|c| c.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("chat")
                .to_string(),
            policy_state: model
                .policy
                .as_ref()
                .and_then(|p| p.get("state"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            max_output_tokens: model
                .capabilities
                .as_ref()
                .and_then(|c| c.get("limits"))
                .and_then(|l| l.get("max_output_tokens"))
                .and_then(Value::as_u64)
                .filter(|limit| *limit > 0),
            supports_tool_calls: supports
                .and_then(|s| s.get("tool_calls"))
                .and_then(Value::as_bool),
            supports_parallel_tool_calls: supports
                .and_then(|s| s.get("parallel_tool_calls"))
                .and_then(Value::as_bool),
            supports_vision: supports
                .and_then(|s| s.get("vision"))
                .and_then(Value::as_bool),
            reasoning_efforts: supports
                .and_then(|s| s.get("reasoning_effort"))
                .and_then(Value::as_array)
                .map(|levels| {
                    levels
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                }),
            name: if model.name.trim().is_empty() {
                model.id.clone()
            } else {
                model.name
            },
            id: model.id,
            vendor: model.vendor,
            model_picker_enabled: model.model_picker_enabled,
            supported_endpoints: model.supported_endpoints,
        }
    }
}

struct CachedCopilotModels {
    models: Vec<CopilotModel>,
    fetched_at: Instant,
}

impl CachedCopilotModels {
    fn fresh_models(&self) -> Option<Vec<CopilotModel>> {
        (self.fetched_at.elapsed() < MODEL_CATALOG_TTL).then(|| self.models.clone())
    }
}

/// Copilot authentication error
#[derive(Debug, thiserror::Error)]
pub enum CopilotAuthError {
    #[error("Waiting for user authorization")]
    AuthorizationPending,

    #[error("User refuses authorization")]
    AccessDenied,

    #[error("Device code has expired")]
    ExpiredToken,

    #[error("GitHub token is invalid or expired")]
    GitHubTokenInvalid,

    #[error("Copilot token acquisition failed: {0}")]
    CopilotTokenFetchFailed(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("IO error: {0}")]
    IoError(String),

    #[error("User is not subscribed to Copilot")]
    NoCopilotSubscription,

    #[error("Account does not exist: {0}")]
    AccountNotFound(String),

    #[error("Invalid GitHub domain name: {0}")]
    InvalidDomain(String),
}

impl From<reqwest::Error> for CopilotAuthError {
    fn from(err: reqwest::Error) -> Self {
        CopilotAuthError::NetworkError(err.to_string())
    }
}

impl From<std::io::Error> for CopilotAuthError {
    fn from(err: std::io::Error) -> Self {
        CopilotAuthError::IoError(err.to_string())
    }
}

/// GitHub device code response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubDeviceCodeResponse {
    /// Device code (for polling)
    pub device_code: String,
    /// User code (displayed to user)
    pub user_code: String,
    /// Verify URL
    pub verification_uri: String,
    /// Expiration time (seconds)
    pub expires_in: u64,
    /// Polling interval (seconds)
    pub interval: u64,
}

/// GitHub OAuth Token response
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubOAuthResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Copilot Token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotToken {
    /// JWT Token
    pub token: String,
    /// Expiration timestamp (Unix seconds)
    pub expires_at: i64,
}

impl CopilotToken {
    /// Check if the token is about to expire (60 seconds in advance)
    pub fn is_expiring_soon(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.expires_at - now < TOKEN_REFRESH_BUFFER_SECONDS
    }
}

/// Copilot Token API response
#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    token: String,
    expires_at: i64,
}

/// GitHub user information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubUser {
    pub login: String,
    pub id: u64,
    pub avatar_url: Option<String>,
}

/// GitHub account (public information, returned to the front end)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubAccount {
    /// GitHub user ID (as a string that serves as a unique identifier)
    pub id: String,
    /// GitHub username
    pub login: String,
    /// Avatar URL
    pub avatar_url: Option<String>,
    /// Authentication timestamp
    pub authenticated_at: i64,
    /// GitHub domain name (github.com or GHES domain name)
    #[serde(default = "default_github_domain")]
    pub github_domain: String,
    /// Whether the hosting account needs to log in again to complete the missing credentials.
    /// Codex: true for old accounts that lack persistent id_token; Copilot is always false.
    #[serde(default)]
    pub reauth_required: bool,
}

impl From<&GitHubAccountData> for GitHubAccount {
    fn from(data: &GitHubAccountData) -> Self {
        GitHubAccount {
            id: composite_account_id(&data.github_domain, data.user.id),
            login: data.user.login.clone(),
            avatar_url: data.user.avatar_url.clone(),
            authenticated_at: data.authenticated_at,
            github_domain: data.github_domain.clone(),
            reauth_required: false,
        }
    }
}

/// Copilot authentication status (supports multiple accounts)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotAuthStatus {
    /// All verified accounts
    pub accounts: Vec<GitHubAccount>,
    /// Default account ID (explicit state, avoids relying on HashMap order)
    pub default_account_id: Option<String>,
    /// Whether it has been authenticated
    pub authenticated: bool,
}

/// Account data (internal storage structure)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubAccountData {
    /// GitHub OAuth Token
    ///
    /// Security Note: In order to reuse login status, the token will be persisted locally.
    /// The current implementation is not connected to the system keychain and relies on private file permissions (0600 under Unix) for protection.
    pub github_token: String,
    /// User information
    pub user: GitHubUser,
    /// Authentication timestamp
    pub authenticated_at: i64,
    /// GitHub domain name (github.com or GHES domain name)
    #[serde(default = "default_github_domain")]
    pub github_domain: String,
}

/// Persistent storage structure for Atlas 6.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CopilotAuthStore {
    /// Storage format version
    version: u32,
    /// Multiple account data (key = GitHub user ID)
    #[serde(default)]
    accounts: HashMap<String, GitHubAccountData>,
    /// Default account ID
    #[serde(skip_serializing_if = "Option::is_none")]
    default_account_id: Option<String>,
}

/// Copilot authentication manager (supports multiple accounts)
pub struct CopilotAuthManager {
    /// All GitHub accounts (key = GitHub user ID)
    accounts: Arc<RwLock<HashMap<String, GitHubAccountData>>>,
    /// Default account ID
    default_account_id: Arc<RwLock<Option<String>>>,
    /// Refresh lock for each account to prevent concurrent refreshes from hitting the GitHub API repeatedly
    refresh_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
    /// Copilot Token cache (key = GitHub user ID, memory cache, automatic refresh)
    copilot_tokens: Arc<RwLock<HashMap<String, CopilotToken>>>,
    /// Copilot Models cache (key = GitHub user ID, in-process reuse only)
    copilot_models: Arc<RwLock<HashMap<String, CachedCopilotModels>>>,
    model_catalog_lock: Mutex<()>,
    /// Copilot API endpoint cache (key = GitHub user ID, obtained from /copilot_internal/user)
    api_endpoints: Arc<RwLock<HashMap<String, String>>>,
    /// Endpoint pull lock for each account to avoid repeated pulls to the GitHub API
    endpoint_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
    /// storage path
    storage_path: PathBuf,
}

impl CopilotAuthManager {
    /// Create a new authentication manager
    pub fn new(data_dir: PathBuf) -> Self {
        let storage_path = data_dir.join("copilot_auth.json");

        let manager = Self {
            accounts: Arc::new(RwLock::new(HashMap::new())),
            default_account_id: Arc::new(RwLock::new(None)),
            refresh_locks: Arc::new(RwLock::new(HashMap::new())),
            copilot_tokens: Arc::new(RwLock::new(HashMap::new())),
            copilot_models: Arc::new(RwLock::new(HashMap::new())),
            model_catalog_lock: Mutex::new(()),
            api_endpoints: Arc::new(RwLock::new(HashMap::new())),
            endpoint_locks: Arc::new(RwLock::new(HashMap::new())),
            storage_path,
        };

        // Attempt to load from disk (synchronous, no network request initiated)
        if let Err(e) = manager.load_from_disk_sync() {
            log::warn!("[CopilotAuth] Failed to load storage: {e}");
        }

        manager
    }

    // ==================== Multiple account management methods ====================

    /// Remove specified account
    pub async fn remove_account(&self, account_id: &str) -> Result<(), CopilotAuthError> {
        log::info!("[CopilotAuth] Remove account: {account_id}");

        {
            let mut accounts = self.accounts.write().await;
            if accounts.remove(account_id).is_none() {
                return Err(CopilotAuthError::AccountNotFound(account_id.to_string()));
            }
        }

        // Also remove cached Copilot tokens
        {
            let mut tokens = self.copilot_tokens.write().await;
            tokens.remove(account_id);
        }
        {
            let mut models = self.copilot_models.write().await;
            models.remove(account_id);
        }
        {
            let mut refresh_locks = self.refresh_locks.write().await;
            refresh_locks.remove(account_id);
        }
        // Clean API endpoint cache
        {
            let mut api_endpoints = self.api_endpoints.write().await;
            api_endpoints.remove(account_id);
        }
        {
            let mut endpoint_locks = self.endpoint_locks.write().await;
            endpoint_locks.remove(account_id);
        }

        {
            let accounts = self.accounts.read().await;
            let mut default_account_id = self.default_account_id.write().await;
            if default_account_id.as_deref() == Some(account_id) {
                *default_account_id = Self::fallback_default_account_id(&accounts);
            }
        }

        // persistence
        self.save_to_disk().await?;

        Ok(())
    }

    /// Add new account (internal method, called after OAuth is completed)
    async fn add_account_internal(
        &self,
        github_token: String,
        user: GitHubUser,
        github_domain: String,
    ) -> Result<GitHubAccount, CopilotAuthError> {
        let account_id = composite_account_id(&github_domain, user.id);
        let now = chrono::Utc::now().timestamp();

        let account_data = GitHubAccountData {
            github_token,
            user: user.clone(),
            authenticated_at: now,
            github_domain: github_domain.clone(),
        };

        let account = GitHubAccount {
            id: account_id.clone(),
            login: user.login.clone(),
            avatar_url: user.avatar_url.clone(),
            authenticated_at: now,
            github_domain,
            reauth_required: false,
        };

        {
            let mut accounts = self.accounts.write().await;
            accounts.insert(account_id, account_data);
        }

        {
            let mut default_account_id = self.default_account_id.write().await;
            if default_account_id.is_none() {
                *default_account_id = Some(account.id.clone());
            }
        }

        // persistence
        self.save_to_disk().await?;

        log::info!("[CopilotAuth] Account added successfully: {}", user.login);

        Ok(account)
    }

    /// Set default account
    pub async fn set_default_account(&self, account_id: &str) -> Result<(), CopilotAuthError> {
        {
            let accounts = self.accounts.read().await;
            if !accounts.contains_key(account_id) {
                return Err(CopilotAuthError::AccountNotFound(account_id.to_string()));
            }
        }

        {
            let mut default_account_id = self.default_account_id.write().await;
            *default_account_id = Some(account_id.to_string());
        }

        self.save_to_disk().await?;
        Ok(())
    }

    // ==================== Device code process ====================

    /// Start the device code process
    pub async fn start_device_flow(
        &self,
        github_domain: Option<&str>,
    ) -> Result<GitHubDeviceCodeResponse, CopilotAuthError> {
        let domain = match github_domain {
            Some(d) => normalize_github_domain(d)?,
            None => DEFAULT_GITHUB_DOMAIN.to_string(),
        };
        log::info!("[CopilotAuth] Start device code process (domain: {domain})");

        let response = crate::proxy::http_client::get()
            .post(github_device_code_url(&domain))
            .header("Accept", "application/json")
            .header("User-Agent", COPILOT_USER_AGENT)
            .form(&[
                ("client_id", github_client_id(&domain)),
                ("scope", "read:user"),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CopilotAuthError::NetworkError(format!(
                "GitHub device code request failed: {status} - {text}"
            )));
        }

        let device_code: GitHubDeviceCodeResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        log::info!(
            "[CopilotAuth] Obtained device code successfully, user_code: {}",
            device_code.user_code
        );

        Ok(device_code)
    }

    /// Poll to obtain OAuth Token (return the newly added account if successful)
    pub async fn poll_for_token(
        &self,
        device_code: &str,
        github_domain: Option<&str>,
    ) -> Result<Option<GitHubAccount>, CopilotAuthError> {
        let domain = match github_domain {
            Some(d) => normalize_github_domain(d)?,
            None => DEFAULT_GITHUB_DOMAIN.to_string(),
        };
        log::debug!("[CopilotAuth] Poll OAuth Token (domain: {domain})");

        let response = crate::proxy::http_client::get()
            .post(github_oauth_token_url(&domain))
            .header("Accept", "application/json")
            .header("User-Agent", COPILOT_USER_AGENT)
            .form(&[
                ("client_id", github_client_id(&domain)),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await?;

        let oauth_response: GitHubOAuthResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        // Check for errors
        if let Some(error) = oauth_response.error {
            return match error.as_str() {
                "authorization_pending" => Err(CopilotAuthError::AuthorizationPending),
                "slow_down" => Err(CopilotAuthError::AuthorizationPending),
                "expired_token" => Err(CopilotAuthError::ExpiredToken),
                "access_denied" => Err(CopilotAuthError::AccessDenied),
                _ => Err(CopilotAuthError::NetworkError(format!(
                    "{}: {}",
                    error,
                    oauth_response.error_description.unwrap_or_default()
                ))),
            };
        }

        // Get access_token
        let access_token = oauth_response
            .access_token
            .ok_or_else(|| CopilotAuthError::ParseError("access_token missing".to_string()))?;

        log::info!("[CopilotAuth] OAuth Token obtained successfully");

        // Get user information
        let user = self
            .fetch_user_info_with_token(&access_token, &domain)
            .await?;

        // GHES does not need to exchange for Copilot Token and directly uses OAuth token as Bearer.
        // Refer to the implementation of OpenCode: GHE Copilot directly uses OAuth token to call copilot-api.{domain}
        if !is_ghes(&domain) {
            // github.com: Verify Copilot subscription (get Copilot Token)
            self.fetch_copilot_token_with_github_token(
                &access_token,
                &user.id.to_string(),
                &domain,
            )
            .await?;
        } else {
            log::info!("[CopilotAuth] GHES account, skip Copilot Token exchange and use OAuth token directly");
        }

        // Add account
        let account = self
            .add_account_internal(access_token, user, domain)
            .await?;

        Ok(Some(account))
    }

    // ==================== Token acquisition method ====================

    /// Get the valid Copilot Token of the specified account (automatically refreshed)
    pub async fn get_valid_token_for_account(
        &self,
        account_id: &str,
    ) -> Result<String, CopilotAuthError> {
        // GHES accounts use GitHub OAuth tokens directly without Copilot token exchange.
        let domain = self.get_account_domain(account_id).await;
        if is_ghes(&domain) {
            let accounts = self.accounts.read().await;
            return accounts
                .get(account_id)
                .map(|a| a.github_token.clone())
                .ok_or_else(|| CopilotAuthError::AccountNotFound(account_id.to_string()));
        }

        // Check cached tokens
        {
            let tokens = self.copilot_tokens.read().await;
            if let Some(copilot_token) = tokens.get(account_id) {
                if !copilot_token.is_expiring_soon() {
                    return Ok(copilot_token.token.clone());
                }
            }
        }

        // Need to refresh
        log::info!("[CopilotAuth] The Copilot Token of account {account_id} needs to be refreshed");

        let refresh_lock = self.get_refresh_lock(account_id).await;
        let _refresh_guard = refresh_lock.lock().await;

        // double-check: While waiting for the lock, the refresh may have been completed by other requests
        {
            let tokens = self.copilot_tokens.read().await;
            if let Some(copilot_token) = tokens.get(account_id) {
                if !copilot_token.is_expiring_soon() {
                    return Ok(copilot_token.token.clone());
                }
            }
        }

        // Get the GitHub token of the account
        let (github_token, domain) = {
            let accounts = self.accounts.read().await;
            let account = accounts
                .get(account_id)
                .ok_or_else(|| CopilotAuthError::AccountNotFound(account_id.to_string()))?;
            (account.github_token.clone(), account.github_domain.clone())
        };

        // Refresh Copilot token
        self.fetch_copilot_token_with_github_token(&github_token, account_id, &domain)
            .await?;

        // Return new token
        let tokens = self.copilot_tokens.read().await;
        tokens.get(account_id).map(|t| t.token.clone()).ok_or(
            CopilotAuthError::CopilotTokenFetchFailed("Still no token after refresh".to_string()),
        )
    }

    /// Get a valid Copilot Token (backward compatibility: use the first account)
    pub async fn get_valid_token(&self) -> Result<String, CopilotAuthError> {
        match self.resolve_default_account_id().await {
            Some(id) => self.get_valid_token_for_account(&id).await,
            None => Err(CopilotAuthError::GitHubTokenInvalid),
        }
    }

    // ==================== Models and Usage ====================

    /// Get the list of Copilot available models for the specified account
    pub async fn fetch_models_for_account(
        &self,
        account_id: &str,
    ) -> Result<Vec<CopilotModel>, CopilotAuthError> {
        Ok(self
            .load_models_for_account(account_id, true)
            .await?
            .into_iter()
            .filter(super::copilot_model_map::is_selectable_model)
            .collect())
    }

    async fn load_models_for_account(
        &self,
        account_id: &str,
        force_refresh: bool,
    ) -> Result<Vec<CopilotModel>, CopilotAuthError> {
        {
            let models = self.copilot_models.read().await;
            if !force_refresh {
                if let Some(cached) = models
                    .get(account_id)
                    .and_then(CachedCopilotModels::fresh_models)
                {
                    return Ok(cached);
                }
            }
        }

        let _guard = self.model_catalog_lock.lock().await;
        if !force_refresh {
            if let Some(cached) = self
                .copilot_models
                .read()
                .await
                .get(account_id)
                .and_then(CachedCopilotModels::fresh_models)
            {
                return Ok(cached);
            }
        }
        let models = self.fetch_models_for_account_uncached(account_id).await?;
        {
            let mut cache = self.copilot_models.write().await;
            cache.insert(
                account_id.to_string(),
                CachedCopilotModels {
                    models: models.clone(),
                    fetched_at: Instant::now(),
                },
            );
        }
        Ok(models)
    }

    async fn fetch_models_for_account_uncached(
        &self,
        account_id: &str,
    ) -> Result<Vec<CopilotModel>, CopilotAuthError> {
        let copilot_token = self.get_valid_token_for_account(account_id).await?;

        // Use get_api_endpoint() to dynamically resolve the Copilot API base URL.
        // For github.com accounts, /copilot_internal/user will be queried to obtain the endpoints.api field.
        // For GHES accounts, /copilot_internal/user may not return endpoints - at this time
        // get_api_endpoint() will fall back to copilot_api_base(&domain), which is the same as the previous static URL
        // The splicing results are consistent. This fallback behavior is safe and expected.
        let api_base = self.get_api_endpoint(account_id).await;
        let models_url = format!("{}/models", api_base);

        log::info!("[CopilotAuth] Get the Copilot available models for account {account_id}");

        let response = crate::proxy::http_client::get()
            .get(&models_url)
            .header("Authorization", format!("Bearer {copilot_token}"))
            .header("Content-Type", "application/json")
            .header("copilot-integration-id", "vscode-chat")
            .header("editor-version", COPILOT_EDITOR_VERSION)
            .header("editor-plugin-version", COPILOT_PLUGIN_VERSION)
            .header("user-agent", COPILOT_USER_AGENT)
            .header("x-github-api-version", COPILOT_API_VERSION)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CopilotAuthError::CopilotTokenFetchFailed(format!(
                "Failed to get model list: {status} - {text}"
            )));
        }

        let models_response: CopilotModelsResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        let models: Vec<CopilotModel> = models_response
            .data
            .into_iter()
            .map(CopilotModel::from)
            .collect();

        log::info!("[CopilotAuth] Obtained {} available models", models.len());

        Ok(models)
    }

    pub async fn resolve_model_for_account(
        &self,
        account_id: &str,
        model_id: &str,
    ) -> Result<Option<super::copilot_model_map::ResolvedCopilotModel>, CopilotAuthError> {
        let models = self.load_models_for_account(account_id, false).await?;
        Ok(super::copilot_model_map::resolve_model(model_id, &models))
    }

    /// Get the list of available models for Copilot (backward compatibility: use the first account)
    pub async fn fetch_models(&self) -> Result<Vec<CopilotModel>, CopilotAuthError> {
        match self.resolve_default_account_id().await {
            Some(id) => self.fetch_models_for_account(&id).await,
            None => Err(CopilotAuthError::GitHubTokenInvalid),
        }
    }

    pub async fn resolve_model(
        &self,
        model_id: &str,
    ) -> Result<Option<super::copilot_model_map::ResolvedCopilotModel>, CopilotAuthError> {
        match self.resolve_default_account_id().await {
            Some(id) => self.resolve_model_for_account(&id, model_id).await,
            None => Err(CopilotAuthError::GitHubTokenInvalid),
        }
    }

    /// Get Copilot usage information for a specified account
    pub async fn fetch_usage_for_account(
        &self,
        account_id: &str,
    ) -> Result<CopilotUsageResponse, CopilotAuthError> {
        let (github_token, domain) = {
            let accounts = self.accounts.read().await;
            let account = accounts
                .get(account_id)
                .ok_or_else(|| CopilotAuthError::AccountNotFound(account_id.to_string()))?;
            (account.github_token.clone(), account.github_domain.clone())
        };

        log::info!("[CopilotAuth] Get the Copilot usage of account {account_id}");

        let response = crate::proxy::http_client::get()
            .get(copilot_usage_url(&domain))
            .header("Authorization", format!("token {github_token}"))
            .header("Content-Type", "application/json")
            .header("editor-version", COPILOT_EDITOR_VERSION)
            .header("editor-plugin-version", COPILOT_PLUGIN_VERSION)
            .header("user-agent", COPILOT_USER_AGENT)
            .header("x-github-api-version", COPILOT_API_VERSION)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CopilotAuthError::GitHubTokenInvalid);
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CopilotAuthError::CopilotTokenFetchFailed(format!(
                "Failed to obtain usage: {status} - {text}"
            )));
        }

        let usage: CopilotUsageResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        // Stores dynamic API endpoints (if any)
        if let Some(ref endpoints) = usage.endpoints {
            let mut api_endpoints = self.api_endpoints.write().await;
            api_endpoints.insert(account_id.to_string(), endpoints.api.clone());
            // Use debug level to avoid exposing internal domain names in logs
            log::debug!("[CopilotAuth] Account {account_id} has saved dynamic API endpoints");
        }

        log::info!(
            "[CopilotAuth] Obtained usage successfully, plan: {}, reset date: {}",
            usage.copilot_plan,
            usage.quota_reset_date
        );

        Ok(usage)
    }

    /// Get Copilot usage information (backward compatibility: use first account)
    pub async fn fetch_usage(&self) -> Result<CopilotUsageResponse, CopilotAuthError> {
        match self.resolve_default_account_id().await {
            Some(id) => self.fetch_usage_for_account(&id).await,
            None => Err(CopilotAuthError::GitHubTokenInvalid),
        }
    }

    // ==================== Status Query ====================

    /// Obtain the API endpoint of the specified account (return directly if cache hits, lazily pull from API if not)
    pub async fn get_api_endpoint(&self, account_id: &str) -> String {
        {
            let endpoints = self.api_endpoints.read().await;
            if let Some(endpoint) = endpoints.get(account_id) {
                return endpoint.clone();
            }
        }

        // Use locks to serialize concurrent pulls for the same account to avoid repeated requests to the GitHub API
        let lock = self.get_endpoint_lock(account_id).await;
        let _guard = lock.lock().await;

        // Second check after lock holding: may have been filled by other requests
        {
            let endpoints = self.api_endpoints.read().await;
            if let Some(endpoint) = endpoints.get(account_id) {
                return endpoint.clone();
            }
        }

        match self.fetch_and_cache_endpoint(account_id).await {
            Ok(endpoint) => endpoint,
            Err(e) => {
                log::debug!(
                    "[CopilotAuth] Failed to get account {account_id} dynamic API endpoint: {e}, use default value"
                );
                let domain = self.get_account_domain(account_id).await;
                copilot_api_base(&domain)
            }
        }
    }

    /// Get the API endpoint of the default account
    pub async fn get_default_api_endpoint(&self) -> String {
        match self.resolve_default_account_id().await {
            Some(id) => self.get_api_endpoint(&id).await,
            None => {
                // Fall back to the default endpoint of github.com when there is no account
                copilot_api_base(DEFAULT_GITHUB_DOMAIN)
            }
        }
    }

    async fn fetch_and_cache_endpoint(&self, account_id: &str) -> Result<String, CopilotAuthError> {
        let (github_token, domain) = {
            let accounts = self.accounts.read().await;
            let account = accounts
                .get(account_id)
                .ok_or_else(|| CopilotAuthError::AccountNotFound(account_id.to_string()))?;
            (account.github_token.clone(), account.github_domain.clone())
        };

        log::debug!("[CopilotAuth] Lazy pull of dynamic API endpoint for account {account_id}");

        let response = crate::proxy::http_client::get()
            .get(copilot_usage_url(&domain))
            .header("Authorization", format!("token {github_token}"))
            .header("Content-Type", "application/json")
            .header("editor-version", COPILOT_EDITOR_VERSION)
            .header("editor-plugin-version", COPILOT_PLUGIN_VERSION)
            .header("user-agent", COPILOT_USER_AGENT)
            .header("x-github-api-version", COPILOT_API_VERSION)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CopilotAuthError::GitHubTokenInvalid);
        }

        if !response.status().is_success() {
            return Err(CopilotAuthError::CopilotTokenFetchFailed(format!(
                "Failed to get API endpoint: {}",
                response.status()
            )));
        }

        let usage: CopilotUsageResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        let endpoint = match usage.endpoints {
            Some(endpoints) => endpoints.api.clone(),
            None => copilot_api_base(&domain),
        };

        // Cache endpoints (including defaults) to avoid duplicate requests
        let mut api_endpoints = self.api_endpoints.write().await;
        api_endpoints.insert(account_id.to_string(), endpoint.clone());
        log::debug!("[CopilotAuth] Account {account_id} cached API endpoint");

        Ok(endpoint)
    }

    async fn get_endpoint_lock(&self, account_id: &str) -> Arc<Mutex<()>> {
        {
            let locks = self.endpoint_locks.read().await;
            if let Some(lock) = locks.get(account_id) {
                return Arc::clone(lock);
            }
        }

        let mut locks = self.endpoint_locks.write().await;
        Arc::clone(
            locks
                .entry(account_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    /// Get authentication status (supports multiple accounts)
    pub async fn get_status(&self) -> CopilotAuthStatus {
        let accounts = self.accounts.read().await.clone();
        let default_account_id = self.resolve_default_account_id().await;

        let account_list = Self::sorted_accounts(&accounts, default_account_id.as_deref());
        let authenticated = !account_list.is_empty();

        CopilotAuthStatus {
            accounts: account_list,
            default_account_id,
            authenticated,
        }
    }

    /// Check whether it is authenticated (with any account)
    pub async fn is_authenticated(&self) -> bool {
        let accounts = self.accounts.read().await;
        !accounts.is_empty()
    }

    /// Clear all certifications (log out of all accounts)
    pub async fn clear_auth(&self) -> Result<(), CopilotAuthError> {
        log::info!("[CopilotAuth] Clear all authentication");

        // First clean up the memory status to ensure that even if the file deletion fails, the user can see that they have logged out.
        {
            let mut accounts = self.accounts.write().await;
            accounts.clear();
        }
        {
            let mut default_account_id = self.default_account_id.write().await;
            default_account_id.take();
        }
        {
            let mut tokens = self.copilot_tokens.write().await;
            tokens.clear();
        }
        {
            let mut models = self.copilot_models.write().await;
            models.clear();
        }
        {
            let mut refresh_locks = self.refresh_locks.write().await;
            refresh_locks.clear();
        }
        // Clean API endpoint cache
        {
            let mut api_endpoints = self.api_endpoints.write().await;
            api_endpoints.clear();
        }
        {
            let mut endpoint_locks = self.endpoint_locks.write().await;
            endpoint_locks.clear();
        }

        // Finally delete the stored file
        if self.storage_path.exists() {
            std::fs::remove_file(&self.storage_path)?;
        }

        Ok(())
    }

    // ==================== Internal methods ====================

    fn fallback_default_account_id(
        accounts: &HashMap<String, GitHubAccountData>,
    ) -> Option<String> {
        accounts
            .iter()
            .max_by(|(id_a, a), (id_b, b)| {
                a.authenticated_at
                    .cmp(&b.authenticated_at)
                    .then_with(|| id_b.cmp(id_a))
            })
            .map(|(id, _)| id.clone())
    }

    fn sorted_accounts(
        accounts: &HashMap<String, GitHubAccountData>,
        default_account_id: Option<&str>,
    ) -> Vec<GitHubAccount> {
        let mut account_list: Vec<GitHubAccount> =
            accounts.values().map(GitHubAccount::from).collect();
        account_list.sort_by(|a, b| {
            let a_default = default_account_id == Some(a.id.as_str());
            let b_default = default_account_id == Some(b.id.as_str());

            b_default
                .cmp(&a_default)
                .then_with(|| b.authenticated_at.cmp(&a.authenticated_at))
                .then_with(|| a.login.cmp(&b.login))
        });
        account_list
    }

    async fn resolve_default_account_id(&self) -> Option<String> {
        let stored_default = self.default_account_id.read().await.clone();
        let accounts = self.accounts.read().await;

        if let Some(default_id) = stored_default {
            if accounts.contains_key(&default_id) {
                return Some(default_id);
            }
        }

        Self::fallback_default_account_id(&accounts)
    }

    /// Get the GitHub domain name of the specified account
    async fn get_account_domain(&self, account_id: &str) -> String {
        let accounts = self.accounts.read().await;
        accounts
            .get(account_id)
            .map(|a| a.github_domain.clone())
            .unwrap_or_else(|| DEFAULT_GITHUB_DOMAIN.to_string())
    }

    async fn get_refresh_lock(&self, account_id: &str) -> Arc<Mutex<()>> {
        {
            let refresh_locks = self.refresh_locks.read().await;
            if let Some(lock) = refresh_locks.get(account_id) {
                return Arc::clone(lock);
            }
        }

        let mut refresh_locks = self.refresh_locks.write().await;
        Arc::clone(
            refresh_locks
                .entry(account_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    fn write_store_atomic(&self, content: &str) -> Result<(), CopilotAuthError> {
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let parent = self
            .storage_path
            .parent()
            .ok_or_else(|| CopilotAuthError::IoError("Invalid storage path".to_string()))?;
        let file_name = self
            .storage_path
            .file_name()
            .ok_or_else(|| CopilotAuthError::IoError("Invalid storage file name".to_string()))?
            .to_string_lossy()
            .to_string();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let tmp_path = parent.join(format!("{file_name}.tmp.{ts}"));

        {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;

            if self.storage_path.exists() {
                let _ = fs::remove_file(&self.storage_path);
            }
            fs::rename(&tmp_path, &self.storage_path)?;
        }

        Ok(())
    }

    /// Obtain GitHub user information using the specified token
    async fn fetch_user_info_with_token(
        &self,
        github_token: &str,
        domain: &str,
    ) -> Result<GitHubUser, CopilotAuthError> {
        let response = crate::proxy::http_client::get()
            .get(github_user_url(domain))
            .header("Authorization", format!("token {github_token}"))
            .header("User-Agent", COPILOT_USER_AGENT)
            .header("Editor-Version", COPILOT_EDITOR_VERSION)
            .header("Editor-Plugin-Version", COPILOT_PLUGIN_VERSION)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(CopilotAuthError::GitHubTokenInvalid);
        }

        let user: GitHubUser = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        log::info!(
            "[CopilotAuth] Obtained user information successfully: {}",
            user.login
        );

        Ok(user)
    }

    /// Get Copilot Token using GitHub token
    async fn fetch_copilot_token_with_github_token(
        &self,
        github_token: &str,
        account_id: &str,
        domain: &str,
    ) -> Result<(), CopilotAuthError> {
        log::debug!(
            "[CopilotAuth] Get the Copilot Token of account {account_id} (domain: {domain})"
        );

        let response = crate::proxy::http_client::get()
            .get(copilot_token_url(domain))
            .header("Authorization", format!("token {github_token}"))
            .header("User-Agent", COPILOT_USER_AGENT)
            .header("Editor-Version", COPILOT_EDITOR_VERSION)
            .header("Editor-Plugin-Version", COPILOT_PLUGIN_VERSION)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CopilotAuthError::GitHubTokenInvalid);
        }

        if response.status() == reqwest::StatusCode::FORBIDDEN {
            return Err(CopilotAuthError::NoCopilotSubscription);
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CopilotAuthError::CopilotTokenFetchFailed(format!(
                "{status}: {text}"
            )));
        }

        let token_response: CopilotTokenResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        log::info!(
            "[CopilotAuth] The Copilot Token of account {} was successfully obtained, and the expiration time is: {}",
            account_id,
            token_response.expires_at
        );

        let copilot_token = CopilotToken {
            token: token_response.token,
            expires_at: token_response.expires_at,
        };

        let mut tokens = self.copilot_tokens.write().await;
        tokens.insert(account_id.to_string(), copilot_token);

        Ok(())
    }

    // ==================== Storage ====================

    /// Load from disk (only load token, do not initiate network request)
    fn load_from_disk_sync(&self) -> Result<(), CopilotAuthError> {
        if !self.storage_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&self.storage_path)?;
        let store: CopilotAuthStore = serde_json::from_str(&content)
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        if store.version != 6 {
            return Err(CopilotAuthError::ParseError(format!(
                "Unsupported Copilot auth store version {}; Atlas 6 requires version 6",
                store.version
            )));
        }
        if let Ok(mut accounts) = self.accounts.try_write() {
            *accounts = store.accounts;
            log::info!("[CopilotAuth] Load {} accounts from disk", accounts.len());
        }
        if let Ok(mut default_account_id) = self.default_account_id.try_write() {
            *default_account_id = store.default_account_id;
            if default_account_id.is_none() {
                if let Ok(accounts) = self.accounts.try_read() {
                    *default_account_id = Self::fallback_default_account_id(&accounts);
                }
            }
        }

        Ok(())
    }

    /// save to disk
    async fn save_to_disk(&self) -> Result<(), CopilotAuthError> {
        let accounts = self.accounts.read().await.clone();
        let default_account_id = self.resolve_default_account_id().await;

        let store = CopilotAuthStore {
            version: 6,
            accounts,
            default_account_id,
        };

        let content = serde_json::to_string_pretty(&store)
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        self.write_store_atomic(&content)?;

        log::info!(
            "[CopilotAuth] Saved to disk successfully ({} accounts)",
            store.accounts.len()
        );

        Ok(())
    }
}

fn auth_header_value(value: &str) -> Result<http::HeaderValue, ProxyError> {
    http::HeaderValue::from_str(value)
        .map_err(|error| ProxyError::AuthError(format!("Invalid authentication header: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn model_cache_expires_and_unknown_model_metadata_is_preserved() {
        let cache = CachedCopilotModels {
            models: vec![CopilotModel {
                id: "future-vendor/model".into(),
                ..Default::default()
            }],
            fetched_at: Instant::now(),
        };
        assert_eq!(cache.fresh_models().unwrap()[0].id, "future-vendor/model");
        let expired = CachedCopilotModels {
            fetched_at: Instant::now() - MODEL_CATALOG_TTL,
            ..cache
        };
        assert!(expired.fresh_models().is_none());

        let item: CopilotModelsResponseItem = serde_json::from_value(serde_json::json!({
            "id": "future-vendor/model", "vendor": "New Vendor", "model_picker_enabled": true,
            "supported_endpoints": ["/chat/completions"], "policy": {"state": "enabled"},
            "capabilities": {
                "type": "chat", "limits": {"max_prompt_tokens": 32000, "max_output_tokens": 4096},
                "supports": {"tool_calls": true, "parallel_tool_calls": false, "vision": false, "reasoning_effort": []}
            },
            "future_metadata": {"not_an_allowlist": true}
        })).unwrap();
        let model = CopilotModel::from(item);
        assert_eq!(model.name, "future-vendor/model");
        assert_eq!(model.vendor, "New Vendor");
        assert_eq!(model.max_output_tokens, Some(4096));
        assert_eq!(model.supports_tool_calls, Some(true));
        assert_eq!(model.reasoning_efforts, Some(vec![]));
        assert!(super::super::copilot_model_map::is_selectable_model(&model));
    }

    #[test]
    fn extracts_model_context_and_supported_endpoints() {
        let item: CopilotModelsResponseItem = serde_json::from_value(serde_json::json!({
            "id": "gpt-5.6",
            "name": "GPT-5.6",
            "vendor": "OpenAI",
            "model_picker_enabled": true,
            "supported_endpoints": ["/responses", "/chat/completions"],
            "capabilities": {
                "limits": {
                    "max_context_window_tokens": 400000
                }
            }
        }))
        .unwrap();

        assert_eq!(
            extract_copilot_context_window(item.capabilities.as_ref()),
            Some(400_000)
        );
        assert_eq!(
            item.supported_endpoints,
            vec!["/responses", "/chat/completions"]
        );
    }

    #[test]
    fn live_models_keep_capabilities_and_limit_input_by_the_smaller_server_budget() {
        let item: CopilotModelsResponseItem = serde_json::from_value(serde_json::json!({
            "id": "gpt-6-luna", "name": "GPT-6 Luna", "vendor": "OpenAI",
            "model_picker_enabled": true, "supported_endpoints": ["/responses"],
            "capabilities": {
                "limits": { "max_context_window_tokens": 1000000, "max_prompt_tokens": 872000 },
                "supports": { "parallel_tool_calls": true, "vision": true, "reasoning_effort": ["none", "low", "medium", "max"] }
            }
        })).unwrap();
        let model = CopilotModel::from(item);
        assert_eq!(model.context_window, Some(872000));
        assert_eq!(model.supports_parallel_tool_calls, Some(true));
        assert_eq!(model.supports_vision, Some(true));
        assert_eq!(model.max_context_window_tokens, Some(1000000));
        assert_eq!(
            model.reasoning_efforts.unwrap(),
            ["none", "low", "medium", "max"]
        );
        assert_eq!(extract_copilot_context_window(None), None);
        assert_eq!(
            extract_copilot_context_window(Some(&serde_json::json!({
                "limits": { "max_context_window_tokens": 400000, "max_prompt_tokens": 0 }
            }))),
            Some(400000)
        );
    }

    #[test]
    fn shared_request_headers_include_copilot_identity() {
        let headers = build_copilot_request_headers("test-token").unwrap();
        let headers = headers.into_iter().collect::<http::HeaderMap>();

        assert_eq!(
            headers
                .get(http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer test-token")
        );
        assert_eq!(
            headers
                .get("editor-version")
                .and_then(|value| value.to_str().ok()),
            Some(COPILOT_EDITOR_VERSION)
        );
        assert_eq!(
            headers
                .get("copilot-integration-id")
                .and_then(|value| value.to_str().ok()),
            Some(COPILOT_INTEGRATION_ID)
        );
        assert!(headers.contains_key("x-request-id"));
        assert_eq!(headers.get("x-request-id"), headers.get("x-agent-task-id"));
    }

    #[test]
    fn test_copilot_token_expiry() {
        let now = chrono::Utc::now().timestamp();

        // Unexpired token (expires after 1 hour, not within the 60 second buffer period)
        let token = CopilotToken {
            token: "test".to_string(),
            expires_at: now + 3600,
        };
        assert!(!token.is_expiring_soon());

        // Token that is about to expire (expires in 30 seconds, within 60 seconds buffer period)
        let token = CopilotToken {
            token: "test".to_string(),
            expires_at: now + 30,
        };
        assert!(token.is_expiring_soon());

        // Expired token (also within the buffer period)
        let token = CopilotToken {
            token: "test".to_string(),
            expires_at: now - 100,
        };
        assert!(token.is_expiring_soon());
    }

    #[test]
    fn test_auth_status_serialization() {
        let status = CopilotAuthStatus {
            accounts: vec![GitHubAccount {
                id: "12345".to_string(),
                login: "testuser".to_string(),
                avatar_url: Some("https://example.com/avatar.png".to_string()),
                authenticated_at: 1234567890,
                github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
                reauth_required: false,
            }],
            default_account_id: Some("12345".to_string()),
            authenticated: true,
        };

        let json = serde_json::to_string(&status).unwrap();
        let parsed: CopilotAuthStatus = serde_json::from_str(&json).unwrap();

        assert!(parsed.authenticated);
        assert_eq!(parsed.default_account_id, Some("12345".to_string()));
        assert_eq!(parsed.accounts.len(), 1);
        assert_eq!(parsed.accounts[0].id, "12345");
        assert_eq!(parsed.accounts[0].login, "testuser");
    }

    #[test]
    fn test_multi_account_store_serialization() {
        let mut accounts = HashMap::new();
        accounts.insert(
            "12345".to_string(),
            GitHubAccountData {
                github_token: "gho_test_token".to_string(),
                user: GitHubUser {
                    login: "alice".to_string(),
                    id: 12345,
                    avatar_url: Some("https://example.com/alice.png".to_string()),
                },
                authenticated_at: 1700000000,
                github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
            },
        );
        accounts.insert(
            "67890".to_string(),
            GitHubAccountData {
                github_token: "gho_test_token_2".to_string(),
                user: GitHubUser {
                    login: "bob".to_string(),
                    id: 67890,
                    avatar_url: None,
                },
                authenticated_at: 1700000001,
                github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
            },
        );

        let store = CopilotAuthStore {
            version: 6,
            accounts,
            default_account_id: Some("67890".to_string()),
        };

        let json = serde_json::to_string_pretty(&store).unwrap();
        let parsed: CopilotAuthStore = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, 6);
        assert_eq!(parsed.default_account_id, Some("67890".to_string()));
        assert_eq!(parsed.accounts.len(), 2);
        assert!(parsed.accounts.contains_key("12345"));
        assert!(parsed.accounts.contains_key("67890"));
        assert_eq!(parsed.accounts["12345"].user.login, "alice");
        assert_eq!(parsed.accounts["67890"].user.login, "bob");
    }

    #[test]
    fn test_github_account_from_data() {
        let data = GitHubAccountData {
            github_token: "gho_test".to_string(),
            user: GitHubUser {
                login: "testuser".to_string(),
                id: 99999,
                avatar_url: Some("https://example.com/avatar.png".to_string()),
            },
            authenticated_at: 1700000000,
            github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
        };

        let account = GitHubAccount::from(&data);
        assert_eq!(account.id, "99999");
        assert_eq!(account.login, "testuser");
        assert_eq!(
            account.avatar_url,
            Some("https://example.com/avatar.png".to_string())
        );
        assert_eq!(account.authenticated_at, 1700000000);
    }

    #[test]
    fn test_fallback_default_account_prefers_latest_authenticated() {
        let mut accounts = HashMap::new();
        accounts.insert(
            "12345".to_string(),
            GitHubAccountData {
                github_token: "gho_test_token".to_string(),
                user: GitHubUser {
                    login: "alice".to_string(),
                    id: 12345,
                    avatar_url: None,
                },
                authenticated_at: 1700000000,
                github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
            },
        );
        accounts.insert(
            "67890".to_string(),
            GitHubAccountData {
                github_token: "gho_test_token_2".to_string(),
                user: GitHubUser {
                    login: "bob".to_string(),
                    id: 67890,
                    avatar_url: None,
                },
                authenticated_at: 1700000001,
                github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
            },
        );

        assert_eq!(
            CopilotAuthManager::fallback_default_account_id(&accounts),
            Some("67890".to_string())
        );
    }

    #[tokio::test]
    async fn test_get_api_endpoint_returns_cached_value() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        // Manually set api_endpoints cache
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert(
                "12345".to_string(),
                "https://copilot-api.enterprise.example.com".to_string(),
            );
        }

        let endpoint = manager.get_api_endpoint("12345").await;
        assert_eq!(endpoint, "https://copilot-api.enterprise.example.com");
    }

    #[tokio::test]
    async fn test_get_api_endpoint_returns_default_when_not_cached() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        let endpoint = manager.get_api_endpoint("99999").await;
        assert_eq!(endpoint, "https://api.githubcopilot.com");
    }

    #[tokio::test]
    async fn test_get_default_api_endpoint_uses_default_account() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        // Set default account
        {
            let mut default_account_id = manager.default_account_id.write().await;
            *default_account_id = Some("12345".to_string());
        }
        // Add account data
        {
            let mut accounts = manager.accounts.write().await;
            accounts.insert(
                "12345".to_string(),
                GitHubAccountData {
                    github_token: "gho_test".to_string(),
                    user: GitHubUser {
                        login: "alice".to_string(),
                        id: 12345,
                        avatar_url: None,
                    },
                    authenticated_at: 1700000000,
                    github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
                },
            );
        }
        // Set up API endpoint caching
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert(
                "12345".to_string(),
                "https://copilot-api.corp.example.com".to_string(),
            );
        }

        let endpoint = manager.get_default_api_endpoint().await;
        assert_eq!(endpoint, "https://copilot-api.corp.example.com");
    }

    #[tokio::test]
    async fn test_remove_account_clears_api_endpoint_cache() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        // Add account data
        {
            let mut accounts = manager.accounts.write().await;
            accounts.insert(
                "12345".to_string(),
                GitHubAccountData {
                    github_token: "gho_test".to_string(),
                    user: GitHubUser {
                        login: "alice".to_string(),
                        id: 12345,
                        avatar_url: None,
                    },
                    authenticated_at: 1700000000,
                    github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
                },
            );
        }
        // Set up API endpoint caching
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert(
                "12345".to_string(),
                "https://copilot-api.enterprise.example.com".to_string(),
            );
        }

        // Confirm cache exists
        {
            let api_endpoints = manager.api_endpoints.read().await;
            assert!(api_endpoints.contains_key("12345"));
        }

        // Remove account
        manager.remove_account("12345").await.unwrap();

        // Confirm cache has been cleared
        {
            let api_endpoints = manager.api_endpoints.read().await;
            assert!(!api_endpoints.contains_key("12345"));
        }
    }

    #[tokio::test]
    async fn test_clear_auth_clears_all_api_endpoint_cache() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        // Add API endpoint cache for multiple accounts
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert(
                "12345".to_string(),
                "https://copilot-api.enterprise1.example.com".to_string(),
            );
            api_endpoints.insert(
                "67890".to_string(),
                "https://copilot-api.enterprise2.example.com".to_string(),
            );
        }

        // Confirm cache exists
        {
            let api_endpoints = manager.api_endpoints.read().await;
            assert_eq!(api_endpoints.len(), 2);
        }

        // Clear all certifications
        manager.clear_auth().await.unwrap();

        // Confirm cache has been cleared
        {
            let api_endpoints = manager.api_endpoints.read().await;
            assert!(api_endpoints.is_empty());
        }
    }

    #[tokio::test]
    async fn test_clear_auth_cleans_memory_even_when_file_removal_fails() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        // Create a directory at storage_path so remove_file fails
        std::fs::create_dir_all(&manager.storage_path).unwrap();

        {
            let mut accounts = manager.accounts.write().await;
            accounts.insert(
                "12345".to_string(),
                GitHubAccountData {
                    github_token: "gho_test".to_string(),
                    user: GitHubUser {
                        login: "alice".to_string(),
                        id: 12345,
                        avatar_url: None,
                    },
                    authenticated_at: 1700000000,
                    github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
                },
            );
        }
        {
            let mut default_account_id = manager.default_account_id.write().await;
            *default_account_id = Some("12345".to_string());
        }
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert(
                "12345".to_string(),
                "https://copilot-api.enterprise.example.com".to_string(),
            );
        }

        let result = manager.clear_auth().await;
        // Should still return an error for the file deletion failure
        assert!(result.is_err());

        // But memory state should already be cleaned
        let accounts = manager.accounts.read().await;
        assert!(accounts.is_empty());
        drop(accounts);

        let default_account_id = manager.default_account_id.read().await;
        assert!(default_account_id.is_none());
        drop(default_account_id);

        let api_endpoints = manager.api_endpoints.read().await;
        assert!(api_endpoints.is_empty());
    }

    #[tokio::test]
    async fn test_get_api_endpoint_cache_hit_skips_fetch() {
        // When the cache is hit, it should be returned directly without initiating a network request.
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        let enterprise_endpoint = "https://copilot-api.enterprise.example.com".to_string();
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert("12345".to_string(), enterprise_endpoint.clone());
        }

        // Even if there is no account data, cache hits should be returned directly
        let endpoint = manager.get_api_endpoint("12345").await;
        assert_eq!(endpoint, enterprise_endpoint);
    }

    #[tokio::test]
    async fn test_get_api_endpoint_returns_default_for_unknown_account() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        let endpoint = manager.get_api_endpoint("12345").await;
        assert_eq!(endpoint, copilot_api_base(DEFAULT_GITHUB_DOMAIN));
    }

    #[tokio::test]
    async fn test_fetch_and_cache_endpoint_requires_account() {
        // When the account does not exist, fetch_and_cache_endpoint should return AccountNotFound error
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        let result = manager.fetch_and_cache_endpoint("nonexistent").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CopilotAuthError::AccountNotFound(id) => assert_eq!(id, "nonexistent"),
            other => panic!("Expected AccountNotFound error, actual: {other:?}"),
        }
    }

    #[test]
    fn test_normalize_github_domain() {
        // Basic usage
        assert_eq!(normalize_github_domain("github.com").unwrap(), "github.com");
        assert_eq!(
            normalize_github_domain("company.ghe.com").unwrap(),
            "company.ghe.com"
        );

        // divestiture agreement
        assert_eq!(
            normalize_github_domain("https://company.ghe.com").unwrap(),
            "company.ghe.com"
        );
        assert_eq!(
            normalize_github_domain("http://company.ghe.com").unwrap(),
            "company.ghe.com"
        );

        // Lowercase
        assert_eq!(normalize_github_domain("GitHub.COM").unwrap(), "github.com");
        assert_eq!(
            normalize_github_domain("Company.GHE.Com").unwrap(),
            "company.ghe.com"
        );

        // Strip trailing slashes and path
        assert_eq!(
            normalize_github_domain("company.ghe.com/").unwrap(),
            "company.ghe.com"
        );
        assert_eq!(
            normalize_github_domain("company.ghe.com/api/v3").unwrap(),
            "company.ghe.com"
        );

        // Strip query and fragment
        assert_eq!(
            normalize_github_domain("company.ghe.com?foo=bar").unwrap(),
            "company.ghe.com"
        );
        assert_eq!(
            normalize_github_domain("company.ghe.com#section").unwrap(),
            "company.ghe.com"
        );

        // reserved port
        assert_eq!(
            normalize_github_domain("company.ghe.com:8443").unwrap(),
            "company.ghe.com:8443"
        );

        // Deny userinfo
        assert!(normalize_github_domain("user@company.ghe.com").is_err());

        // reject empty input
        assert!(normalize_github_domain("").is_err());
        assert!(normalize_github_domain("   ").is_err());
    }

    #[test]
    fn test_composite_account_id() {
        // github.com maintains the original format (backwards compatible)
        assert_eq!(composite_account_id("github.com", 12345), "12345");

        // GHES uses composite format
        assert_eq!(
            composite_account_id("company.ghe.com", 12345),
            "company.ghe.com:12345"
        );

        // Different GHES instances, same user ID, no conflict
        assert_ne!(
            composite_account_id("a.ghe.com", 1),
            composite_account_id("b.ghe.com", 1)
        );
    }

    #[test]
    fn test_github_account_from_data_ghes_uses_composite_id() {
        let data = GitHubAccountData {
            github_token: "gho_test".to_string(),
            user: GitHubUser {
                login: "testuser".to_string(),
                id: 99999,
                avatar_url: None,
            },
            authenticated_at: 1700000000,
            github_domain: "company.ghe.com".to_string(),
        };

        let account = GitHubAccount::from(&data);
        assert_eq!(account.id, "company.ghe.com:99999");
    }
}
