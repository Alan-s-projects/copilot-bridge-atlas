//! Read-only Codex discovery and Atlas-owned model catalog generation.
use crate::config::get_home_dir;
use crate::error::AppError;
use crate::proxy::providers::copilot_model_map::is_valid_model_id;
use once_cell::sync::OnceCell;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(not(test))]
static CODEX_MODEL_CATALOG_TEMPLATE_CACHE: OnceCell<CodexCatalogTemplate> = OnceCell::new();

#[derive(Clone, Debug, PartialEq)]
struct CodexCatalogTemplate {
    compatibility: Value,
    instructions: serde_json::Map<String, Value>,
}

const OPENAI_INSTRUCTION_PROFILES: [&str; 3] = ["gpt-6-astra", "gpt-6-luna", "gpt-6-sol"];
const OPENAI_FALLBACK_PROFILE: &str = "gpt-6-astra";

/// Get Codex configuration directory path
pub fn get_codex_config_dir() -> PathBuf {
    get_home_dir().join(".codex")
}

/// Get Codex config.toml path
pub fn get_codex_config_path() -> PathBuf {
    get_codex_config_dir().join("config.toml")
}

fn parse_codex_positive_u64(value: Option<&Value>) -> Option<u64> {
    match value {
        Some(Value::Number(n)) => n.as_u64().filter(|v| *v > 0),
        Some(Value::String(s)) => s.trim().parse::<u64>().ok().filter(|v| *v > 0),
        _ => None,
    }
}

fn extract_codex_top_level_u64(config_text: &str, field: &str) -> Option<u64> {
    let doc = config_text.parse::<toml::Value>().ok()?;
    doc.get(field)
        .and_then(|value| value.as_integer())
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn codex_catalog_input_modalities(declared_modalities: Option<&[String]>) -> Vec<String> {
    let mut modalities = vec!["text".to_string()];
    if declared_modalities.is_some_and(|items| {
        items
            .iter()
            .any(|item| item.trim().eq_ignore_ascii_case("image"))
    }) {
        modalities.push("image".to_string());
    }
    modalities
}

/// Canonical reasoning effort levels Codex understands. `none` disables thinking.
const CODEX_REASONING_LEVEL_DESCRIPTIONS: &[(&str, &str)] = &[
    ("none", "Disable Thinking"),
    ("minimal", "Minimal reasoning"),
    ("low", "Fast responses with lighter reasoning"),
    (
        "medium",
        "Balances speed and reasoning depth for everyday tasks",
    ),
    ("high", "Greater reasoning depth for complex problems"),
    ("xhigh", "Extra high reasoning depth for complex problems"),
    ("max", "Maximum reasoning depth for the hardest problems"),
    ("ultra", "Ultra reasoning depth"),
];

fn codex_reasoning_level_description(effort: &str) -> Option<&'static str> {
    CODEX_REASONING_LEVEL_DESCRIPTIONS
        .iter()
        .find(|(candidate, _)| *candidate == effort)
        .map(|(_, description)| *description)
}

/// User-declared levels reduced to the canonical efforts Codex understands,
/// in canonical (lowest → highest) order regardless of declaration order.
/// Unknown efforts are dropped so a typo can never produce an entry Codex
/// would reject.
fn codex_canonical_efforts(levels: &[String]) -> Vec<&str> {
    CODEX_REASONING_LEVEL_DESCRIPTIONS
        .iter()
        .filter(|(effort, _)| levels.iter().any(|candidate| candidate == effort))
        .map(|(effort, _)| *effort)
        .collect()
}

/// Build a `supported_reasoning_levels` array from user-declared effort values.
fn codex_supported_reasoning_levels(levels: &[String]) -> Value {
    let entries: Vec<Value> = codex_canonical_efforts(levels)
        .into_iter()
        .map(|effort| {
            let description = codex_reasoning_level_description(effort)
                .expect("canonical effort always has a description");
            json!({ "effort": effort, "description": description })
        })
        .collect();
    json!(entries)
}

