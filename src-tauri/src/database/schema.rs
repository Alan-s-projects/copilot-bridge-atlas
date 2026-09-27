//! Minimal bridge schema. Imported version-19 data and retired tables remain intact.
use super::{lock_conn, Database, SCHEMA_VERSION};
use crate::error::AppError;
use rusqlite::Connection;

impl Database {
    pub(crate) fn create_tables_on_conn(conn: &Connection) -> Result<(), AppError> {
        conn.execute_batch(r#"
CREATE TABLE IF NOT EXISTS providers (
    id TEXT NOT NULL, app_type TEXT NOT NULL, name TEXT NOT NULL,
    settings_config TEXT NOT NULL, meta TEXT NOT NULL DEFAULT '{}',
    is_current BOOLEAN NOT NULL DEFAULT 0, PRIMARY KEY (id, app_type)
);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE IF NOT EXISTS proxy_config (
    app_type TEXT PRIMARY KEY,
    proxy_enabled INTEGER NOT NULL DEFAULT 0,
    listen_address TEXT NOT NULL DEFAULT '127.0.0.1',
    listen_port INTEGER NOT NULL DEFAULT 15722,
    enable_logging INTEGER NOT NULL DEFAULT 1,
    default_cost_multiplier TEXT NOT NULL DEFAULT '1',
    pricing_model_source TEXT NOT NULL DEFAULT 'response',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT OR IGNORE INTO proxy_config (app_type, listen_port) VALUES ('codex', 15722);
CREATE TABLE IF NOT EXISTS proxy_request_logs (
    request_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, app_type TEXT NOT NULL, model TEXT NOT NULL,
    request_model TEXT, pricing_model TEXT, pricing_tier TEXT,
    requested_reasoning_effort TEXT, applied_reasoning_effort TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    input_token_semantics INTEGER NOT NULL DEFAULT 0,
    input_cost_usd TEXT NOT NULL DEFAULT '0', output_cost_usd TEXT NOT NULL DEFAULT '0',
    cache_read_cost_usd TEXT NOT NULL DEFAULT '0', cache_creation_cost_usd TEXT NOT NULL DEFAULT '0',
    total_cost_usd TEXT NOT NULL DEFAULT '0', latency_ms INTEGER NOT NULL, first_token_ms INTEGER,
    duration_ms INTEGER, status_code INTEGER NOT NULL, error_message TEXT, session_id TEXT,
    provider_type TEXT, is_streaming INTEGER NOT NULL DEFAULT 0,
    cost_multiplier TEXT NOT NULL DEFAULT '1.0', created_at INTEGER NOT NULL,
    data_source TEXT NOT NULL DEFAULT 'proxy'
);
CREATE INDEX IF NOT EXISTS idx_request_logs_provider ON proxy_request_logs(provider_id, app_type);
CREATE INDEX IF NOT EXISTS idx_request_logs_created_at ON proxy_request_logs(created_at);
CREATE INDEX IF NOT EXISTS idx_request_logs_model ON proxy_request_logs(model);
CREATE INDEX IF NOT EXISTS idx_request_logs_session ON proxy_request_logs(session_id);
CREATE INDEX IF NOT EXISTS idx_request_logs_status ON proxy_request_logs(status_code);
CREATE INDEX IF NOT EXISTS idx_request_logs_app_created_at ON proxy_request_logs(app_type, created_at DESC);
CREATE TABLE IF NOT EXISTS model_pricing (
    model_id TEXT PRIMARY KEY, display_name TEXT NOT NULL,
    input_cost_per_million TEXT NOT NULL, output_cost_per_million TEXT NOT NULL,
    cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
    cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0',
    long_context TEXT
);
CREATE TABLE IF NOT EXISTS stream_check_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT, provider_id TEXT NOT NULL, provider_name TEXT NOT NULL,
    app_type TEXT NOT NULL, status TEXT NOT NULL, success INTEGER NOT NULL, message TEXT NOT NULL,
    response_time_ms INTEGER, http_status INTEGER, model_used TEXT,
    retry_count INTEGER DEFAULT 0, tested_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_stream_check_logs_provider ON stream_check_logs(app_type, provider_id, tested_at DESC);
CREATE TABLE IF NOT EXISTS usage_daily_rollups (
    date TEXT NOT NULL, app_type TEXT NOT NULL, provider_id TEXT NOT NULL, model TEXT NOT NULL,
    request_model TEXT NOT NULL DEFAULT '', pricing_model TEXT NOT NULL DEFAULT '',
    request_count INTEGER NOT NULL DEFAULT 0, success_count INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    input_token_semantics INTEGER NOT NULL DEFAULT 0,
    total_cost_usd TEXT NOT NULL DEFAULT '0', avg_latency_ms INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, app_type, provider_id, model, request_model, pricing_model)
);
"#).map_err(|error| AppError::Database(error.to_string()))
    }

    pub(crate) fn apply_schema_migrations(&self) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        Self::apply_schema_migrations_on_conn(&conn)
    }

