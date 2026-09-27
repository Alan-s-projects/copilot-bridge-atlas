use super::*;
use crate::Provider;
use serde_json::{json, Value};

#[test]
fn fresh_database_creates_only_bridge_tables() {
    let db = Database::memory().unwrap();
    let conn = db.conn.lock().unwrap();
    let mut statement = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    ).unwrap();
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        tables,
        [
            "model_pricing",
            "providers",
            "proxy_config",
            "proxy_request_logs",
            "settings",
            "stream_check_logs",
            "usage_daily_rollups"
        ]
    );
    assert_eq!(Database::get_user_version(&conn).unwrap(), SCHEMA_VERSION);
    assert_eq!(
        conn.query_row(
            "SELECT listen_port FROM proxy_config WHERE app_type = 'codex'",
            [],
            |row| row.get::<_, u16>(0)
        )
        .unwrap(),
        15722
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM proxy_config", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn version_19_upgrade_preserves_extra_columns_tables_and_historical_costs() {
    let db = Database::memory().unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute_batch(
            "ALTER TABLE providers ADD COLUMN notes TEXT;
             CREATE TABLE retired_preferences (id TEXT PRIMARY KEY, value BLOB);
             INSERT INTO retired_preferences VALUES ('keep', X'00FF12');
             INSERT INTO providers (id, app_type, name, settings_config, meta, is_current, notes)
             VALUES ('copilot', 'codex', 'Copilot', '{}',
                     '{\"providerType\":\"github_copilot\",\"retiredPreference\":{\"keep\":true}}', 1, 'keep notes');
             INSERT INTO proxy_request_logs
                 (request_id, provider_id, app_type, model, latency_ms, status_code, created_at, total_cost_usd, data_source)
             VALUES ('historical', 'copilot', 'codex', 'gpt-6-astra', 1, 200, 1, '12.345600', 'imported');
             PRAGMA user_version = 19;"
        ).unwrap();
    }
    db.apply_schema_migrations().unwrap();
    let mut provider = db.get_provider_by_id("copilot", "codex").unwrap().unwrap();
    provider.settings_config = json!({"modelCatalog": {"models": [{
        "model": "gpt-6-astra", "reasoningLevels": ["high", "ultra"], "defaultReasoningLevel": "ultra"
    }]}});
    db.save_provider("codex", &provider).unwrap();
    let conn = db.conn.lock().unwrap();
    assert_eq!(Database::get_user_version(&conn).unwrap(), SCHEMA_VERSION);
    let (meta, notes, current): (String, String, bool) = conn
        .query_row(
            "SELECT meta, notes, is_current FROM providers WHERE id = 'copilot'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&meta).unwrap()["retiredPreference"],
        json!({"keep": true})
    );
    assert_eq!(notes, "keep notes");
    assert!(current);
    assert_eq!(
        conn.query_row(
            "SELECT value FROM retired_preferences WHERE id = 'keep'",
            [],
            |row| row.get::<_, Vec<u8>>(0)
        )
        .unwrap(),
        [0, 255, 18]
    );
    assert_eq!(
        conn.query_row(
            "SELECT total_cost_usd FROM proxy_request_logs WHERE request_id = 'historical'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "12.345600"
    );
}

