use crate::commands::copilot::CopilotAuthState;
use crate::proxy::providers::copilot_auth::{CopilotAuthError, GitHubAccount};
use tauri::State;

const GITHUB_COPILOT: &str = "github_copilot";

#[derive(Debug, Clone, serde::Serialize)]
pub struct ManagedAuthAccount {
    pub id: String,
    pub provider: &'static str,
    pub login: String,
    pub avatar_url: Option<String>,
    pub authenticated_at: i64,
    pub is_default: bool,
    pub github_domain: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ManagedAuthStatus {
    pub provider: &'static str,
    pub authenticated: bool,
    pub default_account_id: Option<String>,
    pub accounts: Vec<ManagedAuthAccount>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ManagedAuthDeviceCodeResponse {
    pub provider: &'static str,
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

fn ensure_github(auth_provider: &str) -> Result<(), String> {
    if auth_provider == GITHUB_COPILOT {
        Ok(())
    } else {
        Err("Only GitHub Copilot authentication is supported".into())
    }
}

fn map_account(account: GitHubAccount, default_id: Option<&str>) -> ManagedAuthAccount {
    ManagedAuthAccount {
        is_default: default_id == Some(account.id.as_str()),
        id: account.id,
        provider: GITHUB_COPILOT,
        login: account.login,
        avatar_url: account.avatar_url,
        authenticated_at: account.authenticated_at,
        github_domain: account.github_domain,
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_start_login(
    auth_provider: String,
    github_domain: Option<String>,
    copilot_state: State<'_, CopilotAuthState>,
) -> Result<ManagedAuthDeviceCodeResponse, String> {
    ensure_github(&auth_provider)?;
    let response = copilot_state
        .0
        .read()
        .await
        .start_device_flow(github_domain.as_deref())
        .await
        .map_err(|error| error.to_string())?;
    Ok(ManagedAuthDeviceCodeResponse {
        provider: GITHUB_COPILOT,
        device_code: response.device_code,
        user_code: response.user_code,
        verification_uri: response.verification_uri,
        expires_in: response.expires_in,
        interval: response.interval,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_poll_for_account(
    auth_provider: String,
    device_code: String,
    github_domain: Option<String>,
    copilot_state: State<'_, CopilotAuthState>,
) -> Result<Option<ManagedAuthAccount>, String> {
    ensure_github(&auth_provider)?;
    let manager = copilot_state.0.write().await;
    match manager
        .poll_for_token(&device_code, github_domain.as_deref())
        .await
    {
        Ok(account) => {
            let default_id = manager.get_status().await.default_account_id;
            Ok(account.map(|account| map_account(account, default_id.as_deref())))
        }
        Err(CopilotAuthError::AuthorizationPending) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_get_status(
    auth_provider: String,
    copilot_state: State<'_, CopilotAuthState>,
) -> Result<ManagedAuthStatus, String> {
    ensure_github(&auth_provider)?;
    let status = copilot_state.0.read().await.get_status().await;
    let default_id = status.default_account_id;
    Ok(ManagedAuthStatus {
        provider: GITHUB_COPILOT,
        authenticated: status.authenticated,
        accounts: status
            .accounts
            .into_iter()
            .map(|account| map_account(account, default_id.as_deref()))
            .collect(),
        default_account_id: default_id,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_remove_account(
    auth_provider: String,
    account_id: String,
    copilot_state: State<'_, CopilotAuthState>,
) -> Result<(), String> {
    ensure_github(&auth_provider)?;
    copilot_state
        .0
        .write()
        .await
        .remove_account(&account_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_set_default_account(
    auth_provider: String,
    account_id: String,
    copilot_state: State<'_, CopilotAuthState>,
) -> Result<(), String> {
    ensure_github(&auth_provider)?;
    copilot_state
        .0
        .write()
        .await
        .set_default_account(&account_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn auth_logout(
    auth_provider: String,
    copilot_state: State<'_, CopilotAuthState>,
) -> Result<(), String> {
    ensure_github(&auth_provider)?;
    copilot_state
        .0
        .write()
        .await
        .clear_auth()
        .await
        .map_err(|error| error.to_string())
}