/// Apply a per-model reasoning-level override onto a catalog entry. Returns
/// true when the override was applied (so callers can skip further work).
/// `template_default` is the base entry's `default_reasoning_level` (from the
/// model template) used as the fallback when the
/// user did not declare one explicitly.
fn apply_codex_reasoning_level_override(
    entry_obj: &mut serde_json::Map<String, Value>,
    template_default: Option<&str>,
    spec: &CodexCatalogModelSpec,
) -> bool {
    let levels = spec.reasoning_levels.as_deref().unwrap_or(&[]);
    let canonical = codex_canonical_efforts(levels);
    if canonical.is_empty() {
        entry_obj.insert(
            "supported_reasoning_levels".into(),
            codex_supported_reasoning_levels(&["none".into()]),
        );
        entry_obj.insert("default_reasoning_level".into(), json!("none"));
        entry_obj.insert("supports_reasoning_summaries".into(), json!(false));
        return true;
    }
    let supported = codex_supported_reasoning_levels(levels);
    entry_obj.insert("supported_reasoning_levels".to_string(), supported);

    // Default: explicit user value wins; otherwise keep the base default when
    // it is still supported; otherwise fall back to the highest supported
    // level in canonical order. All candidates are validated against the
    // canonical set so the default can never reference a dropped effort.
    let default_level = spec
        .default_reasoning_level
        .as_deref()
        .filter(|level| canonical.contains(level))
        .or_else(|| template_default.filter(|level| canonical.contains(level)))
        .or_else(|| canonical.last().copied());
    if let Some(default_level) = default_level {
        entry_obj.insert("default_reasoning_level".to_string(), json!(default_level));
    }
    true
}

fn codex_catalog_model_entry(
    template: &CodexCatalogTemplate,
    spec: &CodexCatalogModelSpec,
    priority: usize,
    default_context_window: u64,
) -> Value {
    let mut entry = template.compatibility.clone();
    let entry_obj = entry
        .as_object_mut()
        .expect("validated compatibility object");
    let profile = template
        .instructions
        .get(&spec.model.to_ascii_lowercase())
        .unwrap_or(&template.instructions[OPENAI_FALLBACK_PROFILE]);
    let messages = &profile["modelMessages"];
    // Mirror the upstream literal template for older Codex clients, without rendering or trimming.
    entry_obj.insert(
        "base_instructions".into(),
        messages["instructions_template"].clone(),
    );
    entry_obj.insert("model_messages".into(), messages.clone());

    let display_name = spec.display_name.as_deref().unwrap_or(&spec.model);
    let context_window = spec.context_window.unwrap_or(default_context_window);
    let max_context_window = spec
        .max_context_window
        .unwrap_or(context_window)
        .max(context_window);
    entry_obj.insert("slug".to_string(), json!(spec.model));
    entry_obj.insert("display_name".to_string(), json!(display_name));
    entry_obj.insert("description".to_string(), json!(display_name));
    entry_obj.insert("context_window".to_string(), json!(context_window));
    entry_obj.insert("max_context_window".to_string(), json!(max_context_window));
    entry_obj.insert("priority".to_string(), json!(1000 + priority));
    entry_obj.insert("additional_speed_tiers".to_string(), json!([]));
    entry_obj.insert("service_tiers".to_string(), json!([]));
    entry_obj.insert("availability_nux".to_string(), Value::Null);
    entry_obj.insert("upgrade".to_string(), Value::Null);

    // Publish only declared image support; the compatibility template is model-neutral.
    entry_obj.insert(
        "input_modalities".to_string(),
        json!(codex_catalog_input_modalities(
            spec.input_modalities.as_deref()
        )),
    );

    entry_obj.insert(
        "supports_parallel_tool_calls".to_string(),
        json!(spec.supports_parallel_tool_calls.unwrap_or(false)),
    );

    // Reasoning efforts come from the model, not the compatibility template.
    let template_default = template
        .compatibility
        .get("default_reasoning_level")
        .and_then(|value| value.as_str());
    apply_codex_reasoning_level_override(entry_obj, template_default, spec);

    entry
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexCatalogModelSpec {
    model: String,
    /// Display name; defaults to the model ID when absent.
    display_name: Option<String>,
    /// Saved context limit, capped by live Copilot metadata during refresh.
    context_window: Option<u64>,
    /// Total context limit from Copilot capability limits.
    max_context_window: Option<u64>,
    /// Live Copilot parallel-tool declaration.
    supports_parallel_tool_calls: Option<bool>,
    /// Live Copilot image-input declaration.
    input_modalities: Option<Vec<String>>,
    /// Per-row override for the generated catalog's `supported_reasoning_levels`
    /// (e.g. ["none", "low", "medium", "high", "xhigh", "max"]). Missing
    /// declarations expose only `none`, never another model's efforts.
    reasoning_levels: Option<Vec<String>>,
    /// Per-row override for the generated catalog's `default_reasoning_level`.
    /// Only meaningful together with `reasoning_levels`; when absent the
    /// template default is kept if it is still in the list, otherwise the last
    /// (highest) declared level wins.
    default_reasoning_level: Option<String>,
}

fn codex_catalog_model_specs(settings: &Value) -> Vec<CodexCatalogModelSpec> {
    let Some(models) = settings
        .get("modelCatalog")
        .and_then(|catalog| catalog.get("models"))
        .and_then(|models| models.as_array())
    else {
        return Vec::new();
    };
    let ultra_enabled = settings
        .get("enableUltraReasoning")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut seen = std::collections::HashSet::new();
    let mut specs = Vec::new();

    for model_config in models {
        if model_config.get("enabled").and_then(Value::as_bool) == Some(false)
            || model_config.get("available").and_then(Value::as_bool) == Some(false)
        {
            continue;
        }

        let Some(model) = model_config
            .get("model")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|model| is_valid_model_id(model))
        else {
            continue;
        };

        if !seen.insert(model.to_ascii_lowercase()) {
            continue;
        }

        let display_name = model_config
            .get("displayName")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        let context_window = parse_codex_positive_u64(model_config.get("contextWindow"));
        let max_context_window = parse_codex_positive_u64(model_config.get("maxContextWindow"));

        let supports_parallel_tool_calls = model_config
            .get("supportsParallelToolCalls")
            .and_then(|value| value.as_bool());
        let input_modalities = model_config
            .get("inputModalities")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|items| !items.is_empty());

        let mut reasoning_levels = model_config
            .get("reasoningLevels")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|level| !level.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .filter(|levels| !levels.is_empty());
        if let Some(supported) = model_config
            .get("supportedReasoningLevels")
            .and_then(Value::as_array)
        {
            let supported = supported
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            reasoning_levels = Some(supported.into_iter().map(str::to_string).collect());
        }
        if ultra_enabled {
            if let Some(ref mut levels) = reasoning_levels {
                let has_reasoning = levels.iter().any(|l| l != "none");
                if has_reasoning && !levels.iter().any(|l| l == "ultra") {
                    levels.push("ultra".to_string());
                }
            }
        }
        let default_reasoning_level = model_config
            .get("defaultReasoningLevel")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|level| !level.is_empty())
            .map(str::to_owned);

        specs.push(CodexCatalogModelSpec {
            model: model.to_string(),
            display_name,
            context_window,
            max_context_window,
            supports_parallel_tool_calls,
            input_modalities,
            reasoning_levels,
            default_reasoning_level,
        });
    }

    specs
}

