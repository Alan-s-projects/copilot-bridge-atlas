use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Atlas owns this provider record; it never writes it to Codex configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    #[serde(rename = "settingsConfig")]
    pub settings_config: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ProviderMeta>,
}

impl Provider {
    pub fn with_id(id: String, name: String, settings_config: Value) -> Self {
        Self {
            id,
            name,
            settings_config,
            meta: None,
        }
    }

    pub fn is_github_copilot(&self) -> bool {
        self.meta
            .as_ref()
            .and_then(|meta| meta.provider_type.as_deref())
            == Some("github_copilot")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthBinding {
    #[serde(rename = "authProvider", skip_serializing_if = "Option::is_none")]
    pub auth_provider: Option<String>,
    #[serde(rename = "accountId", skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderMeta {
    #[serde(rename = "providerType", skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    #[serde(rename = "authBinding", skip_serializing_if = "Option::is_none")]
    pub auth_binding: Option<AuthBinding>,
}

impl ProviderMeta {
    pub fn managed_account_id_for(&self, auth_provider: &str) -> Option<String> {
        if auth_provider != "github_copilot" {
            return None;
        }
        self.auth_binding.as_ref().and_then(|binding| {
            (binding.auth_provider.as_deref() == Some(auth_provider))
                .then(|| binding.account_id.clone())
                .flatten()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn managed_account_binding_selects_the_saved_account() {
        let meta: ProviderMeta = serde_json::from_value(json!({
            "providerType": "github_copilot",
            "authBinding": {
                "authProvider": "github_copilot",
                "accountId": "account"
            }
        }))
        .unwrap();
        assert_eq!(
            meta.managed_account_id_for("github_copilot").as_deref(),
            Some("account")
        );
    }
}
