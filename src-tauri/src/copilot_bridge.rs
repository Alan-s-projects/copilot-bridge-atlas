//! The desktop bridge owns its database and generated catalog, never Codex files.
use crate::{AppError, AppState, Database, Provider};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn require_codex(app: &str) -> Result<(), AppError> {
    if app != "codex" {
        return Err(AppError::Message("This build supports Codex only".into()));
    }
    Ok(())
}

pub fn require_copilot(provider: &Provider) -> Result<(), AppError> {
    if !provider.is_github_copilot() {
        return Err(AppError::Message(
            "This build supports GitHub Copilot only".into(),
        ));
    }
    Ok(())
}

pub fn providers(db: &Database) -> Result<IndexMap<String, Provider>, AppError> {
    Ok(db
        .get_all_providers("codex")?
        .into_iter()
        .filter(|(_, provider)| provider.is_github_copilot())
        .collect())
}

pub fn current(db: &Database) -> Result<String, AppError> {
    let providers = providers(db)?;
    let selected = crate::settings::get_effective_current_provider(db)?;
    Ok(selected
        .filter(|id| providers.contains_key(id))
        .or_else(|| providers.keys().next().cloned())
        .unwrap_or_default())
}

pub fn select(db: &Database, id: &str) -> Result<(), AppError> {
    let provider = db
        .get_provider_by_id(id, "codex")?
        .ok_or_else(|| AppError::Message("Copilot provider not found".into()))?;
    require_copilot(&provider)?;
    db.set_current_provider("codex", id)?;
    crate::settings::set_current_provider(Some(id))?;
    refresh_catalog(&provider)
}

pub fn save(db: &Database, provider: &Provider) -> Result<(), AppError> {
    require_copilot(provider)?;
    if current(db)? != provider.id || db.get_provider_by_id(&provider.id, "codex")?.is_none() {
        return Err(AppError::Message(
            "Only the existing Copilot entry can be edited".into(),
        ));
    }
    db.save_provider("codex", provider)?;
    if current(db)? == provider.id {
        select(db, &provider.id)?;
    }
    Ok(())
}

pub fn catalog_path() -> std::path::PathBuf {
    crate::config::get_app_config_dir().join("copilot-model-catalog.json")
}

fn refresh_catalog(provider: &Provider) -> Result<(), AppError> {
    let config = provider
        .settings_config
        .get("config")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if let Some(catalog) =
        crate::codex_config::codex_model_catalog_from_settings(&provider.settings_config, config)?
    {
        crate::config::write_json_file(&catalog_path(), &catalog)?;
    } else {
        match std::fs::remove_file(catalog_path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(AppError::io(catalog_path(), error)),
        }
    }
    Ok(())
}

fn ensure_copilot_entry(db: &Database) -> Result<(), AppError> {
    if !providers(db)?.is_empty() {
        return Ok(());
    }
    let mut provider = Provider::with_id(
        uuid::Uuid::new_v4().to_string(),
        "GitHub Copilot".into(),
        serde_json::json!({
            "auth": {},
            "config": "model_provider = \"copilot\"\n[model_providers.copilot]\nbase_url = \"https://api.githubcopilot.com\"\nwire_api = \"responses\"\n",
            "modelCatalog": { "models": [] }
        }),
    );
    provider.meta = Some(crate::ProviderMeta {
        provider_type: Some("github_copilot".into()),
        ..Default::default()
    });
    db.save_provider("codex", &provider)
}

/// Keep legacy DB rows and backups for rollback, but never activate their writers.
pub fn initialize(state: &AppState) -> Result<(), AppError> {
    ensure_copilot_entry(&state.db)?;
    let current = current(&state.db)?;
    if !current.is_empty() {
        select(&state.db, &current)?;
    }
    Ok(())
}

fn merge_live_capabilities(
    provider: &mut Provider,
    models: &[crate::proxy::providers::copilot_auth::CopilotModel],
) -> bool {
    use serde_json::{json, Value};
    let before = provider.settings_config.clone();
    if let Some(rows) = provider
        .settings_config
        .pointer_mut("/modelCatalog/models")
        .and_then(Value::as_array_mut)
    {
        for row in rows.iter_mut() {
            if !row.is_object() {
                continue;
            }
            let Some(model) = row
                .get("model")
                .and_then(Value::as_str)
                .and_then(|id| models.iter().find(|m| m.id.eq_ignore_ascii_case(id)))
            else {
                row["available"] = json!(false);
                continue;
            };
            row["available"] = json!(true);
            row["model"] = json!(model.id);
            row["displayName"] = json!(model.name);
            row["vendor"] = json!(model.vendor);
            row["supportedReasoningLevels"] =
                json!(model.reasoning_efforts.as_deref().unwrap_or(&[]));
            if let Some(limit) = model.max_output_tokens {
                row["maxOutputTokens"] = json!(limit);
            }
            if let Some(tools) = model.supports_tool_calls {
                row["supportsToolCalls"] = json!(tools);
            }
            if let Some(parallel) = model.supports_parallel_tool_calls {
                row["supportsParallelToolCalls"] = json!(parallel);
            }
            if let Some(vision) = model.supports_vision {
                row["inputModalities"] = if vision {
                    json!(["text", "image"])
                } else {
                    json!(["text"])
                };
            }
            row["contextWindow"] = json!(model.context_window);
            if let Some(limit) = model.max_context_window_tokens {
                row["maxContextWindow"] = json!(limit);
            }
            row["reasoningLevels"] = json!(model.reasoning_efforts.as_deref().unwrap_or(&[]));
            if let Some(object) = row.as_object_mut() {
                for key in [
                    "reasoning_levels",
                    "defaultReasoningLevel",
                    "default_reasoning_level",
                    "context_window",
                    "display_name",
                ] {
                    object.remove(key);
                }
            }
        }
        let known: std::collections::HashSet<_> = rows
            .iter()
            .filter_map(|row| row.get("model").and_then(Value::as_str))
            .map(|id| id.trim().to_ascii_lowercase())
            .collect();
        for model in models
            .iter()
            .filter(|model| !known.contains(&model.id.to_ascii_lowercase()))
        {
            rows.push(json!({
                "model": model.id,
                "displayName": model.name,
                "available": true,
                "vendor": model.vendor,
                "contextWindow": model.context_window,
                "maxContextWindow": model.max_context_window_tokens,
                "maxOutputTokens": model.max_output_tokens,
                "supportsToolCalls": model.supports_tool_calls,
                "supportsParallelToolCalls": model.supports_parallel_tool_calls,
                "inputModalities": model.supports_vision.map(|vision| if vision { vec!["text", "image"] } else { vec!["text"] }),
                "reasoningLevels": model.reasoning_efforts.as_deref().unwrap_or(&[]),
                "supportedReasoningLevels": model.reasoning_efforts.as_deref().unwrap_or(&[])
            }));
        }
    }
    provider.settings_config != before
}