fn load_codex_model_catalog_template_uncached() -> Result<CodexCatalogTemplate, AppError> {
    let compatibility: Value =
        serde_json::from_str(include_str!("resources/codex-model-template.json"))
            .map_err(|error| AppError::Config(format!("Invalid Atlas Codex template: {error}")))?;
    if !compatibility.is_object()
        || compatibility.get("base_instructions").is_some()
        || compatibility.get("model_messages").is_some()
    {
        return Err(AppError::Config(
            "Compatibility profile must not contain instruction overrides".into(),
        ));
    }
    let snapshot: Value = serde_json::from_str(include_str!(
        "resources/openai-codex-instructions.json"
    ))
    .map_err(|error| AppError::Config(format!("Invalid OpenAI instruction snapshot: {error}")))?;
    let instructions = snapshot
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(|| AppError::Config("OpenAI instruction profiles are missing".into()))?
        .clone();
    for profile in OPENAI_INSTRUCTION_PROFILES {
        if instructions
            .get(profile)
            .and_then(|profile| profile.pointer("/modelMessages/instructions_template"))
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(AppError::Config(format!(
                "OpenAI instruction profile {profile} is missing its template"
            )));
        }
    }
    Ok(CodexCatalogTemplate {
        compatibility,
        instructions,
    })
}

fn get_or_load_codex_model_catalog_template<F>(
    cache: &OnceCell<CodexCatalogTemplate>,
    loader: F,
) -> Result<CodexCatalogTemplate, AppError>
where
    F: FnOnce() -> Result<CodexCatalogTemplate, AppError>,
{
    cache.get_or_try_init(loader).cloned()
}

#[cfg(not(test))]
fn load_codex_model_catalog_template() -> Result<CodexCatalogTemplate, AppError> {
    get_or_load_codex_model_catalog_template(
        &CODEX_MODEL_CATALOG_TEMPLATE_CACHE,
        load_codex_model_catalog_template_uncached,
    )
}

