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
            "proxy_request_diagnostics",
            "proxy_request_logs",
            "settings",
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
fn failed_request_snapshots_follow_request_detail_retention() -> Result<(), AppError> {
    let db = Database::memory()?;
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO proxy_request_logs
                (request_id, provider_id, app_type, model, latency_ms, status_code, created_at)
             VALUES ('atlas_failure', 'copilot', 'codex', 'gpt-6-sol', 100, 502, 1)",
            [],
        )?;
    }
    let detail = RequestDiagnosticDetail {
        upstream_status: Some(200),
        failure_stage: Some("stream".into()),
        request_headers: Some("authorization: [redacted]".into()),
        request_body: Some("{\"model\":\"gpt-6-sol\"}".into()),
        response_headers: Some("content-type: text/event-stream".into()),
        response_body: Some("event: response.failed".into()),
    };
    db.save_request_diagnostics("atlas_failure", &detail)?;
    let saved = db.get_request_diagnostics("atlas_failure")?.unwrap();
    assert_eq!(saved.upstream_status, Some(200));
    assert_eq!(
        saved.response_body.as_deref(),
        Some("event: response.failed")
    );

    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM proxy_request_logs WHERE request_id = 'atlas_failure'",
            [],
        )?;
        let remaining: i64 = conn.query_row(
            "SELECT COUNT(*) FROM proxy_request_diagnostics",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(remaining, 0);
    }
    assert!(db.get_request_diagnostics("atlas_failure")?.is_none());
    Ok(())
}

#[test]
fn invalid_column_settings_can_be_replaced_without_changing_other_preferences() {
    let db = Database::memory().unwrap();
    for invalid in ["not json", r#"{"requestLogs":false}"#, "null"] {
        db.set_setting("usage_table_columns", invalid).unwrap();
        db.set_setting("unrelated", "keep").unwrap();
        assert_eq!(db.get_usage_table_columns().unwrap(), Default::default());
        let saved = db
            .set_usage_table_columns("requestLogs", vec!["time".into()])
            .unwrap();
        assert_eq!(saved.request_logs, Some(vec!["time".into()]));
        assert_eq!(db.get_usage_table_columns().unwrap(), saved);
        assert_eq!(
            db.get_setting("unrelated").unwrap().as_deref(),
            Some("keep")
        );
        assert!(db.set_usage_table_columns("unknown", vec![]).is_err());
        assert_eq!(db.get_usage_table_columns().unwrap(), saved);
    }
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
fn bundled_prices_are_unique_github_copilot_rates_for_only_documented_models() {
    use rust_decimal::Decimal;
    use std::str::FromStr;
    let prices = Database::bundled_model_prices().unwrap();
    assert_eq!(prices.len(), 33);
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
        ("grok-4.5", "2", "6", "0.50"),
    ] {
        let row = prices.iter().find(|row| row[0] == id).unwrap();
        assert_eq!((&*row[2], &*row[3], &*row[4]), (input, output, cache));
    }
    assert!(ids.contains(&"mai-code-1.1-flash".to_string()));
    for retired in [
        "gpt-5",
        "gpt-5.6-cyber",
        "gpt-5.5-pro",
        "deepseek-v3",
        "claude-mythos-5",
    ] {
        assert!(!ids.contains(&retired.to_string()));
    }
    assert!(!ids.contains(&"gpt-5.6-sol-fast".to_string()));
}

#[test]
fn copilot_pricing_migration_retires_only_unedited_defaults() {
    let conn = Connection::open_in_memory().unwrap();
    Database::create_tables_on_conn(&conn).unwrap();
    conn.execute(
        "INSERT INTO proxy_request_logs
         (request_id,provider_id,app_type,model,input_tokens,total_cost_usd,latency_ms,status_code,created_at)
         VALUES ('old-flat','copilot','codex','gpt-6-astra',9999999,'7.123456',1,200,1)",
        [],
    ).unwrap();
    let legacy: serde_json::Value =
        serde_json::from_str(include_str!("../resources/legacy-model-pricing.json")).unwrap();
    for row in legacy["prices"].as_array().unwrap() {
        let values: Vec<&str> = row
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        conn.execute(
            "INSERT INTO model_pricing(model_id,display_name,input_cost_per_million,output_cost_per_million,cache_read_cost_per_million,cache_creation_cost_per_million)
             VALUES (?1,?2,?3,?4,?5,?6)", rusqlite::params_from_iter(values),
        ).unwrap();
    }
    conn.execute(
        "UPDATE model_pricing SET input_cost_per_million='99' WHERE model_id='gpt-6-astra'",
        [],
    )
    .unwrap();
    Database::set_user_version(&conn, 21).unwrap();
    Database::apply_schema_migrations_on_conn(&conn).unwrap();
    Database::ensure_model_pricing_seeded_on_conn(&conn).unwrap();
    assert_eq!(conn.query_row(
        "SELECT pricing_tier,total_cost_usd FROM proxy_request_logs WHERE request_id='old-flat'",
        [], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)),
    ).unwrap(), ("default".into(),"7.123456".into()));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM model_pricing", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        33
    );
    assert_eq!(
        conn.query_row(
            "SELECT input_cost_per_million FROM model_pricing WHERE model_id='gpt-6-astra'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "99"
    );
    assert_eq!(
        conn.query_row(
            "SELECT cache_read_cost_per_million FROM model_pricing WHERE model_id='grok-4.5'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "0.50"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM model_pricing WHERE long_context IS NOT NULL",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        10
    );
    Database::apply_schema_migrations_on_conn(&conn).unwrap();
    assert_eq!(Database::get_user_version(&conn).unwrap(), SCHEMA_VERSION);
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