/// Refresh only Atlas's saved model metadata and its own generated catalog.
/// A network failure leaves the last saved capabilities usable offline.
pub async fn refresh_capabilities(
    state: &AppState,
    auth: &crate::proxy::providers::copilot_auth::CopilotAuthManager,
) -> Result<(), AppError> {
    if !auth.is_authenticated().await {
        return Ok(());
    }
    let id = current(&state.db)?;
    let Some(provider) = state.db.get_provider_by_id(&id, "codex")? else {
        return Ok(());
    };
    let account = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.managed_account_id_for("github_copilot"));
    let models = match account.as_deref() {
        Some(id) => auth.fetch_models_for_account(id).await,
        None => auth.fetch_models().await,
    }
    .map_err(|error| AppError::Message(error.to_string()))?;
    // Re-read after the network await so a concurrent settings edit is preserved.
    let Some(mut provider) = state.db.get_provider_by_id(&id, "codex")? else {
        return Ok(());
    };
    let active_account = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.managed_account_id_for("github_copilot"));
    if current(&state.db)? == id
        && active_account == account
        && merge_live_capabilities(&mut provider, &models)
    {
        save(&state.db, &provider)?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigDiffLine {
    pub kind: &'static str,
    pub old_line_number: Option<usize>,
    pub new_line_number: Option<usize>,
    pub text: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SetupRecommendations {
    #[serde(rename = "context1m")]
    pub context_1m: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSetupSuggestion {
    pub config_path: String,
    pub config_exists: bool,
    pub configured: bool,
    pub copilot_config: String,
    pub copilot_lines: Vec<ConfigDiffLine>,
    pub openai_config: String,
    pub openai_lines: Vec<ConfigDiffLine>,
}

fn config_uses_endpoint(current: &toml_edit::DocumentMut, endpoint: &str) -> bool {
    let profile = current
        .get("profile")
        .and_then(toml_edit::Item::as_str)
        .and_then(|name| current.get("profiles")?.get(name));
    let provider = profile
        .and_then(|profile| profile.get("model_provider"))
        .or_else(|| current.get("model_provider"))
        .and_then(toml_edit::Item::as_str)
        .unwrap_or("openai");
    let configured_url = current
        .get("model_providers")
        .and_then(|providers| providers.get(provider))
        .and_then(|provider| provider.get("base_url"))
        .and_then(toml_edit::Item::as_str)
        .or_else(|| {
            (provider == "openai")
                .then(|| {
                    profile
                        .and_then(|profile| profile.get("openai_base_url"))
                        .or_else(|| current.get("openai_base_url"))
                        .and_then(toml_edit::Item::as_str)
                })
                .flatten()
        });
    let normalize = |raw: &str| {
        let mut url = url::Url::parse(raw.trim().trim_end_matches('/')).ok()?;
        if matches!(url.host_str(), Some("localhost" | "127.0.0.1")) {
            url.set_host(Some("127.0.0.1")).ok()?;
        }
        Some(url)
    };
    let Some(expected) = normalize(endpoint) else {
        return false;
    };
    configured_url
        .and_then(normalize)
        .is_some_and(|url| url == expected)
}

fn set_connection_value(
    table: &mut dyn toml_edit::TableLike,
    key: &str,
    mut value: toml_edit::Item,
) {
    let existing = table.get(key);
    let unchanged = value
        .as_str()
        .is_some_and(|new| existing.and_then(toml_edit::Item::as_str) == Some(new))
        || value
            .as_bool()
            .is_some_and(|new| existing.and_then(toml_edit::Item::as_bool) == Some(new))
        || value
            .as_integer()
            .is_some_and(|new| existing.and_then(toml_edit::Item::as_integer) == Some(new));
    if unchanged {
        return;
    }
    if let (Some(old), Some(new)) = (
        existing.and_then(toml_edit::Item::as_value),
        value.as_value_mut(),
    ) {
        *new.decor_mut() = old.decor().clone();
    }
    if let Some(existing) = table.get_mut(key) {
        // The key owns leading comments. Replacing its value in place preserves
        // those comments as well as the value decoration copied above.
        *existing = value;
    } else {
        table.insert(key, value);
    }
}

fn provider_table<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    id: &str,
) -> Result<&'a mut dyn toml_edit::TableLike, AppError> {
    if !doc.contains_key("model_providers") {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        doc["model_providers"] = toml_edit::Item::Table(table);
    }
    let providers = doc["model_providers"]
        .as_table_like_mut()
        .ok_or_else(|| AppError::Message("model_providers must be a TOML table".into()))?;
    if !providers.contains_key(id) {
        providers.insert(id, toml_edit::Item::Table(toml_edit::Table::new()));
    }
    providers
        .get_mut(id)
        .and_then(toml_edit::Item::as_table_like_mut)
        .ok_or_else(|| AppError::Message("The selected model provider must be a TOML table".into()))
}

fn config_diff(current: &str, proposed: &str) -> Vec<ConfigDiffLine> {
    // Line-ending differences must not obscure the actual connection changes.
    let current = current.replace("\r\n", "\n");
    let proposed = proposed.replace("\r\n", "\n");
    let diff = similar::TextDiff::from_lines(&current, &proposed);
    diff.iter_all_changes()
        .map(|change| ConfigDiffLine {
            kind: match change.tag() {
                similar::ChangeTag::Equal => "context",
                similar::ChangeTag::Delete => "removed",
                similar::ChangeTag::Insert => "added",
            },
            old_line_number: change.old_index().map(|index| index + 1),
            new_line_number: change.new_index().map(|index| index + 1),
            text: change.value().to_string(),
        })
        .collect()
}

fn setup_suggestion(
    current_text: &str,
    path: &Path,
    config_exists: bool,
    endpoint: &str,
    catalog: Option<&Path>,
    _recommendations: Option<&SetupRecommendations>,
) -> Result<CodexSetupSuggestion, AppError> {
    let current = current_text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| {
            AppError::Message(
                "Codex config.toml is invalid. Fix it in your editor; no files were changed."
                    .into(),
            )
        })?;
    let active = current
        .get("model_provider")
        .and_then(toml_edit::Item::as_str)
        .unwrap_or("openai");
    let previous = current
        .get("model_providers")
        .and_then(|items| items.get(active));
    let previous_url = previous
        .and_then(|item| item.get("base_url"))
        .and_then(toml_edit::Item::as_str)
        .or_else(|| {
            (active == "openai")
                .then(|| {
                    current
                        .get("openai_base_url")
                        .and_then(toml_edit::Item::as_str)
                })
                .flatten()
        });
    // Reusing the active custom id avoids changing Codex's provider identity when migrating a proxy.
    let id = if !active.is_empty()
        && !matches!(active, "openai" | "ollama" | "lmstudio" | "amazon-bedrock")
    {
        active.to_string()
    } else {
        current
            .get("model_providers")
            .and_then(toml_edit::Item::as_table_like)
            .and_then(|items| {
                items.iter().find_map(|(id, item)| {
                    if matches!(id, "openai" | "ollama" | "lmstudio" | "amazon-bedrock") {
                        return None;
                    }
                    item.get("base_url")
                        .and_then(toml_edit::Item::as_str)
                        .filter(|url| url.trim_end_matches('/') == endpoint.trim_end_matches('/'))
                        .map(|_| id.to_string())
                })
            })
            .unwrap_or_else(|| "copilot-bridge-atlas".to_string())
    };
    let configured = active == id
        && previous_url
            .is_some_and(|url| url.trim_end_matches('/') == endpoint.trim_end_matches('/'));
    let mut copilot = current.clone();
    set_connection_value(
        copilot.as_table_mut(),
        "model_provider",
        toml_edit::value(&id),
    );
    if let Some(catalog) = catalog {
        set_connection_value(
            copilot.as_table_mut(),
            "model_catalog_json",
            toml_edit::value(catalog.to_string_lossy().as_ref()),
        );
    } else {
        copilot.remove("model_catalog_json");
    }
    for key in ["openai_base_url", "chatgpt_base_url", "forced_login_method"] {
        copilot.remove(key);
    }
    if let Some(providers) = copilot
        .get_mut("model_providers")
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        providers.remove("openai");
    }
    let target = provider_table(&mut copilot, &id)?;
    // Codex forbids command-backed auth together with env_key, a direct token, or OpenAI auth.
    // https://developers.openai.com/codex/config-reference/
    for key in ["auth", "env_key", "env_key_instructions"] {
        target.remove(key);
    }
    if !configured {
        for key in ["http_headers", "env_http_headers", "query_params"] {
            target.remove(key);
        }
    }
    for (key, value) in [
        ("name", toml_edit::value("GitHub Copilot")),
        ("base_url", toml_edit::value(endpoint)),
        ("wire_api", toml_edit::value("responses")),
        ("requires_openai_auth", toml_edit::value(false)),
        (
            "experimental_bearer_token",
            toml_edit::value("PROXY_MANAGED"),
        ),
        ("supports_websockets", toml_edit::value(false)),
    ] {
        set_connection_value(target, key, value);
    }
    if target.contains_key("supports_standalone_web_search") {
        set_connection_value(
            target,
            "supports_standalone_web_search",
            toml_edit::value(false),
        );
    }
    let mut openai = current.clone();
    set_connection_value(
        openai.as_table_mut(),
        "model_provider",
        toml_edit::value("openai"),
    );
    set_connection_value(
        openai.as_table_mut(),
        "forced_login_method",
        toml_edit::value("chatgpt"),
    );
    for key in ["model_catalog_json", "openai_base_url", "chatgpt_base_url"] {
        openai.remove(key);
    }
    if let Some(providers) = openai
        .get_mut("model_providers")
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        providers.remove("openai");
        if providers.is_empty() {
            openai.remove("model_providers");
        }
    }
    // Per-model context windows are defined in the catalog (model_catalog_json).
    // Remove top-level model_context_window in both proposals so Codex relies on catalog definitions.
    copilot.remove("model_context_window");
    openai.remove("model_context_window");
    let copilot_config = copilot.to_string();
    let openai_config = openai.to_string();
    let copilot_lines = config_diff(current_text, &copilot_config);
    let openai_lines = config_diff(current_text, &openai_config);
    Ok(CodexSetupSuggestion {
        config_path: path.to_string_lossy().into_owned(),
        config_exists,
        configured: config_uses_endpoint(&current, endpoint),
        copilot_lines,
        openai_lines,
        copilot_config,
        openai_config,
    })
}