#[cfg(test)]
fn load_codex_model_catalog_template() -> Result<CodexCatalogTemplate, AppError> {
    load_codex_model_catalog_template_uncached()
}

pub(crate) fn codex_model_catalog_from_settings(
    settings: &Value,
    config_text: &str,
) -> Result<Option<Value>, AppError> {
    let specs = codex_catalog_model_specs(settings);
    if specs.is_empty() {
        return Ok(None);
    }
    let default_context_window =
        extract_codex_top_level_u64(config_text, "model_context_window").unwrap_or(128_000);
    let template = load_codex_model_catalog_template()?;
    let entries = specs
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            codex_catalog_model_entry(&template, spec, index, default_context_window)
        })
        .collect::<Vec<_>>();
    Ok(Some(json!({ "models": entries })))
}

const MAX_CODEX_CATALOG_BYTES: u64 = 32 * 1024 * 1024;

/// Safely read the file as a string and return an error if the byte limit is exceeded.
pub(crate) fn read_limited_string(path: &Path, max_bytes: u64) -> Result<String, AppError> {
    let metadata = fs::metadata(path).map_err(|error| AppError::io(path, error))?;
    if metadata.len() > max_bytes {
        return Err(AppError::Config(format!(
            "File {} exceeds size limit {} bytes",
            path.display(),
            max_bytes
        )));
    }
    fs::read_to_string(path).map_err(|error| AppError::io(path, error))
}

