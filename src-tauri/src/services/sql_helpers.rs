//! SQL fragment helpers shared across usage aggregation queries.
//!
//! OpenAI input tokens include cached tokens. Normalize Codex rows once while
//! retaining opaque imported rows without reinterpreting their token counts.

/// `app_type` 的存储 `input_tokens` 是否已包含 cache read/write。
pub(crate) fn is_cache_inclusive_app(app_type: &str) -> bool {
    app_type == "codex"
}

pub(crate) const INPUT_TOKEN_SEMANTICS_LEGACY: i64 = 0;
pub(crate) const INPUT_TOKEN_SEMANTICS_TOTAL: i64 = 1;
pub(crate) const INPUT_TOKEN_SEMANTICS_FRESH: i64 = 2;

/// Match the per-request TPS timing: explicit duration, then latency minus TTFT,
/// then end-to-end latency. Invalid generation timing remains unknown.
pub(crate) fn output_generation_ms_sql(alias: &str) -> String {
    format!(
        "CASE WHEN {alias}.duration_ms > 0 THEN {alias}.duration_ms \
         WHEN {alias}.first_token_ms IS NOT NULL THEN \
           CASE WHEN {alias}.latency_ms > {alias}.first_token_ms \
                THEN {alias}.latency_ms - {alias}.first_token_ms END \
         WHEN {alias}.latency_ms > 0 THEN {alias}.latency_ms END"
    )
}

/// Build an SQL expression that returns the cache-normalized `input_tokens`
/// for a single row in `proxy_request_logs` or `usage_daily_rollups`.
///
/// Legacy rows subtract cache reads only. New total-inclusive rows subtract
/// both cache reads and writes. Rollups normalized to fresh input are returned
/// unchanged.
///
/// Pass an empty string to reference the columns directly (no alias),
/// or a table alias such as `"l"` to emit `l.input_tokens` style references.
pub fn fresh_input_sql(alias: &str) -> String {
    let prefix = if alias.is_empty() {
        String::new()
    } else {
        format!("{alias}.")
    };
    format!(
        "CASE \
              WHEN {prefix}input_token_semantics = {INPUT_TOKEN_SEMANTICS_FRESH} THEN {prefix}input_tokens \
              WHEN {prefix}app_type = 'codex' \
                   AND {prefix}input_token_semantics = {INPUT_TOKEN_SEMANTICS_TOTAL} \
                   AND {prefix}input_tokens >= ({prefix}cache_read_tokens + {prefix}cache_creation_tokens) \
              THEN ({prefix}input_tokens - {prefix}cache_read_tokens - {prefix}cache_creation_tokens) \
              WHEN {prefix}app_type = 'codex' \
                   AND {prefix}input_token_semantics = {INPUT_TOKEN_SEMANTICS_LEGACY} \
                   AND {prefix}input_tokens >= {prefix}cache_read_tokens \
              THEN ({prefix}input_tokens - {prefix}cache_read_tokens) \
              ELSE {prefix}input_tokens END"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE proxy_request_logs (
                request_id TEXT PRIMARY KEY,
                app_type TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                input_token_semantics INTEGER NOT NULL DEFAULT 0
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn fresh_input_with_alias_emits_prefixed_columns() {
        let sql = fresh_input_sql("l");
        assert!(sql.contains("l.app_type"));
        assert!(sql.contains("l.input_tokens"));
        assert!(sql.contains("l.cache_read_tokens"));
    }

    #[test]
    fn fresh_input_without_alias_uses_bare_columns() {
        let sql = fresh_input_sql("");
        assert!(!sql.contains("."));
        assert!(sql.contains("'codex'"));
    }

    #[test]
    fn fresh_input_subtracts_cache_for_cache_inclusive_providers() {
        let conn = setup_conn();
        // Codex row: OpenAI semantics — input_tokens includes the 600 cached.
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id, app_type, input_tokens, cache_read_tokens)
             VALUES ('codex-1', 'codex', 1000, 600)",
            [],
        )
        .unwrap();
        // Unknown imported data is not reinterpreted.
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id, app_type, input_tokens, cache_read_tokens)
             VALUES ('legacy-1', 'legacy-import', 200, 5000)",
            [],
        )
        .unwrap();

        let expr = fresh_input_sql("l");
        let sql = format!("SELECT COALESCE(SUM({expr}), 0) FROM proxy_request_logs l");
        let total: i64 = conn.query_row(&sql, [], |r| r.get(0)).unwrap();
        assert_eq!(total, 400 + 200);
    }

    #[test]
    fn fresh_input_handles_codex_with_cache_exceeding_input() {
        // Defensive: if a malformed Codex row somehow has cache > input,
        // we keep the original value rather than producing a negative number.
        let conn = setup_conn();
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id, app_type, input_tokens, cache_read_tokens)
             VALUES ('codex-broken', 'codex', 100, 999)",
            [],
        )
        .unwrap();
        let expr = fresh_input_sql("l");
        let sql = format!("SELECT {expr} FROM proxy_request_logs l");
        let value: i64 = conn.query_row(&sql, [], |r| r.get(0)).unwrap();
        assert_eq!(value, 100);
    }

    #[test]
    fn fresh_input_subtracts_cache_write_for_total_semantics() {
        let conn = setup_conn();
        conn.execute(
            "INSERT INTO proxy_request_logs (
                request_id, app_type, input_tokens, cache_read_tokens,
                cache_creation_tokens, input_token_semantics
             ) VALUES ('codex-total', 'codex', 1000, 300, 200, ?1)",
            [INPUT_TOKEN_SEMANTICS_TOTAL],
        )
        .unwrap();
        let expr = fresh_input_sql("l");
        let sql = format!("SELECT {expr} FROM proxy_request_logs l");
        let value: i64 = conn.query_row(&sql, [], |row| row.get(0)).unwrap();
        assert_eq!(value, 500);
    }

    #[test]
    fn fresh_input_keeps_normalized_rollup_value() {
        let conn = setup_conn();
        conn.execute(
            "INSERT INTO proxy_request_logs (
                request_id, app_type, input_tokens, cache_read_tokens,
                cache_creation_tokens, input_token_semantics
             ) VALUES ('codex-fresh', 'codex', 500, 300, 200, ?1)",
            [INPUT_TOKEN_SEMANTICS_FRESH],
        )
        .unwrap();
        let expr = fresh_input_sql("l");
        let sql = format!("SELECT {expr} FROM proxy_request_logs l");
        let value: i64 = conn.query_row(&sql, [], |row| row.get(0)).unwrap();
        assert_eq!(value, 500);
    }
}