fn normalize_preview_config_path(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    let raw = raw
        .strip_prefix('"')
        .and_then(|path| path.strip_suffix('"'))
        .or_else(|| {
            raw.strip_prefix('\'')
                .and_then(|path| path.strip_suffix('\''))
        })
        .unwrap_or(raw)
        .trim();
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err("Enter the full absolute path to a TOML file.".into());
    }
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
    {
        return Err("Choose a TOML file (.toml).".into());
    }
    Ok(path)
}

fn setup_suggestion_from_file(
    path: &Path,
    explicit: bool,
    endpoint: &str,
    catalog: Option<&Path>,
    recommendations: Option<&SetupRecommendations>,
) -> Result<CodexSetupSuggestion, String> {
    let read_error =
        |error: std::io::Error| format!("Cannot read TOML file {}: {error}", path.display());
    // Read and inspect the same open file. Existence describes this snapshot,
    // including an existing empty file, rather than a later filesystem check.
    let (text, exists) = match std::fs::File::open(path) {
        Ok(mut file) => {
            if !file.metadata().map_err(&read_error)?.is_file() {
                return Err(format!(
                    "Configuration path is not a regular TOML file: {}",
                    path.display()
                ));
            }
            let mut text = String::new();
            file.read_to_string(&mut text).map_err(&read_error)?;
            (text, true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !explicit => {
            (String::new(), false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "Selected TOML file does not exist: {}",
                path.display()
            ));
        }
        Err(error) => return Err(read_error(error)),
    };
    setup_suggestion(&text, path, exists, endpoint, catalog, recommendations)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_codex_setup_suggestion(
    state: tauri::State<'_, AppState>,
    config_path: Option<String>,
    recommendations: Option<SetupRecommendations>,
) -> Result<CodexSetupSuggestion, String> {
    let explicit = config_path.is_some();
    let path = match config_path {
        Some(path) => normalize_preview_config_path(&path)?,
        None => crate::codex_config::get_codex_config_path(),
    };
    let config = state
        .db
        .get_global_proxy_config()
        .await
        .map_err(|e| e.to_string())?;
    let address = match config.listen_address.as_str() {
        "0.0.0.0" | "::" => "127.0.0.1",
        address => address,
    };
    let host = if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]")
    } else {
        address.to_string()
    };
    let endpoint = format!("http://{host}:{}/v1", config.listen_port);
    let catalog = catalog_path();
    // A user-selected UNC path can be slow; keep filesystem reads and diff
    // generation off the asynchronous command worker.
    tauri::async_runtime::spawn_blocking(move || {
        setup_suggestion_from_file(
            &path,
            explicit,
            &endpoint,
            catalog.exists().then_some(catalog.as_path()),
            recommendations.as_ref(),
        )
    })
    .await
    .map_err(|error| format!("Cannot prepare Codex configuration preview: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestHome {
        previous: Option<std::ffi::OsString>,
        _dir: tempfile::TempDir,
    }

    impl TestHome {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let previous = std::env::var_os("COPILOT_BRIDGE_ATLAS_TEST_HOME");
            std::env::set_var("COPILOT_BRIDGE_ATLAS_TEST_HOME", directory.path());
            let fixture = Self {
                previous,
                _dir: directory,
            };
            let app_dir = fixture._dir.path().join(".copilot-bridge-atlas");
            std::fs::create_dir_all(&app_dir).unwrap();
            // Prevent the legacy Windows HOME fallback from escaping the fixture.
            std::fs::write(app_dir.join("copilot-bridge-atlas.db"), []).unwrap();
            assert_eq!(crate::config::get_app_config_dir(), app_dir);
            crate::settings::reload_settings().unwrap();
            fixture
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var("COPILOT_BRIDGE_ATLAS_TEST_HOME", previous);
            } else {
                std::env::remove_var("COPILOT_BRIDGE_ATLAS_TEST_HOME");
            }
            let _ = crate::settings::reload_settings();
        }
    }

    fn assert_diff_snapshots(lines: &[ConfigDiffLine], current: &str, proposed: &str) {
        for line in lines {
            assert!(matches!(
                (line.kind, line.old_line_number, line.new_line_number),
                ("context", Some(_), Some(_))
                    | ("removed", Some(_), None)
                    | ("added", None, Some(_))
            ));
        }
        for (source, old_side) in [(current, true), (proposed, false)] {
            let normalized = source.replace("\r\n", "\n");
            let expected = normalized
                .split_inclusive('\n')
                .enumerate()
                .map(|(index, text)| (index + 1, text))
                .collect::<Vec<_>>();
            let actual = lines
                .iter()
                .filter_map(|line| {
                    let number = if old_side {
                        line.old_line_number
                    } else {
                        line.new_line_number
                    };
                    number.map(|number| (number, line.text.as_str()))
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn connection_status_checks_the_selected_profile_and_local_endpoint() {
        for (text, expected) in [
            ("", false),
            ("model_provider = 'openai'\n", false),
            ("model_provider = 'custom'\n[model_providers.custom]\nbase_url = 'http://localhost:15722/v1/'\n", true),
            ("model_provider = 'custom'\n[model_providers.custom]\nbase_url = 'http://127.0.0.1:4142/v1'\n", false),
            ("model_provider = 'custom'\n[model_providers.custom]\nbase_url = 'http://127.0.0.1:15722/other'\n", false),
            ("model_provider = 'custom'\nprofile = 'official'\n[model_providers.custom]\nbase_url = 'http://127.0.0.1:15722/v1'\n[profiles.official]\nmodel_provider = 'openai'\n", false),
            ("model_provider = 'openai'\nprofile = 'work'\n[model_providers.atlas]\nbase_url = 'http://127.0.0.1:15722/v1'\n[profiles.work]\nmodel_provider = 'atlas'\n", true),
            ("openai_base_url = 'http://127.0.0.1:15722/v1'\n", true),
        ] {
            let current = text.parse::<toml_edit::DocumentMut>().unwrap();
            assert_eq!(config_uses_endpoint(&current, "http://127.0.0.1:15722/v1"), expected, "{text}");
        }
    }

    #[test]
    fn context_preset_preserves_other_preferences_in_both_proposals() {
        use serde_json::json;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let text = r#"# Keep all other preferences
model_provider = "bridge"
model = "gpt-6-astra"
model_context_window = 1048576
model_auto_compact_token_limit = 234_567 # Keep my compaction threshold
model_reasoning_effort = "high"
approval_policy = "never"
sandbox_mode = "workspace-write"
notify = ["pwsh", "notify.ps1"]
disable_response_storage = true
[profiles.keep]
model = "profile-model"
model_context_window = 64000
model_auto_compact_token_limit = 32000
model_reasoning_effort = "low"
approval_policy = "never"
sandbox_mode = "read-only"
notify = ["profile-hook"]
disable_response_storage = false
[custom]
model_context_window = 128000
model_auto_compact_token_limit = 64000
model_reasoning_effort = "medium"
"#;
        std::fs::write(&path, text).unwrap();
        let original = text.parse::<toml_edit::DocumentMut>().unwrap();
        let baseline =
            setup_suggestion_from_file(&path, true, "http://127.0.0.1:15722/v1", None, None)
                .unwrap();
        for flags in [
            json!({}),
            json!({"context1m": false}),
            json!({"context1m": true}),
        ] {
            let recommendations: SetupRecommendations = serde_json::from_value(flags).unwrap();
            let preview = setup_suggestion_from_file(
                &path,
                true,
                "http://127.0.0.1:15722/v1",
                None,
                Some(&recommendations),
            )
            .unwrap();
            if !recommendations.context_1m {
                assert_eq!(
                    serde_json::to_value(&preview).unwrap(),
                    serde_json::to_value(&baseline).unwrap()
                );
            }
            assert!(preview.config_exists);
            for (config, lines) in [
                (&preview.copilot_config, &preview.copilot_lines),
                (&preview.openai_config, &preview.openai_lines),
            ] {
                assert_diff_snapshots(lines, text, config);
                let proposed = config.parse::<toml_edit::DocumentMut>().unwrap();
                assert!(proposed.get("model_context_window").is_none());
                for key in [
                    "model_auto_compact_token_limit",
                    "model_reasoning_effort",
                    "approval_policy",
                    "sandbox_mode",
                ] {
                    assert_eq!(
                        proposed[key].to_string(),
                        original[key].to_string(),
                        "{key}"
                    );
                }
                for key in [
                    "model",
                    "notify",
                    "disable_response_storage",
                    "profiles",
                    "custom",
                ] {
                    assert_eq!(
                        proposed[key].to_string(),
                        original[key].to_string(),
                        "{key}"
                    );
                }
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
    }

    #[test]
    fn proposal_removes_top_level_context_window_and_preserves_granular_policy() {
        let text = r#"model_context_window = 1_000_000 # Keep numeric formatting
model_auto_compact_token_limit = 900_000
approval_policy = { granular = { sandbox_approval = false, rules = true } }
notify = ["unchanged"]
"#;
        let recommendations = SetupRecommendations { context_1m: true };
        let preview = setup_suggestion(
            text,
            Path::new("config.toml"),
            true,
            "http://127.0.0.1:15722/v1",
            None,
            Some(&recommendations),
        )
        .unwrap();
        for config in [&preview.copilot_config, &preview.openai_config] {
            assert!(!config.contains("model_context_window"));
            assert!(config.contains("model_auto_compact_token_limit = 900_000"));
            let proposed = config.parse::<toml_edit::DocumentMut>().unwrap();
            assert!(proposed.get("model_context_window").is_none());
            assert_eq!(
                proposed["approval_policy"].to_string(),
                text.parse::<toml_edit::DocumentMut>().unwrap()["approval_policy"].to_string()
            );
            assert_eq!(proposed["notify"].to_string().trim(), "[\"unchanged\"]");
        }
    }

    #[test]
    fn selected_files_have_independent_snapshots_without_changing_other_files() {
        let directory = tempfile::tempdir().unwrap();
        let files = [
            ("config.toml", "model = 'default-model'\n"),
            ("settings.json", r#"{"codexConfigDir":"unchanged"}"#),
            ("auth.json", r#"{"OPENAI_API_KEY":"unchanged"}"#),
            (
                "first.toml",
                "# First file\nmodel_reasoning_effort = 'low'\n",
            ),
            (
                "second file.TOML",
                "# Second file\r\nmodel_reasoning_effort = 'high'\r\n",
            ),
        ];
        for (name, contents) in files {
            std::fs::write(directory.path().join(name), contents).unwrap();
        }
        for ((name, text), effort, quote) in [(files[3], "low", '"'), (files[4], "high", '\'')] {
            let path = directory.path().join(name);
            let selected =
                normalize_preview_config_path(&format!("  {quote}{}{quote}  ", path.display()))
                    .unwrap();
            assert_eq!(selected, path);
            let preview = setup_suggestion_from_file(
                &selected,
                true,
                "http://127.0.0.1:15722/v1",
                None,
                None,
            )
            .unwrap();
            assert!(preview.config_exists);
            assert_eq!(preview.config_path, selected.to_string_lossy());
            assert_eq!(
                serde_json::to_value(&preview).unwrap()["configExists"],
                true
            );
            for (config, lines) in [
                (&preview.copilot_config, &preview.copilot_lines),
                (&preview.openai_config, &preview.openai_lines),
            ] {
                assert_diff_snapshots(lines, text, config);
                let proposed = config.parse::<toml_edit::DocumentMut>().unwrap();
                assert_eq!(proposed["model_reasoning_effort"].as_str(), Some(effort));
            }
        }
        for (name, contents) in files {
            assert_eq!(
                std::fs::read_to_string(directory.path().join(name)).unwrap(),
                contents
            );
        }
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            files.len()
        );
    }

    #[test]
    fn missing_auto_file_is_distinct_from_missing_selection_and_existing_empty_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing-parent").join("config.toml");
        let preview =
            setup_suggestion_from_file(&path, false, "http://127.0.0.1:15722/v1", None, None)
                .unwrap();
        assert!(!preview.config_exists);
        assert_eq!(
            serde_json::to_value(&preview).unwrap()["configExists"],
            false
        );
        assert_diff_snapshots(&preview.copilot_lines, "", &preview.copilot_config);
        assert!(
            setup_suggestion_from_file(&path, true, "http://127.0.0.1:15722/v1", None, None)
                .unwrap_err()
                .contains("does not exist")
        );
        assert!(!path.parent().unwrap().exists());

        let empty = directory.path().join("empty.toml");
        std::fs::write(&empty, "").unwrap();
        for explicit in [true, false] {
            let preview = setup_suggestion_from_file(
                &empty,
                explicit,
                "http://127.0.0.1:15722/v1",
                None,
                None,
            )
            .unwrap();
            assert!(preview.config_exists);
            assert_diff_snapshots(&preview.copilot_lines, "", &preview.copilot_config);
            assert_eq!(std::fs::read_to_string(&empty).unwrap(), "");
        }
    }

    #[test]
    fn invalid_preview_paths_and_files_fail_without_rewriting_them() {
        for path in [
            "",
            "\"\"",
            "relative/config.toml",
            "~/config.toml",
            "C:config.toml",
        ] {
            assert!(normalize_preview_config_path(path).is_err(), "{path}");
        }
        let directory = tempfile::tempdir().unwrap();
        let wrong_extension = directory.path().join("settings.json");
        assert!(normalize_preview_config_path(&wrong_extension.to_string_lossy()).is_err());
        assert!(!wrong_extension.exists());

        let folder = directory.path().join("folder.toml");
        std::fs::create_dir(&folder).unwrap();
        assert!(
            setup_suggestion_from_file(&folder, true, "http://localhost/v1", None, None).is_err()
        );

        let broken = directory.path().join("broken.toml");
        std::fs::write(&broken, "broken = [").unwrap();
        assert!(
            setup_suggestion_from_file(&broken, true, "http://localhost/v1", None, None)
                .unwrap_err()
                .contains("invalid")
        );
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), "broken = [");

        let unreadable = directory.path().join("not-utf8.toml");
        std::fs::write(&unreadable, [0xff_u8]).unwrap();
        assert!(
            setup_suggestion_from_file(&unreadable, true, "http://localhost/v1", None, None)
                .unwrap_err()
                .contains("Cannot read TOML file")
        );
        assert_eq!(std::fs::read(&unreadable).unwrap(), [0xff_u8]);
    }

    #[test]
    fn config_diff_lines_serialize_numbered_changes_and_preserve_missing_newlines() {
        let lines = config_diff("same\r\nold", "same\nnew\n");
        assert_eq!(
            serde_json::to_value(lines).unwrap(),
            serde_json::json!([
                { "kind": "context", "oldLineNumber": 1, "newLineNumber": 1, "text": "same\n" },
                { "kind": "removed", "oldLineNumber": 2, "newLineNumber": null, "text": "old" },
                { "kind": "added", "oldLineNumber": null, "newLineNumber": 2, "text": "new\n" }
            ])
        );
    }

    #[test]
    fn config_diff_lines_reconstruct_both_complete_files() {
        for (current, proposed) in [
            ("", ""),
            ("", "added\n"),
            ("removed\n", ""),
            ("\n", "\n"),
            ("same\r\nsame\r\n", "same\nsame\n"),
            ("same", "same\n"),
            ("same\n", "same"),
            (
                "start\r\nold\r\nsame\r\ndeleted\r\nsame\r\nend",
                "start\nnew\nsame\nsame\ninserted\nend",
            ),
        ] {
            let lines = config_diff(current, proposed);
            assert_diff_snapshots(&lines, current, proposed);
        }

        // Keep every unchanged line in the comparison.
        let current = format!("before\n{}old\n", "same\n".repeat(10));
        let proposed = format!("before\n{}new\n", "same\n".repeat(10));
        let lines = config_diff(&current, &proposed);
        assert_diff_snapshots(&lines, &current, &proposed);
        assert_eq!(
            lines.iter().filter(|line| line.kind == "context").count(),
            11
        );
    }

    #[test]
    fn seeds_one_copilot_entry_and_preserves_existing_data() {
        let db = Database::memory().unwrap();
        let old = Provider::with_id(
            "legacy".into(),
            "Old provider".into(),
            serde_json::json!({}),
        );
        db.save_provider("codex", &old).unwrap();
        ensure_copilot_entry(&db).unwrap();
        let first = providers(&db).unwrap();
        assert_eq!(first.len(), 1);
        let provider = first.values().next().unwrap();
        assert_eq!(provider.name, "GitHub Copilot");
        assert_eq!(
            provider.settings_config["modelCatalog"]["models"],
            serde_json::json!([])
        );
        ensure_copilot_entry(&db).unwrap();
        assert_eq!(
            providers(&db).unwrap().keys().collect::<Vec<_>>(),
            first.keys().collect::<Vec<_>>()
        );
        assert!(db.get_provider_by_id("legacy", "codex").unwrap().is_some());
    }

    #[test]
    fn refresh_uses_live_metadata_without_replacing_enabled_choices() {
        use crate::proxy::providers::copilot_auth::CopilotModel;
        use serde_json::json;
        let mut provider = Provider::with_id(
            "copilot".into(),
            "Copilot".into(),
            json!({
                "config": "keep the stored template",
                "modelCatalog": {"models": [
                    {"model": "gpt-6-astra", "supportsParallelToolCalls": false, "inputModalities": ["text", "image"], "contextWindow": 1048576},
                    {"model": "gpt-6-luna", "inputModalities": ["text"], "contextWindow": 1000000, "reasoningLevels": ["low", "high", "ultra"], "defaultReasoningLevel": "ultra"},
                    {"model": "custom-alias", "inputModalities": ["text"]}
                ]}
            }),
        );
        let models = [
            CopilotModel {
                id: "gpt-6-astra".into(),
                context_window: Some(1050000),
                supports_parallel_tool_calls: Some(true),
                supports_vision: Some(true),
                ..Default::default()
            },
            CopilotModel {
                id: "gpt-6-luna".into(),
                context_window: Some(872000),
                supports_parallel_tool_calls: Some(true),
                supports_vision: Some(true),
                reasoning_efforts: Some(vec!["low".into(), "medium".into(), "max".into()]),
                ..Default::default()
            },
        ];
        assert!(merge_live_capabilities(&mut provider, &models));
        let rows = &provider.settings_config["modelCatalog"]["models"];
        assert_eq!(rows.as_array().unwrap().len(), 3);
        assert_eq!(rows[0]["supportsParallelToolCalls"], true);
        assert_eq!(rows[0]["contextWindow"], 1050000);
        assert_eq!(rows[1]["contextWindow"], 872000);
        assert_eq!(rows[1]["inputModalities"], json!(["text", "image"]));
        assert_eq!(rows[1]["reasoningLevels"], json!(["low", "medium", "max"]));
        assert!(rows[1].get("defaultReasoningLevel").is_none());
        assert_eq!(
            rows[2],
            json!({"model": "custom-alias", "inputModalities": ["text"], "available": false})
        );
        assert_eq!(
            provider.settings_config["config"],
            "keep the stored template"
        );
        assert!(!merge_live_capabilities(&mut provider, &models));
    }

    #[test]
    fn refresh_discovers_future_models_and_preserves_unavailable_and_disabled_rows() {
        use crate::proxy::providers::copilot_auth::CopilotModel;
        use serde_json::json;
        let mut provider = Provider::with_id(
            "copilot".into(),
            "Copilot".into(),
            json!({
                "modelCatalog": {"models": [
                    {"model":"grok-old", "enabled":false, "reasoningLevels":["high"]},
                    null,
                    {"model":"gemini-returning", "available":false}
                ]}
            }),
        );
        let models = [
            CopilotModel {
                id: "new-vendor/agent".into(),
                name: "Future Agent".into(),
                context_window: Some(16000),
                max_output_tokens: Some(4096),
                reasoning_efforts: Some(vec!["low".into()]),
                ..Default::default()
            },
            CopilotModel {
                id: "gemini-returning".into(),
                ..Default::default()
            },
        ];
        assert!(merge_live_capabilities(&mut provider, &models));
        let rows = &provider.settings_config["modelCatalog"]["models"];
        assert_eq!(rows.as_array().unwrap().len(), 4);
        assert_eq!(rows[0]["enabled"], false);
        assert_eq!(rows[0]["available"], false);
        assert_eq!(rows[0]["reasoningLevels"], json!(["high"]));
        assert!(rows[1].is_null());
        assert_eq!(rows[2]["available"], true);
        assert_eq!(rows[3]["model"], "new-vendor/agent");
        assert_ne!(rows[3]["enabled"], false);
        assert_eq!(rows[3]["maxOutputTokens"], 4096);
        assert!(!merge_live_capabilities(&mut provider, &models));
    }

    #[test]
    fn refresh_replaces_legacy_reasoning_overrides_with_copilot_declarations() {
        use crate::proxy::providers::copilot_auth::CopilotModel;
        use serde_json::json;
        let models = [CopilotModel {
            id: "gpt-6-astra".into(),
            reasoning_efforts: Some(vec!["low".into(), "medium".into()]),
            ..Default::default()
        }];
        for (canonical_levels, canonical_default) in [
            (None, None),
            (Some(json!([])), Some(json!(""))),
            (Some(json!("invalid")), Some(json!(false))),
            (Some(json!([null, 4, " "])), Some(json!(null))),
            (Some(json!(["high", "max"])), Some(json!("max"))),
        ] {
            let mut row = json!({
                "model": "gpt-6-astra",
                "reasoning_levels": ["low", "ultra"],
                "default_reasoning_level": "ultra"
            });
            if let Some(levels) = canonical_levels {
                row["reasoningLevels"] = levels;
            }
            if let Some(default) = canonical_default {
                row["defaultReasoningLevel"] = default;
            }
            let mut provider = Provider::with_id(
                "copilot".into(),
                "Copilot".into(),
                json!({"modelCatalog": {"models": [row]}}),
            );
            merge_live_capabilities(&mut provider, &models);
            let saved = &provider.settings_config["modelCatalog"]["models"][0];
            assert_eq!(saved["reasoningLevels"], json!(["low", "medium"]));
            assert!(saved.get("defaultReasoningLevel").is_none());
            assert!(saved.get("reasoning_levels").is_none());
            assert!(!merge_live_capabilities(&mut provider, &models));
        }
    }

    #[test]
    fn refresh_initializes_unset_reasoning_without_forcing_a_default() {
        use crate::proxy::providers::copilot_auth::CopilotModel;
        use serde_json::json;
        let mut provider = Provider::with_id(
            "copilot".into(),
            "Copilot".into(),
            json!({"modelCatalog": {"models": [
                {"model": "gpt-6-astra"},
                {"model": "gpt-6-luna", "reasoningLevels": []}
            ]}}),
        );
        let models = ["gpt-6-astra", "gpt-6-luna"].map(|id| CopilotModel {
            id: id.into(),
            reasoning_efforts: Some(vec!["low".into(), "high".into()]),
            ..Default::default()
        });
        assert!(merge_live_capabilities(&mut provider, &models));
        for row in provider.settings_config["modelCatalog"]["models"]
            .as_array()
            .unwrap()
        {
            assert_eq!(row["reasoningLevels"], json!(["low", "high"]));
            assert!(row.get("defaultReasoningLevel").is_none());
        }
        assert!(!merge_live_capabilities(&mut provider, &models));
    }

    #[test]
    #[serial_test::serial]
    fn live_reasoning_replaces_overrides_across_database_reopen_and_startup_refresh() {
        use crate::proxy::providers::copilot_auth::CopilotModel;
        use serde_json::{json, Value};
        use std::sync::Arc;

        let fixture = TestHome::new();
        let client_dir = fixture._dir.path().join(".codex");
        std::fs::create_dir_all(client_dir.join("skills")).unwrap();
        let files = [
            (
                "config.toml",
                "# unchanged\nmodel_reasoning_effort = 'medium'\n",
            ),
            ("auth.json", r#"{"OPENAI_API_KEY":"keep-local-auth"}"#),
            ("AGENTS.md", "Keep my instructions"),
            ("skills/SKILL.md", "Keep my skill"),
        ];
        for (name, contents) in files {
            std::fs::write(client_dir.join(name), contents).unwrap();
        }
        let provider_id = {
            let db = Arc::new(Database::init().unwrap());
            let state = AppState::new(db.clone());
            initialize(&state).unwrap();
            let id = current(&db).unwrap();
            let mut provider = db.get_provider_by_id(&id, "codex").unwrap().unwrap();
            provider.settings_config["modelCatalog"] = json!({"models": [{
                "model": "GPT-6-LUNA",
                "contextWindow": 1000000,
                "supportsParallelToolCalls": false,
                "inputModalities": ["text"],
                "reasoningLevels": ["low", "high", "ultra"],
                "defaultReasoningLevel": "ultra"
            }]});
            save(&db, &provider).unwrap();
            id
        };
        let models = [CopilotModel {
            id: "gpt-6-luna".into(),
            context_window: Some(872000),
            supports_parallel_tool_calls: Some(true),
            supports_vision: Some(true),
            reasoning_efforts: Some(vec!["low".into(), "medium".into(), "high".into()]),
            ..Default::default()
        }];

        // Reopen the actual database and run startup initialization twice: once
        // after the user's save and once after the live capabilities refresh.
        for restart in 0..2 {
            crate::settings::reload_settings().unwrap();
            let db = Arc::new(Database::init().unwrap());
            let state = AppState::new(db.clone());
            initialize(&state).unwrap();
            let mut provider = db
                .get_provider_by_id(&provider_id, "codex")
                .unwrap()
                .unwrap();
            let changed = merge_live_capabilities(&mut provider, &models);
            assert_eq!(changed, restart == 0);
            if changed {
                save(&db, &provider).unwrap();
            }
            let saved = db
                .get_provider_by_id(&provider_id, "codex")
                .unwrap()
                .unwrap();
            let row = &saved.settings_config["modelCatalog"]["models"][0];
            assert_eq!(row["reasoningLevels"], json!(["low", "medium", "high"]));
            assert!(row.get("defaultReasoningLevel").is_none());
            assert_eq!(row["contextWindow"], 872000);
            assert_eq!(row["supportsParallelToolCalls"], true);
            assert_eq!(row["inputModalities"], json!(["text", "image"]));

            assert!(catalog_path().starts_with(fixture._dir.path().join(".copilot-bridge-atlas")));
            let catalog: Value =
                serde_json::from_str(&std::fs::read_to_string(catalog_path()).unwrap()).unwrap();
            let entry = &catalog["models"][0];
            let levels = entry["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|level| level["effort"].as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(levels, ["low", "medium", "high"]);
            assert_eq!(entry["default_reasoning_level"], "medium");
        }
        for (name, contents) in files {
            assert_eq!(
                std::fs::read_to_string(client_dir.join(name)).unwrap(),
                contents,
                "{name}"
            );
        }
        assert!(!client_dir
            .join("copilot-bridge-atlas-model-catalog.json")
            .exists());
    }

    #[test]
    fn migrates_an_old_proxy_and_restores_openai_without_touching_preferences_or_files() {
        let text = r#"# Keep my preferences
model_provider = "bridge"
model = "gpt-6-astra"
model_reasoning_effort = "high"
model_auto_compact_token_limit = 900000
model_catalog_json = "old-models.json"
openai_base_url = "http://old-openai-proxy/v1"
[model_providers.bridge]
name = "Copilot Bridge"
base_url = "http://localhost:4142/v1"
wire_api = "chat"
env_key = "OLD_KEY"
http_headers = { Authorization = "old-token" }
env_http_headers = { Authorization = "OLD_KEY" }
query_params = { key = "old-key" }
auth = { command = "old-auth-helper" }
[mcp_servers.keep]
command = "unchanged"
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let catalog = dir.path().join("atlas-models.json");
        std::fs::write(&path, text).unwrap();
        let preview = setup_suggestion_from_file(
            &path,
            true,
            "http://127.0.0.1:15722/v1",
            Some(&catalog),
            None,
        )
        .unwrap();
        assert!(preview.config_exists);
        let proposed = preview
            .copilot_config
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        assert_eq!(proposed["model_provider"].as_str(), Some("bridge"));
        assert_eq!(
            proposed["model_providers"]["bridge"]["base_url"].as_str(),
            Some("http://127.0.0.1:15722/v1")
        );
        for key in [
            "auth",
            "env_key",
            "http_headers",
            "env_http_headers",
            "query_params",
        ] {
            assert!(
                proposed["model_providers"]["bridge"].get(key).is_none(),
                "{key}"
            );
        }
        for (config, lines) in [
            (&preview.copilot_config, &preview.copilot_lines),
            (&preview.openai_config, &preview.openai_lines),
        ] {
            assert_diff_snapshots(lines, text, config);
            let doc = config.parse::<toml_edit::DocumentMut>().unwrap();
            assert_eq!(doc["model_reasoning_effort"].as_str(), Some("high"));
            assert_eq!(
                doc["model_auto_compact_token_limit"].as_integer(),
                Some(900000)
            );
            assert_eq!(
                doc["mcp_servers"]["keep"]["command"].as_str(),
                Some("unchanged")
            );
            assert!(config.contains("# Keep my preferences"));
        }
        let official = preview
            .openai_config
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        assert_eq!(official["model_provider"].as_str(), Some("openai"));
        assert_eq!(official["forced_login_method"].as_str(), Some("chatgpt"));
        assert!(!official.contains_key("model_catalog_json"));
        assert!(!official.contains_key("openai_base_url"));
        assert!(preview.copilot_lines.iter().any(|line| {
            line.kind == "removed"
                && line
                    .text
                    .contains("base_url = \"http://localhost:4142/v1\"")
        }));
        let second = setup_suggestion(
            &preview.copilot_config,
            &path,
            true,
            "http://127.0.0.1:15722/v1",
            Some(&catalog),
            None,
        )
        .unwrap();
        assert_diff_snapshots(
            &second.copilot_lines,
            &preview.copilot_config,
            &second.copilot_config,
        );
        assert!(second
            .copilot_lines
            .iter()
            .all(|line| line.kind == "context"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    }

    #[test]
    fn handles_inline_tables_missing_providers_and_reserved_ids() {
        for text in [
            "model_provider = 'custom'\nmodel_providers = { custom = { base_url = 'http://old/v1', env_key = 'OLD' } }\n",
            "model_provider = 'new'\nmodel_providers = { old = { base_url = 'http://old/v1' } }\n",
            "model_provider = 'missing'\n",
            "model_provider = 'openai'\n[model_providers.openai]\nbase_url = 'http://127.0.0.1:15722/v1'\n",
        ] {
            let preview = setup_suggestion(text, Path::new("config.toml"), false, "http://127.0.0.1:15722/v1", None, None).unwrap();
            let doc = preview.copilot_config.parse::<toml_edit::DocumentMut>().unwrap();
            let id = doc["model_provider"].as_str().unwrap();
            assert_ne!(id, "openai");
            assert_eq!(doc["model_providers"][id]["wire_api"].as_str(), Some("responses"));
            assert_eq!(doc["model_providers"][id]["requires_openai_auth"].as_bool(), Some(false));
            assert!(doc["model_providers"].get("openai").is_none());
        }
    }

    #[test]
    fn proposal_preserves_preferences_and_replaces_the_previous_proxy_token() {
        let text = r#"# user comment
model_provider = "my-copilot"
model = "gpt-6-astra"
model_auto_compact_token_limit = 900000
[model_providers.my-copilot]
base_url = "http://127.0.0.1:15722/v1"
experimental_bearer_token = "private-token"
[mcp_servers.example]
command = "keep-me"
"#;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        let result =
            setup_suggestion_from_file(&path, true, "http://127.0.0.1:15722/v1", None, None)
                .unwrap();
        assert!(result.configured);
        let parsed = result
            .copilot_config
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        assert_eq!(parsed["model_provider"].as_str(), Some("my-copilot"));
        assert_eq!(
            parsed["model_providers"]["my-copilot"]["experimental_bearer_token"].as_str(),
            Some("PROXY_MANAGED")
        );
        assert!(!result.copilot_config.contains("private-token"));
        assert_eq!(
            parsed["mcp_servers"]["example"]["command"].as_str(),
            Some("keep-me")
        );
        assert_eq!(
            parsed["model_auto_compact_token_limit"].as_integer(),
            Some(900000)
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    }

    #[test]
    fn suggestion_handles_fresh_and_official_configs_without_creating_a_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        for text in ["", "model_provider = \"openai\"\n"] {
            let result =
                setup_suggestion(text, &path, false, "http://127.0.0.1:15722/v1", None, None)
                    .unwrap();
            assert_diff_snapshots(&result.copilot_lines, text, &result.copilot_config);
            assert_diff_snapshots(&result.openai_lines, text, &result.openai_config);
            assert!(!result.configured);
            assert!(result
                .copilot_config
                .contains("model_provider = \"copilot-bridge-atlas\""));
            assert!(!path.exists());
        }
        assert!(setup_suggestion(
            "broken = [",
            &path,
            false,
            "http://localhost/v1",
            None,
            None
        )
        .is_err());
        assert!(!path.exists());
        let official = "model_provider = \"openai\"\n[model_providers.copilot]\nbase_url = \"http://127.0.0.1:15722/v1\"\n";
        assert!(
            !setup_suggestion(
                official,
                &path,
                false,
                "http://127.0.0.1:15722/v1",
                None,
                None
            )
            .unwrap()
            .configured
        );
    }

    #[test]
    fn unsupported_clients_and_providers_are_rejected() {
        assert!(require_codex("codex").is_ok());
        for app in ["claude", "gemini", "grokbuild", "opencode"] {
            assert!(require_codex(app).is_err());
        }
        let mut provider = Provider::with_id("test".into(), "test".into(), serde_json::json!({}));
        assert!(require_copilot(&provider).is_err());
        provider.meta = Some(
            serde_json::from_value(serde_json::json!({
                "providerType": "github_copilot"
            }))
            .unwrap(),
        );
        assert!(require_copilot(&provider).is_ok());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn provider_and_proxy_lifecycle_leave_client_files_unchanged() {
        let fixture = TestHome::new();
        let app_dir = fixture._dir.path().join(".copilot-bridge-atlas");
        let client_dir = fixture._dir.path().join(".codex");
        std::fs::create_dir_all(client_dir.join("skills")).unwrap();
        let files = [
            ("config.toml", "# keep comments\nmodel_auto_compact_token_limit = 900000\n[mcp_servers.keep]\ncommand = 'keep'\n"),
            ("auth.json", r#"{"OPENAI_API_KEY":"keep-local-auth"}"#),
            ("AGENTS.md", "Keep my instructions"),
            ("skills/SKILL.md", "Keep my skill"),
        ];
        for (name, contents) in files {
            std::fs::write(client_dir.join(name), contents).unwrap();
        }
        let db = std::sync::Arc::new(Database::memory().unwrap());
        let state = AppState::new(db.clone());
        let mut provider = Provider::with_id(
            "copilot-a".into(),
            "Copilot".into(),
            serde_json::json!({
                "config": "model_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"https://api.githubcopilot.com\"\n",
                "modelCatalog": {"models": [{"model": "gpt-6-astra", "contextWindow": 1048576}]}
            }),
        );
        provider.meta = Some(
            serde_json::from_value(serde_json::json!({
                "providerType": "github_copilot"
            }))
            .unwrap(),
        );
        db.save_provider("codex", &provider).unwrap();
        initialize(&state).unwrap();
        assert!(catalog_path().starts_with(&app_dir));
        assert!(catalog_path().exists());
        provider.name = "Renamed Copilot".into();
        save(&db, &provider).unwrap();
        let mut other = provider.clone();
        other.id = "copilot-b".into();
        db.save_provider("codex", &other).unwrap();
        // Old takeover data must never cause a restore or rewrite in this build.
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TABLE proxy_live_backup (app_type TEXT PRIMARY KEY, original_config TEXT);
             INSERT INTO proxy_live_backup VALUES ('codex', 'old configuration');",
            )
            .unwrap();
        let mut config = db.get_proxy_config().await.unwrap();
        config.listen_port = 0;
        db.update_proxy_config(config).await.unwrap();
        let info = state.proxy_service.start().await.unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let origin = format!("http://127.0.0.1:{}", info.port);
        assert_eq!(
            client
                .get(format!("{origin}/health"))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        // The client TOML deliberately has no catalog pointer. Discovery must
        // still serve the Atlas-owned catalog, not consult or change that file.
        let models: serde_json::Value = client
            .get(format!("{origin}/v1/models"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(models["models"][0]["slug"], "gpt-6-astra");
        assert_eq!(
            client
                .post(format!("{origin}/claude/v1/messages"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            client
                .post(format!("{origin}/grokbuild/v1/responses"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            client
                .get(format!("{origin}/v1/responses"))
                .send()
                .await
                .unwrap()
                .status(),
            405
        );
        let mut updated = state.proxy_service.get_config().await.unwrap();
        updated.listen_port = 0;
        state.proxy_service.update_config(&updated).await.unwrap();
        state.proxy_service.stop().await.unwrap();
        assert!(save(&db, &other).is_err());
        let original_backup: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT original_config FROM proxy_live_backup WHERE app_type = 'codex'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(original_backup, "old configuration");
        for (name, contents) in files {
            assert_eq!(
                std::fs::read_to_string(client_dir.join(name)).unwrap(),
                contents,
                "{name}"
            );
        }
        assert!(!client_dir
            .join("copilot-bridge-atlas-model-catalog.json")
            .exists());
    }
}