/// Read the copilot-bridge-atlas Codex model catalog file with a size cap.
pub(crate) fn read_codex_model_catalog_text(path: &Path) -> Result<String, AppError> {
    read_limited_string(path, MAX_CODEX_CATALOG_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_preserves_saved_reasoning_and_live_gpt_capabilities() {
        let settings = json!({"modelCatalog": {"models": [{
            "model": "gpt-6-luna",
            "contextWindow": 872000,
            "supportsParallelToolCalls": true,
            "inputModalities": ["text", "image"],
            "reasoningLevels": ["ultra", "low", "high", "low"],
            "defaultReasoningLevel": "ultra"
        }]}});
        let specs = codex_catalog_model_specs(&settings);
        let entry = codex_catalog_model_entry(
            &load_codex_model_catalog_template().unwrap(),
            &specs[0],
            0,
            128000,
        );
        assert_eq!(entry["context_window"], 872000);
        assert_eq!(entry["supports_parallel_tool_calls"], true);
        assert_eq!(entry["input_modalities"], json!(["text", "image"]));
        assert_eq!(entry["default_reasoning_level"], "ultra");
        let levels = entry["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["effort"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(levels, ["low", "high", "ultra"]);
        assert!(entry.get("base_instructions").is_some());
        assert!(entry.get("supports_reasoning_summaries").is_some());
    }

    #[test]
    fn enabled_available_models_from_any_vendor_are_published_without_mutating_saved_rows() {
        let settings = json!({"modelCatalog": {"models": [
            {"model": "gpt-6-astra"}, {"model": "future-vendor/model"},
            {"model": "GPT-6-LUNA"}, {"model": "gpt-6-disabled", "enabled": false},
            {"model": ""}, {"model": "not-available", "available": false},
            {"model": "gemini-future"}, {"model": "GEMINI-FUTURE"}
        ]}});
        let specs = codex_catalog_model_specs(&settings);
        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.model.as_str())
                .collect::<Vec<_>>(),
            [
                "gpt-6-astra",
                "future-vendor/model",
                "GPT-6-LUNA",
                "gemini-future"
            ]
        );
        assert_eq!(
            settings["modelCatalog"]["models"].as_array().unwrap().len(),
            8
        );
    }

    #[test]
    fn non_reasoning_and_future_model_catalogs_do_not_inherit_gpt_efforts() {
        let settings = json!({"modelCatalog": {"models": [
            {"model": "future/text-only", "inputModalities": ["text"], "supportsParallelToolCalls": false,
             "reasoningLevels": ["high", "ultra"], "supportedReasoningLevels": []},
            {"model": "gemini-future", "reasoningLevels": ["low", "high", "ultra"], "supportedReasoningLevels": ["low", "high"],
             "defaultReasoningLevel": "ultra", "contextWindow": 936000}
        ]}});
        let catalog = codex_model_catalog_from_settings(&settings, "")
            .unwrap()
            .unwrap();
        let first = &catalog["models"][0];
        assert_eq!(first["input_modalities"], json!(["text"]));
        assert_eq!(first["supports_parallel_tool_calls"], false);
        assert_eq!(first["default_reasoning_level"], "none");
        assert_eq!(
            first["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(first["supports_reasoning_summaries"], false);
        let second = &catalog["models"][1];
        assert_eq!(second["context_window"], 936000);
        assert_eq!(second["default_reasoning_level"], "high");
        assert_eq!(
            second["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            settings["modelCatalog"]["models"][0]["reasoningLevels"],
            json!(["high", "ultra"])
        );
    }

    #[test]
    fn catalog_keeps_input_and_total_context_limits_distinct() {
        let settings = json!({"modelCatalog":{"models":[{
            "model":"grok-4.7", "contextWindow":372000, "maxContextWindow":500000
        }]}});
        let catalog = codex_model_catalog_from_settings(&settings, "")
            .unwrap()
            .unwrap();
        assert_eq!(catalog["models"][0]["context_window"], 372000);
        assert_eq!(catalog["models"][0]["max_context_window"], 500000);
        assert_eq!(catalog["models"][0]["effective_context_window_percent"], 95);
    }

    #[test]
    fn ultra_toggle_extends_live_efforts() {
        let models = json!([
            {"model":"reasoning", "reasoningLevels":["low","high"], "supportedReasoningLevels":["low","high","max"]},
            {"model":"none-only", "supportedReasoningLevels":["none"]},
            {"model":"disabled", "enabled":false, "supportedReasoningLevels":["high"]}
        ]);
        for enabled in [false, true] {
            let settings = json!({
                "enableUltraReasoning":enabled,
                "modelCatalog":{"models":models}
            });
            let before = settings.clone();
            let catalog = codex_model_catalog_from_settings(&settings, "")
                .unwrap()
                .unwrap();
            let entries = catalog["models"].as_array().unwrap();
            assert_eq!(entries.len(), 2);
            let levels: Vec<_> = entries[0]["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|level| level["effort"].as_str().unwrap())
                .collect();
            assert_eq!(
                levels,
                if enabled {
                    vec!["low", "high", "max", "ultra"]
                } else {
                    vec!["low", "high", "max"]
                }
            );
            assert_eq!(
                entries[1]["supported_reasoning_levels"][0]["effort"],
                "none"
            );
            assert_eq!(
                entries[1]["supported_reasoning_levels"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(settings, before);
        }
    }

    #[test]
    fn ultra_reasoning_toggle_exposes_ultra_when_enabled() {
        let settings = json!({
            "enableUltraReasoning": true,
            "modelCatalog": {"models": [
                {"model": "gpt-6-astra", "supportedReasoningLevels": ["low", "high"]},
                {"model": "non-reasoning", "supportedReasoningLevels": []}
            ]}
        });
        let catalog = codex_model_catalog_from_settings(&settings, "")
            .unwrap()
            .unwrap();
        let astra = &catalog["models"][0];
        let efforts: Vec<&str> = astra["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["effort"].as_str())
            .collect();
        assert!(efforts.contains(&"ultra"));
        let non_reasoning = &catalog["models"][1];
        let non_efforts: Vec<&str> = non_reasoning["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["effort"].as_str())
            .collect();
        assert_eq!(non_efforts, vec!["none"]);
    }

    #[test]
    fn cached_model_template_loads_once_and_failed_loads_can_retry() {
        let cache = OnceCell::new();
        assert!(
            get_or_load_codex_model_catalog_template(&cache, || Err(AppError::Config(
                "temporary error".into()
            )))
            .is_err()
        );
        let template = load_codex_model_catalog_template_uncached().unwrap();
        assert_eq!(
            get_or_load_codex_model_catalog_template(&cache, || Ok(template.clone())).unwrap(),
            template
        );
        assert_eq!(
            get_or_load_codex_model_catalog_template(&cache, || panic!(
                "cached template must not reload"
            ))
            .unwrap(),
            template
        );
    }

    #[test]
    fn upstream_instruction_objects_are_unchanged_for_every_generated_model() {
        let template = load_codex_model_catalog_template_uncached().unwrap();
        for (model, profile) in [
            ("gpt-6-astra", "gpt-6-astra"),
            ("GPT-6-LUNA", "gpt-6-luna"),
            ("gpt-6-sol", "gpt-6-sol"),
            ("gemini-3.8-flash", "gpt-6-astra"),
            ("grok-4.7", "gpt-6-astra"),
            ("gpt-5.5", "gpt-6-astra"),
            ("future-provider/model", "gpt-6-astra"),
        ] {
            let settings = json!({"modelCatalog":{"models":[{
                "model":model, "contextWindow":123456, "baseInstructions":"ignored legacy override",
                "inputModalities":["text"], "supportsParallelToolCalls":false
            }]}});
            let entry = &codex_model_catalog_from_settings(&settings, "")
                .unwrap()
                .unwrap()["models"][0];
            let upstream = &template.instructions[profile]["modelMessages"];
            assert_eq!(&entry["model_messages"], upstream, "{model}");
            assert_eq!(
                entry["base_instructions"], upstream["instructions_template"],
                "{model}"
            );
            assert!(entry["base_instructions"].as_str().unwrap().ends_with('\n'));
            assert_eq!(entry["slug"], model);
            assert_eq!(entry["context_window"], 123456);
            assert_eq!(entry["input_modalities"], json!(["text"]));
        }
        assert!(template.compatibility.get("slug").is_none());
        for flag in [
            "supports_reasoning_summaries",
            "support_verbosity",
            "supports_image_detail_original",
            "supports_search_tool",
        ] {
            assert_eq!(template.compatibility[flag], false, "{flag}");
        }
    }

    #[test]
    fn pinned_openai_instruction_hashes_and_provenance_match() {
        use sha2::{Digest, Sha256};
        let snapshot: Value =
            serde_json::from_str(include_str!("resources/openai-codex-instructions.json")).unwrap();
        assert_eq!(
            snapshot["source"]["commit"],
            "d8ec479c34895214b44c062f060d97224c191a50"
        );
        assert_eq!(snapshot["fallbackModel"], OPENAI_FALLBACK_PROFILE);
        for (slug, expected) in [
            (
                "gpt-6-astra",
                "35bd51b5f577cb7b24cd5f4629e49e37cb724ab57754ce6f8f202001635bab8a",
            ),
            (
                "gpt-6-luna",
                "b707476816bfe5e571a1bd2179f130fff2b132da5ab8e61063acdb7fd24daf12",
            ),
            (
                "gpt-6-sol",
                "b1dd8718c037906c53a305c5cbccb4a4be35ccbb7837461ec349bfc495412f0d",
            ),
        ] {
            let profile = &snapshot["models"][slug];
            let text = profile["modelMessages"]["instructions_template"]
                .as_str()
                .unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(text.as_bytes())), expected);
            assert_eq!(profile["instructionsSha256"], expected);
            let canonical =
                crate::proxy::json_canonical::canonical_json_string(&profile["modelMessages"]);
            let digest = format!("{:x}", Sha256::digest(canonical.as_bytes()));
            assert_eq!(profile["modelMessagesSha256"], digest);
        }
    }

    #[test]
    fn undeclared_capabilities_are_not_borrowed_from_another_model() {
        let settings = json!({"modelCatalog": {"models": [{"model": "future-model"}]}});
        let catalog = codex_model_catalog_from_settings(&settings, "")
            .unwrap()
            .unwrap();
        let entry = &catalog["models"][0];
        assert_eq!(entry["input_modalities"], json!(["text"]));
        assert_eq!(entry["supports_parallel_tool_calls"], false);
        assert_eq!(entry["default_reasoning_level"], "none");
        assert_eq!(entry["apply_patch_tool_type"], "freeform");
        assert_eq!(entry["shell_type"], "shell_command");
    }

    #[test]
    #[ignore = "exports a catalog for opt-in installed-Codex contract checks"]
    fn export_catalog_compatibility_fixture() {
        let settings_path = std::env::var("ATLAS_CODEX_CATALOG_SETTINGS").unwrap();
        let output = std::env::var("ATLAS_CODEX_CATALOG_OUT").unwrap();
        let settings: Value =
            serde_json::from_str(&fs::read_to_string(settings_path).unwrap()).unwrap();
        let catalog = codex_model_catalog_from_settings(&settings, "")
            .unwrap()
            .unwrap();
        fs::write(output, serde_json::to_string_pretty(&catalog).unwrap()).unwrap();
    }

    #[test]
    fn bounded_catalog_reads_reject_large_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("catalog.json");
        fs::write(&file, "{}").unwrap();
        assert_eq!(read_limited_string(&file, 2).unwrap(), "{}");
        assert!(read_limited_string(&file, 1).is_err());
    }
}
