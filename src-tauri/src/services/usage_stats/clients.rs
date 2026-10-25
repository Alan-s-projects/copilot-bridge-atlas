//! Caller attribution retained alongside the unchanged all-client rollup totals.
use super::{effective_usage_log_filter, effective_usage_rollup_filter};
use crate::{
    database::{lock_conn, Database},
    error::AppError,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(super) fn client_name_sql(alias: &str) -> String {
    format!("COALESCE(NULLIF(trim({alias}.client_name), ''), 'Unknown')")
}

pub(super) fn push_client_filter(
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::ToSql>>,
    alias: &str,
    client_name: Option<&str>,
) {
    if let Some(name) = client_name {
        conditions.push(format!("{} = ?", client_name_sql(alias)));
        let name = name.trim();
        let name = (!name.is_empty()).then_some(name).unwrap_or("Unknown");
        params.push(Box::new(name.to_owned()));
    }
}

/// Only filtered queries expand attribution. Legacy name→cost maps do not
/// establish per-client token/request counts, so they must not be extrapolated.
/// Existing unfiltered callers keep using the original aggregate totals.
pub(super) fn usage_rollup_table_sql(client_name: Option<&str>) -> String {
    if client_name.is_none() {
        return "usage_daily_rollups".into();
    }
    "(SELECT base.date, base.app_type, base.provider_id, base.model,
        base.request_model, base.pricing_model, c.key AS client_name,
        json_extract(c.value, '$.request_count') AS request_count,
        json_extract(c.value, '$.success_count') AS success_count,
        json_extract(c.value, '$.input_tokens') AS input_tokens,
        json_extract(c.value, '$.output_tokens') AS output_tokens,
        json_extract(c.value, '$.cache_read_tokens') AS cache_read_tokens,
        json_extract(c.value, '$.cache_creation_tokens') AS cache_creation_tokens,
        2 AS input_token_semantics,
        json_extract(c.value, '$.total_cost_usd') AS total_cost_usd,
        json_extract(c.value, '$.total_latency_ms') / MAX(json_extract(c.value, '$.request_count'), 1) AS avg_latency_ms,
        json_extract(c.value, '$.timed_output_tokens') AS timed_output_tokens,
        json_extract(c.value, '$.generation_ms') AS generation_ms,
        json_object(c.key, json_extract(c.value, '$.total_cost_usd')) AS client_costs
        FROM usage_daily_rollups base, json_each(base.client_usage) c)".into()
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct ClientUsageRollup {
    pub request_count: u64,
    pub success_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_cost_usd: f64,
    pub total_latency_ms: f64,
    pub timed_output_tokens: u64,
    pub generation_ms: f64,
}

pub(crate) type ClientUsage = BTreeMap<String, ClientUsageRollup>;

impl ClientUsageRollup {
    pub(crate) fn merge(&mut self, other: Self) {
        self.request_count += other.request_count;
        self.success_count += other.success_count;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
        self.total_cost_usd += other.total_cost_usd;
        self.total_latency_ms += other.total_latency_ms;
        self.timed_output_tokens += other.timed_output_tokens;
        self.generation_ms += other.generation_ms;
    }
}

impl Database {
    /// Options come from retained proxy history, never from model names or
    /// identification sources. Legacy cost-only attribution still supplies
    /// names, even when it cannot supply trustworthy filtered usage metrics.
    pub fn get_usage_clients(&self) -> Result<Vec<String>, AppError> {
        let conn = lock_conn!(self.conn);
        let sql = format!("SELECT DISTINCT client_name FROM (
            SELECT {name} AS client_name FROM proxy_request_logs l WHERE {logs}
            UNION ALL
            SELECT COALESCE(NULLIF(trim(c.key), ''), 'Unknown')
            FROM usage_daily_rollups r, json_each(r.client_usage) c WHERE {rollups}
            UNION ALL
            SELECT COALESCE(NULLIF(trim(c.key), ''), 'Unknown')
            FROM usage_daily_rollups r, json_each(r.client_costs) c WHERE {rollups}
        ) ORDER BY client_name COLLATE NOCASE, client_name",
            name = client_name_sql("l"), logs = effective_usage_log_filter("l"),
            rollups = effective_usage_rollup_filter("r"));
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::usage_stats::LogFilters;

    #[test]
    fn cost_only_legacy_rollup_warns_without_inventing_an_unknown_caller() -> Result<(), AppError> {
        let db = Database::memory()?;
        db.conn.lock().unwrap().execute(
            "INSERT INTO usage_daily_rollups (date,app_type,provider_id,model,input_tokens,total_cost_usd)
             VALUES ('2020-01-02','codex','p','m',10,'3')",
            [],
        )?;
        assert!(db.get_usage_clients()?.is_empty());
        let filtered =
            db.get_usage_summary_filtered(None, None, None, None, None, Some("Unknown"))?;
        assert!(filtered.client_filter_incomplete);
        assert_eq!(filtered.total_cost, "0.000000");
        let all = db.get_usage_summary(None, None, None, None, None)?;
        assert!(!all.client_filter_incomplete);
        assert_eq!(all.total_cost, "3.000000");
        assert_eq!(all.total_input_tokens, 10);
        Ok(())
    }

    #[test]
    fn null_and_blank_names_are_unknown_without_altering_detection_metadata() -> Result<(), AppError>
    {
        let db = Database::memory()?;
        {
            let conn = db.conn.lock().unwrap();
            // Exercise older/imported databases where the name column was nullable.
            conn.execute_batch(
                "ALTER TABLE proxy_request_logs DROP COLUMN client_name;
                ALTER TABLE proxy_request_logs ADD COLUMN client_name TEXT;",
            )?;
            for (index, name) in [
                None,
                Some(""),
                Some("   "),
                Some("Unknown"),
                Some("Claude Code"),
            ]
            .into_iter()
            .enumerate()
            {
                conn.execute("INSERT INTO proxy_request_logs
                    (request_id,provider_id,app_type,model,client_name,client_source,client_version,latency_ms,status_code,created_at,input_tokens,total_cost_usd)
                    VALUES (?1,'p','codex','not-a-client',?2,'originator','2.0',100,200,1578000000,10,'1')",
                    rusqlite::params![index.to_string(), name])?;
            }
            conn.execute("INSERT INTO proxy_request_logs
                (request_id,provider_id,app_type,model,client_name,latency_ms,status_code,created_at)
                VALUES ('foreign','p','other-app','m','Unrelated app',1,200,1578000000)", [])?;
            conn.execute("INSERT INTO usage_daily_rollups (date,app_type,provider_id,model,client_costs)
                VALUES ('2020-01-02','other-app','p','m','{\"Foreign archive\":1}')", [])?;
        }
        assert_eq!(db.get_usage_clients()?, vec!["Claude Code", "Unknown"]);
        for selected in ["Unknown", "", "   "] {
            let summary =
                db.get_usage_summary_filtered(None, None, None, None, None, Some(selected))?;
            assert_eq!(summary.total_requests, 4);
            assert_eq!(summary.total_input_tokens, 40);
            assert!(!summary.client_filter_incomplete);
        }
        let logs = db.get_request_logs(
            &LogFilters {
                client_name: Some("Unknown".into()),
                ..Default::default()
            },
            0,
            10,
        )?;
        assert_eq!(logs.total, 4);
        assert!(logs.data.iter().all(|row| row.client.client_name == "Unknown"));
        assert!(logs
            .data
            .iter()
            .all(|row| row.client.client_source == "originator"));
        assert!(logs
            .data
            .iter()
            .all(|row| row.client.client_version.as_deref() == Some("2.0")));
        assert_eq!(db.rollup_and_prune(30)?, 5);
        assert_eq!(db.get_usage_clients()?, vec!["Claude Code", "Unknown"]);
        let archived =
            db.get_usage_summary_filtered(None, None, None, None, None, Some("Unknown"))?;
        assert_eq!(archived.total_requests, 4);
        assert_eq!(archived.total_input_tokens, 40);
        assert!(!archived.client_filter_incomplete);
        Ok(())
    }
}