#[test]
fn reasoning_log_migration_preserves_old_rows_and_is_repeatable() {
    for version in [19, 20] {
        let conn = Connection::open_in_memory().unwrap();
        Database::create_tables_on_conn(&conn).unwrap();
        conn.execute_batch(
            "ALTER TABLE proxy_request_logs DROP COLUMN requested_reasoning_effort;
             ALTER TABLE proxy_request_logs DROP COLUMN applied_reasoning_effort;
             INSERT INTO proxy_request_logs
                 (request_id, provider_id, app_type, model, latency_ms, status_code, created_at, total_cost_usd)
             VALUES ('historical', 'copilot', 'codex', 'gpt-6-astra', 100, 200, 1, '12.345600');"
        ).unwrap();
        Database::set_user_version(&conn, version).unwrap();
        for _ in 0..2 {
            Database::apply_schema_migrations_on_conn(&conn).unwrap();
        }
        let row: (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT total_cost_usd, requested_reasoning_effort, applied_reasoning_effort
             FROM proxy_request_logs WHERE request_id = 'historical'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("12.345600".into(), None, None));
        assert_eq!(Database::get_user_version(&conn).unwrap(), SCHEMA_VERSION);
        conn.execute(
            "UPDATE proxy_request_logs SET requested_reasoning_effort = 'ultra', applied_reasoning_effort = 'max'",
            [],
        ).unwrap();
        Database::apply_schema_migrations_on_conn(&conn).unwrap();
        let applied: String = conn
            .query_row(
                "SELECT applied_reasoning_effort FROM proxy_request_logs",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(applied, "max");
    }
}

#[test]
fn unsupported_schema_does_not_modify_tables_or_version() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE sentinel (value TEXT); INSERT INTO sentinel VALUES ('keep');")
        .unwrap();
    Database::set_user_version(&conn, SCHEMA_VERSION + 1).unwrap();
    assert!(Database::apply_schema_migrations_on_conn(&conn).is_err());
    assert_eq!(
        Database::get_user_version(&conn).unwrap(),
        SCHEMA_VERSION + 1
    );
    assert!(!Database::table_exists(&conn, "providers").unwrap());
    assert_eq!(
        conn.query_row("SELECT value FROM sentinel", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}

#[test]
fn pricing_seeds_all_bundled_models_and_preserves_custom_prices() {
    let db = Database::memory().unwrap();
    {
        let conn = db.conn.lock().unwrap();
        assert!(
            conn.query_row(
                "SELECT COUNT(*) FROM model_pricing WHERE model_id NOT LIKE 'gpt-%'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap()
                > 0
        );
        conn.execute("UPDATE model_pricing SET input_cost_per_million = '123.456' WHERE model_id = 'gpt-6-astra'", []).unwrap();
    }
    db.ensure_model_pricing_seeded().unwrap();
    assert_eq!(
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT input_cost_per_million FROM model_pricing WHERE model_id = 'gpt-6-astra'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "123.456"
    );
}

#[test]
fn bundled_prices_are_unique_valid_cc_switch_estimates_for_multiple_vendors() {
    use rust_decimal::Decimal;
    use std::str::FromStr;
    let prices = Database::bundled_model_prices().unwrap();
    assert_eq!(prices.len(), 219);
    let mut ids = std::collections::HashSet::new();
    for [id, name, input, output, cache_read, cache_creation] in &prices {
        assert!(ids.insert(id));
        assert!(crate::proxy::providers::copilot_model_map::is_valid_model_id(id));
        assert!(!name.trim().is_empty());
        for value in [input, output, cache_read, cache_creation] {
            assert!(Decimal::from_str(value).unwrap() >= Decimal::ZERO);
        }
    }
    for (id, input, output, cache) in [
        ("gemini-3.8-flash", "0.75", "3.75", "0.075"),
        ("gemini-3.5-flash", "1.50", "9.00", "0.15"),
        ("grok-4.7", "2", "6", "0.50"),
        ("grok-4.5", "2", "6", "0.30"),
    ] {
        let row = prices.iter().find(|row| row[0] == id).unwrap();
        assert_eq!((&*row[2], &*row[3], &*row[4]), (input, output, cache));
    }
    assert!(!ids.contains(&"mai-code-1.1-flash".to_string()));
    assert!(!ids.contains(&"gpt-5.6-sol-fast".to_string()));
}

#[test]
fn selecting_a_missing_provider_keeps_the_current_entry() {
    let db = Database::memory().unwrap();
    let provider = Provider::with_id("copilot".into(), "Copilot".into(), json!({}));
    db.save_provider("codex", &provider).unwrap();
    db.set_current_provider("codex", &provider.id).unwrap();
    assert!(db.set_current_provider("codex", "missing").is_err());
    assert_eq!(
        db.get_current_provider("codex").unwrap().as_deref(),
        Some("copilot")
    );
}

#[test]
fn incremental_vacuum_rebuild_preserves_rows() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE sentinel (value TEXT); INSERT INTO sentinel VALUES ('keep');")
        .unwrap();
    assert!(Database::ensure_incremental_auto_vacuum_on_conn(&conn).unwrap());
    assert_eq!(Database::get_auto_vacuum_mode(&conn).unwrap(), 2);
    assert_eq!(
        conn.query_row("SELECT value FROM sentinel", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
}