    pub(crate) fn apply_schema_migrations_on_conn(conn: &Connection) -> Result<(), AppError> {
        let version = Self::get_user_version(conn)?;
        if version > SCHEMA_VERSION || (version != 0 && version < 19) {
            return Err(AppError::Database(format!(
                "Unsupported database schema {version}; this app accepts version 19 through {SCHEMA_VERSION}."
            )));
        }
        conn.execute_batch("SAVEPOINT atlas_schema;")
            .map_err(|error| AppError::Database(error.to_string()))?;
        let result = Self::create_tables_on_conn(conn)
            .and_then(|_| Self::add_reasoning_log_columns(conn))
            .and_then(|_| Self::migrate_copilot_pricing(conn, version))
            .and_then(|_| Self::set_user_version(conn, SCHEMA_VERSION));
        match result {
            Ok(()) => conn
                .execute_batch("RELEASE atlas_schema;")
                .map_err(|error| AppError::Database(error.to_string())),
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK TO atlas_schema; RELEASE atlas_schema;");
                Err(error)
            }
        }
    }

    fn add_reasoning_log_columns(conn: &Connection) -> Result<(), AppError> {
        let mut statement = conn.prepare("PRAGMA table_info(proxy_request_logs)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<std::collections::HashSet<_>, _>>()?;
        drop(statement);
        for column in ["requested_reasoning_effort", "applied_reasoning_effort"] {
            if !columns.contains(column) {
                conn.execute(
                    &format!("ALTER TABLE proxy_request_logs ADD COLUMN {column} TEXT"),
                    [],
                )?;
            }
        }
        Ok(())
    }

    pub fn ensure_model_pricing_seeded(&self) -> Result<(), AppError> {
        let conn = lock_conn!(self.conn);
        Self::ensure_model_pricing_seeded_on_conn(&conn)
    }

    pub(crate) fn ensure_model_pricing_seeded_on_conn(conn: &Connection) -> Result<(), AppError> {
        let tiers = Self::bundled_long_context_prices()?;
        // Fill missing estimates; never overwrite imported or custom prices.
        for [id, name, input, output, cache_read, cache_creation] in Self::bundled_model_prices()? {
            conn.execute(
                "INSERT OR IGNORE INTO model_pricing (model_id, display_name,
                 input_cost_per_million, output_cost_per_million,
                 cache_read_cost_per_million, cache_creation_cost_per_million, long_context)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    id,
                    name,
                    input,
                    output,
                    cache_read,
                    cache_creation,
                    tiers
                        .get(&id)
                        .map(serde_json::to_string)
                        .transpose()
                        .map_err(|e| AppError::Config(e.to_string()))?
                ],
            )
            .map_err(|error| AppError::Database(error.to_string()))?;
        }
        Ok(())
    }

    pub(crate) fn bundled_model_prices() -> Result<Vec<[String; 6]>, AppError> {
        #[derive(serde::Deserialize)]
        struct BundledPricing {
            prices: Vec<[String; 6]>,
        }
        let bundled: BundledPricing = serde_json::from_str(include_str!(
            "../resources/model-pricing.json"
        ))
        .map_err(|error| AppError::Config(format!("Invalid bundled model pricing: {error}")))?;
        Ok(bundled.prices)
    }

    pub(crate) fn bundled_long_context_prices() -> Result<
        std::collections::BTreeMap<String, crate::services::model_pricing::LongContextPricing>,
        AppError,
    > {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct BundledPricing {
            long_context: std::collections::BTreeMap<
                String,
                crate::services::model_pricing::LongContextPricing,
            >,
        }
        serde_json::from_str::<BundledPricing>(include_str!("../resources/model-pricing.json"))
            .map(|bundled| bundled.long_context)
            .map_err(|e| AppError::Config(e.to_string()))
    }

    fn migrate_copilot_pricing(conn: &Connection, version: i32) -> Result<(), AppError> {
        let has_recorded_tier: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('proxy_request_logs') WHERE name='pricing_tier')",
            [], |row| row.get(0),
        )?;
        if !has_recorded_tier {
            conn.execute(
                "ALTER TABLE proxy_request_logs ADD COLUMN pricing_tier TEXT",
                [],
            )?;
        }
        let has_tier: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('model_pricing') WHERE name='long_context')",
            [], |row| row.get(0),
        )?;
        if !has_tier {
            conn.execute("ALTER TABLE model_pricing ADD COLUMN long_context TEXT", [])?;
        }
        if version < 22 {
            // Atlas used flat rates before schema 22. Do not relabel old costs
            // with a tier inferred from today's pricing.
            conn.execute(
                "UPDATE proxy_request_logs SET pricing_tier='default'
                 WHERE pricing_tier IS NULL AND COALESCE(data_source,'proxy')='proxy'
                   AND CAST(total_cost_usd AS REAL)>0",
                [],
            )?;
            #[derive(serde::Deserialize)]
            struct LegacyPricing {
                prices: Vec<[String; 6]>,
            }
            let legacy: LegacyPricing =
                serde_json::from_str(include_str!("../resources/legacy-model-pricing.json"))
                    .map_err(|e| AppError::Config(e.to_string()))?;
            // Replace only unchanged old defaults. Explicit local overrides are
            // reapplied on startup; edited/imported prices and recorded costs survive.
            for [id, name, input, output, read, write] in legacy.prices {
                conn.execute(
                    "DELETE FROM model_pricing WHERE model_id=?1 AND display_name=?2
                     AND input_cost_per_million=?3 AND output_cost_per_million=?4
                     AND cache_read_cost_per_million=?5 AND cache_creation_cost_per_million=?6",
                    rusqlite::params![id, name, input, output, read, write],
                )?;
            }
        }
        Ok(())
    }
    pub(crate) fn get_user_version(conn: &Connection) -> Result<i32, AppError> {
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| AppError::Database(error.to_string()))
    }

    pub(crate) fn set_user_version(conn: &Connection, version: i32) -> Result<(), AppError> {
        conn.pragma_update(None, "user_version", version)
            .map_err(|error| AppError::Database(error.to_string()))
    }

    pub(crate) fn table_exists(conn: &Connection, table: &str) -> Result<bool, AppError> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get(0),
        )
        .map_err(|error| AppError::Database(error.to_string()))
    }
}
