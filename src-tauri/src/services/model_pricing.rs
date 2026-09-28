use crate::config::{atomic_write, get_app_config_dir};
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::proxy::providers::copilot_model_map::is_valid_model_id;
use rusqlite::{params, Transaction};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

const MODEL_PRICING_FILE_NAME: &str = "model-pricing.json";
const MODEL_PRICING_FILE_VERSION: u32 = 1;
pub(crate) const DEFAULT_PRICING_TIER: &str = "default";
pub(crate) const LONG_CONTEXT_PRICING_TIER: &str = "long_context";

static MODEL_PRICING_FILE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn file_lock() -> &'static Mutex<()> {
    MODEL_PRICING_FILE_LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricingInfo {
    pub model_id: String,
    pub display_name: String,
    pub input_cost_per_million: String,
    pub output_cost_per_million: String,
    pub cache_read_cost_per_million: String,
    pub cache_creation_cost_per_million: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_context: Option<LongContextPricing>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LongContextPricing {
    pub threshold_input_tokens: u64,
    pub input_cost_per_million: String,
    pub output_cost_per_million: String,
    pub cache_read_cost_per_million: String,
    pub cache_creation_cost_per_million: String,
}

impl LongContextPricing {
    fn normalize(self) -> Result<Self, AppError> {
        if self.threshold_input_tokens == 0 {
            return Err(AppError::Config(
                "Long-context threshold must be positive".into(),
            ));
        }
        Ok(Self {
            threshold_input_tokens: self.threshold_input_tokens,
            input_cost_per_million: normalize_decimal(
                "long_context_input",
                &self.input_cost_per_million,
            )?,
            output_cost_per_million: normalize_decimal(
                "long_context_output",
                &self.output_cost_per_million,
            )?,
            cache_read_cost_per_million: normalize_decimal(
                "long_context_read",
                &self.cache_read_cost_per_million,
            )?,
            cache_creation_cost_per_million: normalize_decimal(
                "long_context_write",
                &self.cache_creation_cost_per_million,
            )?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelPricingFile {
    version: u32,
    #[serde(default)]
    models: Vec<ModelPricingInfo>,
    #[serde(default)]
    deleted_model_ids: Vec<String>,
}

impl Default for ModelPricingFile {
    fn default() -> Self {
        Self {
            version: MODEL_PRICING_FILE_VERSION,
            models: Vec::new(),
            deleted_model_ids: Vec::new(),
        }
    }
}

pub fn model_pricing_file_path() -> PathBuf {
    get_app_config_dir().join(MODEL_PRICING_FILE_NAME)
}

fn normalize_decimal(label: &str, value: &str) -> Result<String, AppError> {
    let value = value.trim();
    let parsed = Decimal::from_str(value).map_err(|error| {
        AppError::Message(format!("{label} price is invalid: {value} - {error}"))
    })?;
    if parsed < Decimal::ZERO {
        return Err(AppError::Message(format!(
            "{label} price must be non-negative: {value}"
        )));
    }
    Ok(value.to_string())
}

fn normalize_pricing(entry: ModelPricingInfo) -> Result<ModelPricingInfo, AppError> {
    let model_id = entry.model_id.trim().to_ascii_lowercase();
    let display_name = entry.display_name.trim().to_string();
    if !is_valid_model_id(&model_id) {
        return Err(AppError::Message(
            "A model ID without whitespace is required".into(),
        ));
    }
    if display_name.is_empty() {
        return Err(AppError::Message("Display name is required".into()));
    }

    Ok(ModelPricingInfo {
        long_context: entry
            .long_context
            .map(LongContextPricing::normalize)
            .transpose()?,
        model_id,
        display_name,
        input_cost_per_million: normalize_decimal("input_cost", &entry.input_cost_per_million)?,
        output_cost_per_million: normalize_decimal("output_cost", &entry.output_cost_per_million)?,
        cache_read_cost_per_million: normalize_decimal(
            "cache_read_cost",
            &entry.cache_read_cost_per_million,
        )?,
        cache_creation_cost_per_million: normalize_decimal(
            "cache_creation_cost",
            &entry.cache_creation_cost_per_million,
        )?,
    })
}

fn normalize_key_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalize_file(mut file: ModelPricingFile) -> Result<ModelPricingFile, AppError> {
    if file.version != MODEL_PRICING_FILE_VERSION {
        return Err(AppError::Config(format!(
            "model-pricing.json version {} is incompatible with version {}",
            file.version, MODEL_PRICING_FILE_VERSION
        )));
    }

    let deleted = normalize_key_list(file.deleted_model_ids)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut models = BTreeMap::new();
    for entry in file.models {
        let entry = normalize_pricing(entry)?;
        if !deleted.contains(&entry.model_id) {
            models.insert(entry.model_id.clone(), entry);
        }
    }

    file.version = MODEL_PRICING_FILE_VERSION;
    file.models = models.into_values().collect();
    file.deleted_model_ids = deleted.into_iter().collect();
    Ok(file)
}

fn read_file_unlocked() -> Result<Option<ModelPricingFile>, AppError> {
    let path = model_pricing_file_path();
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|error| AppError::io(&path, error))?;
    let file = serde_json::from_str(&content).map_err(|error| AppError::json(&path, error))?;
    normalize_file(file).map(Some)
}

fn write_file_unlocked(file: &ModelPricingFile) -> Result<(), AppError> {
    let path = model_pricing_file_path();
    let mut data = serde_json::to_vec_pretty(file).map_err(|error| {
        AppError::Config(format!(
            "Serialized model pricing configuration failed: {error}"
        ))
    })?;
    data.push(b'\n');
    atomic_write(&path, &data)
}

fn load_or_create_file_unlocked() -> Result<ModelPricingFile, AppError> {
    if let Some(file) = read_file_unlocked()? {
        return Ok(file);
    }

    // Store explicit user overrides; leave seeded prices in SQLite.
    let file = ModelPricingFile::default();
    write_file_unlocked(&file)?;
    Ok(file)
}

fn upsert_pricing(
    transaction: &Transaction<'_>,
    entry: &ModelPricingInfo,
) -> Result<usize, AppError> {
    transaction
        .execute(
            "INSERT INTO model_pricing (
                model_id, display_name, input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million, long_context
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(model_id) DO UPDATE SET
                display_name = excluded.display_name,
                input_cost_per_million = excluded.input_cost_per_million,
                output_cost_per_million = excluded.output_cost_per_million,
                cache_read_cost_per_million = excluded.cache_read_cost_per_million,
                cache_creation_cost_per_million = excluded.cache_creation_cost_per_million,
                long_context = excluded.long_context
            WHERE display_name <> excluded.display_name
               OR input_cost_per_million <> excluded.input_cost_per_million
               OR output_cost_per_million <> excluded.output_cost_per_million
               OR cache_read_cost_per_million <> excluded.cache_read_cost_per_million
               OR cache_creation_cost_per_million <> excluded.cache_creation_cost_per_million
               OR long_context IS NOT excluded.long_context",
            params![
                entry.model_id,
                entry.display_name,
                entry.input_cost_per_million,
                entry.output_cost_per_million,
                entry.cache_read_cost_per_million,
                entry.cache_creation_cost_per_million,
                entry
                    .long_context
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|e| AppError::Config(e.to_string()))?
            ],
        )
        .map_err(|error| AppError::Database(format!("Failed to update model pricing: {error}")))
}

fn apply_file_to_database(
    db: &Database,
    file: &ModelPricingFile,
) -> Result<(usize, usize), AppError> {
    let mut conn = lock_conn!(db.conn);
    let transaction = conn.transaction()?;
    let mut upserted = 0;
    for entry in &file.models {
        upserted += upsert_pricing(&transaction, entry)?;
    }
    let mut deleted = 0;
    for model_id in &file.deleted_model_ids {
        deleted += transaction.execute(
            "DELETE FROM model_pricing WHERE model_id = ?1",
            params![model_id],
        )?;
    }
    transaction.commit()?;
    Ok((upserted, deleted))
}

/// Load all model overrides from Atlas's own model-pricing.json.
pub fn sync_local_model_pricing(db: &Database) -> Result<usize, AppError> {
    let (upserted, deleted) = {
        let _file_guard = file_lock().lock().map_err(|error| {
            AppError::Config(format!("Model pricing file lock failed: {error}"))
        })?;
        let file = load_or_create_file_unlocked()?;
        apply_file_to_database(db, &file)?
    };

    // Deleting pricing cannot make a zero-cost usage row calculable. In
    // particular, seeded rows covered by tombstones may be reinserted and
    // deleted on every startup; they must not trigger a full-table backfill.
    if upserted > 0 {
        if let Err(error) = db.backfill_missing_usage_costs() {
            log::warn!("Failed to backfill historical usage cost after local model pricing synchronization: {error}");
        }
    }
    Ok(upserted + deleted)
}

pub fn update_model_pricing(db: &Database, entry: ModelPricingInfo) -> Result<usize, AppError> {
    let mut entry = normalize_pricing(entry)?;
    entry.model_id.make_ascii_lowercase();

    sync_local_model_pricing(db)?;
    let changed = {
        let _file_guard = file_lock().lock().map_err(|error| {
            AppError::Config(format!("Model pricing file lock failed: {error}"))
        })?;
        let mut file = load_or_create_file_unlocked()?;
        let mut file_models = file
            .models
            .into_iter()
            .map(|entry| (entry.model_id.clone(), entry))
            .collect::<BTreeMap<_, _>>();
        file_models.insert(entry.model_id.clone(), entry.clone());
        file.models = file_models.into_values().collect();
        file.deleted_model_ids
            .retain(|model_id| model_id != &entry.model_id);

        let mut conn = lock_conn!(db.conn);
        let transaction = conn.transaction()?;
        let changed = upsert_pricing(&transaction, &entry)?;
        write_file_unlocked(&file)?;
        transaction.commit()?;
        changed
    };

    if changed > 0 {
        if let Err(error) = db.backfill_missing_usage_costs_for_model(&entry.model_id) {
            log::warn!(
                "Could not backfill usage costs after updating pricing for {}: {error}",
                entry.model_id
            );
        }
    }
    Ok(changed)
}

pub fn delete_model_pricing(db: &Database, model_id: &str) -> Result<(), AppError> {
    let model_id = model_id.trim().to_ascii_lowercase();
    if !is_valid_model_id(&model_id) {
        return Err(AppError::Message(
            "A model ID without whitespace is required".into(),
        ));
    }

    sync_local_model_pricing(db)?;
    let _file_guard = file_lock()
        .lock()
        .map_err(|error| AppError::Config(format!("Model pricing file lock failed: {error}")))?;
    let mut file = load_or_create_file_unlocked()?;
    file.models.retain(|entry| entry.model_id != model_id);
    if !file
        .deleted_model_ids
        .iter()
        .any(|entry| entry == &model_id)
    {
        file.deleted_model_ids.push(model_id.to_string());
        file.deleted_model_ids.sort();
    }

    let mut conn = lock_conn!(db.conn);
    let transaction = conn.transaction()?;
    transaction.execute(
        "DELETE FROM model_pricing WHERE model_id = ?1",
        params![model_id],
    )?;
    write_file_unlocked(&file)?;
    transaction.commit()?;
    Ok(())
}

/// Restore bundled prices and remove explicit overrides and deletion tombstones.
/// Recorded request costs remain untouched.
pub fn reset_model_pricing_to_defaults(db: &Database) -> Result<(), AppError> {
    let _file_guard = file_lock()
        .lock()
        .map_err(|error| AppError::Config(format!("Model pricing file lock failed: {error}")))?;
    let mut file = read_file_unlocked()?.unwrap_or_default();
    file.models.clear();
    file.deleted_model_ids.clear();

    let mut conn = lock_conn!(db.conn);
    let transaction = conn.transaction()?;
    transaction.execute("DELETE FROM model_pricing", [])?;
    Database::ensure_model_pricing_seeded_on_conn(&transaction)?;
    write_file_unlocked(&file)?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn with_test_home(test: impl FnOnce(&Database, &PathBuf)) {
        struct TestEnvironment(Option<std::ffi::OsString>);
        impl Drop for TestEnvironment {
            fn drop(&mut self) {
                match &self.0 {
                    Some(previous) => std::env::set_var("COPILOT_BRIDGE_ATLAS_TEST_HOME", previous),
                    None => std::env::remove_var("COPILOT_BRIDGE_ATLAS_TEST_HOME"),
                }
            }
        }
        let temp = tempfile::tempdir().expect("tempdir");
        let _environment = TestEnvironment(std::env::var_os("COPILOT_BRIDGE_ATLAS_TEST_HOME"));
        std::env::set_var("COPILOT_BRIDGE_ATLAS_TEST_HOME", temp.path());

        let db = Database::memory().expect("memory database");
        let path = model_pricing_file_path();
        assert!(path.starts_with(temp.path()));
        test(&db, &path);
    }

    fn sample_pricing() -> ModelPricingInfo {
        ModelPricingInfo {
            long_context: None,
            model_id: "gpt-custom-model".to_string(),
            display_name: "GPT Custom Model".to_string(),
            input_cost_per_million: "1.25".to_string(),
            output_cost_per_million: "5".to_string(),
            cache_read_cost_per_million: "0.1".to_string(),
            cache_creation_cost_per_million: "1.5".to_string(),
        }
    }

    #[test]
    #[serial]
    fn long_context_override_round_trips_and_rejects_invalid_tiers() {
        with_test_home(|db, path| {
            let mut entry = sample_pricing();
            entry.long_context = Some(LongContextPricing {
                threshold_input_tokens: 200000,
                input_cost_per_million: "4".into(),
                output_cost_per_million: "12".into(),
                cache_read_cost_per_million: "1".into(),
                cache_creation_cost_per_million: "0".into(),
            });
            update_model_pricing(db, entry.clone()).unwrap();
            let file: ModelPricingFile = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            assert_eq!(file.models, vec![entry.clone()]);
            let before = fs::read(path).unwrap();
            entry.long_context.as_mut().unwrap().threshold_input_tokens = 0;
            assert!(update_model_pricing(db, entry).is_err());
            assert_eq!(fs::read(path).unwrap(), before);
            let conn = db.conn.lock().unwrap();
            assert_eq!(
                crate::services::usage_stats::find_model_pricing_for_input(
                    &conn,
                    "gpt-custom-model",
                    200001
                )
                .unwrap()
                .unwrap(),
                ("4".into(), "12".into(), "1".into(), "0".into()),
            );
        });
    }

    #[test]
    #[serial]
    fn creates_local_file_without_sync_settings() {
        with_test_home(|db, path| {
            sync_local_model_pricing(db).expect("load local pricing");
            assert!(path.exists());

            let content = fs::read_to_string(path).expect("read pricing file");
            let file: ModelPricingFile = serde_json::from_str(&content).expect("parse file");
            assert!(file.models.is_empty());
        });
    }

    #[test]
    #[serial]
    fn empty_override_file_preserves_existing_database_prices() {
        with_test_home(|db, path| {
            sync_local_model_pricing(db).expect("create override file");
            {
                let conn = db.conn.lock().expect("lock test database");
                assert_eq!(
                    conn.execute(
                        "UPDATE model_pricing
                         SET input_cost_per_million = '99'
                         WHERE model_id = 'gpt-6-astra'",
                        [],
                    )
                    .expect("simulate built-in pricing repair"),
                    1
                );
            }

            assert_eq!(sync_local_model_pricing(db).expect("reload overrides"), 0);

            let conn = db.conn.lock().expect("lock test database");
            let input: String = conn
                .query_row(
                    "SELECT input_cost_per_million
                     FROM model_pricing WHERE model_id = 'gpt-6-astra'",
                    [],
                    |row| row.get(0),
                )
                .expect("query repaired pricing");
            drop(conn);
            assert_eq!(input, "99");

            let content = fs::read_to_string(path).expect("read override file");
            let file: ModelPricingFile = serde_json::from_str(&content).expect("parse file");
            assert!(file.models.is_empty());
        });
    }

    #[test]
    #[serial]
    fn manual_edit_updates_existing_price_without_duplicate_rows() {
        with_test_home(|db, path| {
            let mut manual = sample_pricing();
            manual.input_cost_per_million = "9".to_string();
            manual.output_cost_per_million = "18".to_string();
            update_model_pricing(db, manual).expect("save manual pricing");

            let edited = sample_pricing();
            update_model_pricing(db, edited.clone()).expect("edit pricing");

            let conn = db.conn.lock().expect("lock test database");
            let input: String = conn
                .query_row(
                    "SELECT input_cost_per_million FROM model_pricing WHERE model_id = ?1",
                    params!["gpt-custom-model"],
                    |row| row.get(0),
                )
                .expect("query edited pricing");
            drop(conn);
            assert_eq!(input, edited.input_cost_per_million);

            let content = fs::read_to_string(path).expect("read pricing file");
            let file: ModelPricingFile = serde_json::from_str(&content).expect("parse file");
            let saved = file
                .models
                .iter()
                .find(|entry| entry.model_id == "gpt-custom-model")
                .expect("saved edited pricing");
            assert_eq!(saved, &edited);
            assert_eq!(file.models.len(), 1);
        });
    }

    #[test]
    #[serial]
    fn manual_update_and_delete_are_persisted_to_local_file() {
        with_test_home(|db, path| {
            assert_eq!(
                update_model_pricing(db, sample_pricing()).expect("manual update"),
                1
            );
            let content = fs::read_to_string(path).expect("read pricing file");
            let file: ModelPricingFile = serde_json::from_str(&content).expect("parse file");
            assert!(file
                .models
                .iter()
                .any(|entry| entry.model_id == "gpt-custom-model"));

            delete_model_pricing(db, "gpt-custom-model").expect("delete pricing");
            let content = fs::read_to_string(path).expect("read updated file");
            let file: ModelPricingFile =
                serde_json::from_str(&content).expect("parse updated file");
            assert!(!file
                .models
                .iter()
                .any(|entry| entry.model_id == "gpt-custom-model"));
            assert!(file
                .deleted_model_ids
                .iter()
                .any(|entry| entry == "gpt-custom-model"));
        });
    }

    #[test]
    #[serial]
    fn reset_restores_all_bundled_prices_and_preserves_metadata_and_history() {
        with_test_home(|db, path| {
            let edited_default = ModelPricingInfo {
                long_context: None,
                model_id: "gpt-6-astra".into(),
                display_name: "GPT-6 Astra custom".into(),
                input_cost_per_million: "99".into(),
                output_cost_per_million: "199".into(),
                cache_read_cost_per_million: "9".into(),
                cache_creation_cost_per_million: "19".into(),
            };
            update_model_pricing(db, edited_default).expect("edit default price");
            update_model_pricing(db, sample_pricing()).expect("add custom GPT price");
            delete_model_pricing(db, "gpt-6-luna").expect("add GPT tombstone");

            let retired_entry = ModelPricingInfo {
                long_context: None,
                model_id: "retired-model".into(),
                display_name: "Retired model".into(),
                input_cost_per_million: "4.2".into(),
                output_cost_per_million: "8.4".into(),
                cache_read_cost_per_million: "0".into(),
                cache_creation_cost_per_million: "0".into(),
            };
            {
                let mut file = read_file_unlocked()
                    .expect("read pricing file")
                    .expect("pricing file exists");
                file.models.push(retired_entry.clone());
                file.deleted_model_ids.push("retired-tombstone".into());
                write_file_unlocked(&file).expect("save custom pricing");

                let conn = db.conn.lock().expect("lock test database");
                conn.execute(
                    "INSERT INTO model_pricing (
                        model_id, display_name, input_cost_per_million, output_cost_per_million
                    ) VALUES ('retired-model', 'Retired model', '4.2', '8.4')",
                    [],
                )
                .expect("insert retired database price");
                conn.execute(
                    "INSERT INTO proxy_request_logs (
                        request_id, provider_id, app_type, model, request_model,
                        input_tokens, output_tokens, input_cost_usd, output_cost_usd,
                        total_cost_usd, latency_ms, status_code, created_at
                    ) VALUES (
                        'recorded-price', 'copilot', 'codex', 'gpt-6-astra',
                        'gpt-6-astra', 1000, 20, '0.099', '0.004', '0.103', 100, 200, 1
                    )",
                    [],
                )
                .expect("insert recorded usage cost");
            }

            reset_model_pricing_to_defaults(db).expect("reset bundled prices");

            let conn = db.conn.lock().expect("lock test database");
            let default_input: String = conn
                .query_row(
                    "SELECT input_cost_per_million FROM model_pricing
                     WHERE model_id = 'gpt-6-astra'",
                    [],
                    |row| row.get(0),
                )
                .expect("query restored default");
            assert_eq!(default_input, "10");
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM model_pricing
                     WHERE model_id = 'gpt-6-luna'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("query restored tombstoned default"),
                1
            );
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM model_pricing
                     WHERE model_id = 'gpt-custom-model'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("query removed custom GPT price"),
                0
            );
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM model_pricing
                     WHERE model_id = 'retired-model'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("query removed custom price"),
                0
            );
            assert_eq!(
                conn.query_row(
                    "SELECT total_cost_usd FROM proxy_request_logs
                     WHERE request_id = 'recorded-price'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("query unchanged history"),
                "0.103"
            );
            drop(conn);

            let saved: serde_json::Value =
                serde_json::from_slice(&fs::read(path).expect("read saved pricing file"))
                    .expect("parse saved pricing file");
            assert_eq!(saved["models"], serde_json::json!([]));
            assert_eq!(saved["deletedModelIds"], serde_json::json!([]));
        });
    }

    #[test]
    #[serial]
    fn reloads_manual_file_edits_and_deletion_tombstones() {
        with_test_home(|db, path| {
            sync_local_model_pricing(db).expect("create pricing file");
            let content = fs::read_to_string(path).expect("read pricing file");
            let mut file: ModelPricingFile =
                serde_json::from_str(&content).expect("parse pricing file");
            file.models.push(sample_pricing());
            fs::write(
                path,
                serde_json::to_vec_pretty(&file).expect("serialize file"),
            )
            .expect("write manual edit");

            assert_eq!(sync_local_model_pricing(db).expect("reload file"), 1);
            {
                let conn = db.conn.lock().expect("lock test database");
                let input: String = conn
                    .query_row(
                        "SELECT input_cost_per_million FROM model_pricing WHERE model_id = ?1",
                        params!["gpt-custom-model"],
                        |row| row.get(0),
                    )
                    .expect("query manually added pricing");
                assert_eq!(input, "1.25");
            }

            let content = fs::read_to_string(path).expect("read updated pricing file");
            let mut file: ModelPricingFile =
                serde_json::from_str(&content).expect("parse updated pricing file");
            file.deleted_model_ids.push("gpt-custom-model".to_string());
            fs::write(
                path,
                serde_json::to_vec_pretty(&file).expect("serialize tombstone"),
            )
            .expect("write tombstone");

            assert_eq!(sync_local_model_pricing(db).expect("apply tombstone"), 1);
            let conn = db.conn.lock().expect("lock test database");
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM model_pricing WHERE model_id = ?1",
                    params!["gpt-custom-model"],
                    |row| row.get(0),
                )
                .expect("query deleted pricing");
            assert_eq!(count, 0);
        });
    }

    #[test]
    #[serial]
    fn repeated_seeded_tombstone_deletion_does_not_backfill_unrelated_usage() {
        with_test_home(|db, _path| {
            sync_local_model_pricing(db).expect("create override file");
            {
                let conn = db.conn.lock().expect("lock test database");
                conn.execute(
                    "INSERT INTO proxy_request_logs (
                        request_id, provider_id, app_type, model, request_model,
                        input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                        input_cost_usd, output_cost_usd, cache_read_cost_usd,
                        cache_creation_cost_usd, total_cost_usd, latency_ms,
                        status_code, created_at, data_source
                    ) VALUES (
                        'pending-cost', 'test-provider', 'codex', 'gpt-5', 'gpt-5',
                        1000000, 0, 0, 0, '0', '0', '0', '0', '0', 100, 200, 1, 'proxy'
                    )",
                    [],
                )
                .expect("insert zero-cost usage");
            }

            delete_model_pricing(db, "gpt-6-astra").expect("create tombstone");
            db.ensure_model_pricing_seeded()
                .expect("reseed built-in pricing");
            assert_eq!(sync_local_model_pricing(db).expect("apply tombstone"), 1);

            let conn = db.conn.lock().expect("lock test database");
            let deleted_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM model_pricing
                     WHERE model_id = 'gpt-6-astra'",
                    [],
                    |row| row.get(0),
                )
                .expect("query tombstoned pricing");
            let total_cost: f64 = conn
                .query_row(
                    "SELECT CAST(total_cost_usd AS REAL)
                     FROM proxy_request_logs WHERE request_id = 'pending-cost'",
                    [],
                    |row| row.get(0),
                )
                .expect("query pending usage cost");
            assert_eq!(deleted_count, 0);
            assert_eq!(total_cost, 0.0);
        });
    }

    #[test]
    #[serial]
    fn rejects_invalid_pricing_before_writing_a_file_or_database() {
        with_test_home(|db, path| {
            let mut retired = sample_pricing();
            retired.model_id = "invalid model".into();
            assert!(update_model_pricing(db, retired).is_err());
            assert!(delete_model_pricing(db, "invalid model").is_err());
            assert!(!path.exists());
            assert_eq!(db.conn.lock().unwrap().query_row(
                "SELECT COUNT(*) FROM model_pricing WHERE model_id IN ('gpt-custom-model', 'retired-model')",
                [], |row| row.get::<_, i64>(0)
            ).unwrap(), 0);
        });
    }

    #[test]
    #[serial]
    fn explicit_non_gpt_overrides_and_tombstones_are_applied_without_rewriting_the_file() {
        with_test_home(|db, path| {
            let mut legacy = sample_pricing();
            legacy.model_id = "retired-model".into();
            legacy.input_cost_per_million = "99".into();
            let file = ModelPricingFile {
                models: vec![legacy],
                deleted_model_ids: vec!["retired-tombstone".into()],
                ..Default::default()
            };
            db.conn.lock().unwrap().execute_batch(
                "INSERT INTO model_pricing (model_id, display_name, input_cost_per_million, output_cost_per_million)
                 VALUES ('retired-model', 'Historical price', '4.2', '5'),
                        ('retired-tombstone', 'Historical price', '4.2', '5');"
            ).unwrap();
            write_file_unlocked(&file).unwrap();
            let before = fs::read(path).unwrap();
            assert_eq!(sync_local_model_pricing(db).unwrap(), 2);
            assert_eq!(fs::read(path).unwrap(), before);
            assert_eq!(db.conn.lock().unwrap().query_row(
                "SELECT COUNT(*) FROM model_pricing WHERE model_id LIKE 'retired-%' AND input_cost_per_million = '99'",
                [], |row| row.get::<_, i64>(0)
            ).unwrap(), 1);
        });
    }
}
